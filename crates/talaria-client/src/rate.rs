//! How often this client asks for frames, how it works that out, and how it
//! says so to the human.
//!
//! **The one rule this module must never break: it decides how *often* frames
//! arrive, and it never decides whether the human may drive.** `D-05-06` is
//! explicit about why, and the sentence is worth carrying rather than citing:
//! a degraded takeover of a login wall still clears the login wall, which is
//! the entire product. Withholding control on a slow link would fail the core
//! value in exactly the situation it exists for — the human is at the client
//! *because* something needs a human, and a slow picture is not a reason to
//! leave them watching it happen. So there is no state in here that stops
//! input, and no branch that could grow into one.
//!
//! ## The measurement, and why it needs no synchronised clock
//!
//! Every input this client sends carries a sequence number, and every frame
//! header echoes the last input sequence the server had already applied when
//! it painted ([`talaria_protocol::wire::FrameHeader::last_applied_input`]).
//! When a frame arrives echoing sequence *n*, the time since *n* was sent is
//! an **input-to-photon** sample: both readings come off this machine's own
//! clock, so nothing has to be agreed between the two machines.
//!
//! That is also why the sample is the honest number rather than a round-trip
//! ping. It contains the transmission out, the server's hit test, the paint,
//! the framebuffer readback, the tile comparison, the encode, the transmission
//! back and this client's own presentation — which is the whole of what a
//! human waits for. A ping would measure the one part of that which is
//! already fast.
//!
//! ## The estimate and the movement are two different readings, deliberately
//!
//! The **estimate** is an exponentially weighted moving average, and it is the
//! only floating-point value in this module. It is what the human is shown,
//! because a number that jumped to every sample would be unreadable.
//!
//! The **ladder** moves on the raw samples, in whole milliseconds. That split
//! is not an oversight: a single slow frame moves an average for many samples
//! afterwards, so a ladder driven by the average would step down on one spike
//! and then take a dozen good frames to notice. Driven by the samples, "one
//! overrun then a good frame" is exactly what it looks like.
//!
//! ## The report
//!
//! The register is `04-UI-SPEC.md`'s: the fact first, the next step second, no
//! exclamation and no reassurance. What the human is told is what they are
//! getting and why — never a bit rate, which is a number nobody can act on.
//!
//! | State | Fact | Next step |
//! |-------|------|-----------|
//! | at the fastest rung | `Takeover is running at full speed: {rung}.` | — |
//! | stepped down, path relayed | `Your connection to this server is relayed rather than direct, so frames are arriving at {rung}.` | `Clicks and keystrokes all still arrive — takeover works, it just will not feel immediate. A direct path between the two machines is what makes it feel immediate.` |
//! | stepped down, path direct | `This link cannot carry full-speed frames, so they are arriving at {rung}.` | `Tailscale reports a direct path, so the limit is one of the two machines rather than the route between them. Clicks and keystrokes all still arrive.` |
//! | stepped down, path unknown | `This link cannot carry full-speed frames, so they are arriving at {rung}.` | `Clicks and keystrokes all still arrive — takeover works, it just will not feel immediate.` |
//! | any state, once measured | `Input to picture: about {estimate} ms.` | — |
//!
//! `{rung}` is the rung in plain words — "full detail, about 33 frames a
//! second" — derived from the ladder's own numbers rather than written out per
//! rung, so a rung added later describes itself.
//!
//! **The path line degrades rather than disappearing.** Asking the overlay
//! network whether this peer's path is direct or relayed means running a
//! subprocess, and a subprocess can be missing, slow or unhappy. When it
//! cannot be asked the report drops the clause and keeps the line, because a
//! report that vanishes when the platform is unhappy is a report that vanishes
//! exactly when somebody needs it. It also runs on its own thread and is never
//! waited for: the window does not stall on a status command.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use talaria_protocol::wire::{InputMessage, Rung};

/// The weight a new sample carries in the smoothed estimate.
///
/// A quarter. What it trades is legibility against reaction time: a heavier
/// weight follows the link more closely and is noisier to read, a lighter one
/// is steady and lags. The ladder does not depend on this choice at all — it
/// moves on the raw samples — so this number is a presentation decision and can
/// be tuned without changing any behaviour.
const SAMPLE_WEIGHT: f64 = 0.25;

/// How many consecutive overruns step the ladder one rung slower.
///
/// **The small half of the first asymmetry.** Degrading is cheap to undo and
/// expensive to delay: a human who cannot aim a click is stuck now. Three is
/// enough that one slow frame does not move anything and few enough that a
/// genuinely slower link is answered within a tenth of a second of driving.
const OVERRUNS_BEFORE_SLOWER: u32 = 3;

/// How many consecutive comfortable samples step the ladder one rung faster.
///
/// **The large half of the first asymmetry, and it is four times the other
/// one on purpose.** Recovering early on a link that has not actually
/// recovered costs a keyframe and an immediate step back down, which is the
/// oscillation this whole mechanism exists to avoid. A brief spike therefore
/// degrades slowly and a sustained improvement recovers slowly.
const COMFORTABLE_BEFORE_FASTER: u32 = 12;

/// The recovery margin's numerator, over [`RECOVERY_MARGIN_DENOMINATOR`].
///
/// **The second asymmetry, and the one that actually prevents oscillation.**
/// Stepping up on any sample merely *under* the interval would let a link
/// sitting exactly on a rung's edge climb, overrun, fall, and climb again
/// forever — and since a rung change forces a keyframe, that flapping is
/// expensive rather than merely untidy (T-05-12-E).
///
/// **Two fifths, and the number is derived rather than tuned.** The property
/// that has to hold is: a sample good enough to step *up* from one rung must
/// not be an overrun on the rung it steps up to, or the ladder climbs and
/// falls forever on a link that never changed. Most steps of the ladder halve
/// the interval, so the margin has to be at most one half; the slowest step is
/// 120 ms to 250 ms, which allows at most 0.48. Two fifths clears both with
/// room, and a unit test walks the ladder asserting the property directly, so
/// a rung inserted later that broke it fails rather than oscillating in the
/// field.
const RECOVERY_MARGIN_NUMERATOR: u64 = 2;

/// The denominator of [`RECOVERY_MARGIN_NUMERATOR`]. An integer fraction, the
/// same shape the server's keyframe threshold uses, so the comparison stays
/// whole-millisecond arithmetic end to end.
const RECOVERY_MARGIN_DENOMINATOR: u64 = 5;

/// How many outstanding send times are kept.
///
/// A ceiling rather than a courtesy: a client driving a server that has stopped
/// echoing would otherwise accumulate one entry per pointer move forever. The
/// oldest go first, which is also the right order — a sample from four seconds
/// ago is not a measurement of anything current.
const OUTSTANDING_SENDS: usize = 256;

/// What the overlay network says about the path to this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkPath {
    /// Not asked yet, or asked and not answered. The report drops its path
    /// clause and keeps the rest.
    #[default]
    Unknown,
    /// A direct path. The latency target is achievable on one of these.
    Direct,
    /// A relayed path. `05-RESEARCH.md` measured one of these on this very
    /// tailnet at about 13 Mbit/s, where a single full keyframe takes about
    /// three hundred milliseconds to transmit before any latency is added.
    Relayed,
}

/// A shared, late-arriving answer to "is this path direct or relayed?".
///
/// A handle rather than a value because the answer costs a subprocess and the
/// window must not wait for one. [`Link::probing`] spawns a thread that fills
/// it in; everything else reads whatever is there, which is
/// [`LinkPath::Unknown`] until it is not.
#[derive(Debug, Clone, Default)]
pub struct Link {
    path: Arc<Mutex<LinkPath>>,
}

impl Link {
    /// Start asking, on a thread, and answer with the handle immediately.
    pub fn probing(host: String) -> Self {
        let link = Self::default();
        let path = Arc::clone(&link.path);
        let thread = std::thread::Builder::new().name("talaria-link".into()).spawn(move || {
            let answer = probe(&host);
            match path.lock() {
                Ok(mut held) => *held = answer,
                // A poisoned lock means the only other holder panicked, which
                // it cannot: it does nothing but assign. Nothing to do here
                // but leave the path unknown, which the copy already handles.
                Err(_) => log::debug!("the link path could not be recorded"),
            }
        });
        if let Err(error) = thread {
            log::debug!("the link path could not be looked up: {error}");
        }
        link
    }

    /// A handle whose answer is already known. Test-only: on a shipped path
    /// the answer is whatever the overlay network said, and a way to assert one
    /// would be a way to assert one.
    #[cfg(test)]
    pub fn known(path: LinkPath) -> Self {
        Self { path: Arc::new(Mutex::new(path)) }
    }

    /// Whatever is known right now.
    pub fn path(&self) -> LinkPath {
        match self.path.lock() {
            Ok(held) => *held,
            Err(_) => LinkPath::Unknown,
        }
    }
}

/// Ask the overlay network about `host`, or answer [`LinkPath::Unknown`].
///
/// Every failure mode lands on `Unknown`: no such command, a non-zero exit,
/// output that is not what this expects, a peer this host does not name. None
/// of them is worth a line in the window — the report degrades to the part it
/// knows and says nothing about the path.
fn probe(host: &str) -> LinkPath {
    let output = std::process::Command::new("tailscale").arg("status").arg("--json").output();
    match output {
        Ok(output) if output.status.success() => {
            classify_path(&String::from_utf8_lossy(&output.stdout), host)
        },
        Ok(_) => LinkPath::Unknown,
        Err(error) => {
            log::debug!("the overlay network could not be asked about the path: {error}");
            LinkPath::Unknown
        },
    }
}

/// The path to `host` according to one `tailscale status --json` document.
///
/// Split out from [`probe`] with the document as a parameter, the same shape
/// every other environment-reading function in this project takes, so what the
/// parsing does is assertable without a daemon.
///
/// A peer is matched by either of the two names a human can point this client
/// at: its DNS name (with or without the trailing dot, and with or without the
/// tailnet suffix) or one of its addresses. A peer with a current address is on
/// a direct path; a peer with a relay and no current address is not.
pub fn classify_path(status: &str, host: &str) -> LinkPath {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(status) else {
        return LinkPath::Unknown;
    };
    let Some(peers) = document.get("Peer").and_then(|peers| peers.as_object()) else {
        return LinkPath::Unknown;
    };
    let wanted = host.trim_end_matches('.').to_ascii_lowercase();
    for peer in peers.values() {
        if !names(peer, &wanted) {
            continue;
        }
        let current = peer.get("CurAddr").and_then(|value| value.as_str()).unwrap_or_default();
        let relay = peer.get("Relay").and_then(|value| value.as_str()).unwrap_or_default();
        return match (current.is_empty(), relay.is_empty()) {
            (false, _) => LinkPath::Direct,
            (true, false) => LinkPath::Relayed,
            (true, true) => LinkPath::Unknown,
        };
    }
    LinkPath::Unknown
}

/// Whether one peer of a status document is the host this client was pointed
/// at.
fn names(peer: &serde_json::Value, wanted: &str) -> bool {
    let dns = peer
        .get("DNSName")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !dns.is_empty() && (dns == wanted || dns.split('.').next() == Some(wanted)) {
        return true;
    }
    peer.get("TailscaleIPs")
        .and_then(|value| value.as_array())
        .is_some_and(|addresses| {
            addresses.iter().any(|address| address.as_str() == Some(wanted))
        })
}

/// The sequence number one outbound input message carries.
///
/// Every variant has one, and every variant's is read here rather than at the
/// call site: a variant added later that this match forgot would stop
/// compiling, where a call site that forgot it would silently stop measuring.
pub fn message_seq(message: &InputMessage) -> u64 {
    match message {
        InputMessage::MouseMove { seq, .. } => *seq,
        InputMessage::MouseButton { seq, .. } => *seq,
        InputMessage::Wheel { seq, .. } => *seq,
        InputMessage::Key { seq, .. } => *seq,
    }
}

/// Whether `sample_ms` overran a rung whose interval is `interval_ms`.
///
/// **Strictly greater, and the strictness is the contract.** At a sixty
/// millisecond rung, sixty is fine and sixty-one is an overrun. Either reading
/// is defensible in the abstract; what is not defensible is leaving it for
/// somebody to discover by testing, so it is stated here and pinned by a test
/// at the boundary and one millisecond either side.
fn is_overrun(sample_ms: u64, interval_ms: u32) -> bool {
    sample_ms > u64::from(interval_ms)
}

/// Whether `sample_ms` is comfortably under a rung whose interval is
/// `interval_ms` — under the recovery margin rather than merely under the
/// interval. See [`RECOVERY_MARGIN_NUMERATOR`] for the asymmetry this provides.
fn is_comfortable(sample_ms: u64, interval_ms: u32) -> bool {
    sample_ms.saturating_mul(RECOVERY_MARGIN_DENOMINATOR)
        <= u64::from(interval_ms).saturating_mul(RECOVERY_MARGIN_NUMERATOR)
}

/// One rung in the words a human reads, derived from the ladder's own numbers.
///
/// Derived rather than written out per rung so a rung added to the table
/// describes itself, and phrased as detail-and-rate rather than as an interval
/// because "about eight frames a second" is a thing somebody has an intuition
/// for and "a hundred and twenty milliseconds" is not.
pub fn describe(rung: Rung) -> String {
    let detail = match rung.scale_denominator() {
        1 => "full detail",
        _ => "reduced detail",
    };
    let per_second = 1000 / rung.interval_ms().max(1);
    format!("{detail}, about {per_second} frames a second")
}

/// What the human is told, and everything an interface needs to draw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The rung frames are being asked for at.
    pub rung: Rung,
    /// Whether that is below the top of the ladder.
    pub degraded: bool,
    /// What the overlay network said about the path, if it said anything.
    pub path: LinkPath,
    /// The smoothed input-to-photon estimate in milliseconds, once there has
    /// been anything to smooth.
    pub estimate_ms: Option<f64>,
    /// How many times the ladder has moved this session. Not shown to a human
    /// — it is the anti-oscillation property made observable.
    pub changes: u32,
    /// The fact, first.
    pub fact: String,
    /// The next step, second, where there is one.
    pub next_step: Option<String>,
    /// The measured figure, where one has been measured.
    pub measurement: Option<String>,
}

/// The rate controller: what the link is doing, which rung to ask for, and what
/// to tell the human about it.
pub struct RateController {
    /// Send times against the input sequences they went out with, oldest first.
    outstanding: VecDeque<(u64, Instant)>,
    /// The highest sequence this client has actually sent. An echo above it is
    /// a value nothing here produced.
    highest_sent: u64,
    /// The highest sequence already turned into a sample, so one sequence is
    /// measured once.
    sampled: u64,
    /// The smoothed estimate, in milliseconds. The only floating-point value in
    /// this module.
    estimate: Option<f64>,
    /// The rung this controller currently believes the link sustains.
    rung: Rung,
    consecutive_overruns: u32,
    consecutive_comfortable: u32,
    changes: u32,
    /// Whether the next sample is the first of an attachment.
    ///
    /// 05-02 measured the first frame after a lease is taken at 53–112 ms
    /// against a 0.19–5.1 ms steady state, and the pump makes an attachment due
    /// immediately — so that outlier is the very first thing a controller sees.
    /// It is left out of the estimate and out of the counters rather than
    /// smoothed, because it is a measurement of the attach and not of the link.
    settling: bool,
    /// When the last input went out, which is what the driven cadence is
    /// measured from.
    last_input: Option<Instant>,
    /// The idle threshold, read from the same environment variable the server
    /// reads.
    idle: Duration,
    /// The overlay network's verdict on the path, when it arrives.
    link: Link,
}

impl RateController {
    /// A controller for a client pointed at `link`'s host.
    pub fn new(link: Link) -> Self {
        Self {
            outstanding: VecDeque::new(),
            highest_sent: 0,
            sampled: 0,
            estimate: None,
            rung: Rung::FASTEST,
            consecutive_overruns: 0,
            consecutive_comfortable: 0,
            changes: 0,
            settling: true,
            last_input: None,
            idle: Duration::from_millis(talaria_protocol::wire::parse_view_idle_ms(
                std::env::var(talaria_protocol::wire::VIEW_IDLE_ENV).ok().as_deref(),
            )),
            link,
        }
    }

    /// Record that an input carrying `seq` went out at `at`.
    ///
    /// This is also where the driven cadence starts. The server starts it on an
    /// *accepted* input and this starts it on a *sent* one, which differ by one
    /// transmission — but the comparison and the threshold are literally the
    /// same function and the same variable
    /// ([`talaria_protocol::wire::is_driving`],
    /// [`talaria_protocol::wire::VIEW_IDLE_ENV`]), so the two ends cannot come
    /// to disagree about the rule itself.
    pub fn sent(&mut self, seq: u64, at: Instant) {
        self.highest_sent = self.highest_sent.max(seq);
        self.outstanding.push_back((seq, at));
        while self.outstanding.len() > OUTSTANDING_SENDS {
            self.outstanding.pop_front();
        }
        self.last_input = Some(at);
    }

    /// One frame arrived echoing `applied`. Answers the round-trip sample it
    /// yields, in whole milliseconds, or nothing.
    ///
    /// Nothing for three cases, each of which is a real state rather than a
    /// fault: a server that has applied no input yet, a sequence already
    /// measured, and a sequence above anything this client sent. That last is
    /// the one worth naming — an echo this client did not produce is not a
    /// measurement of this client, and treating it as one would let the far end
    /// choose what this client believes about its own link.
    pub fn frame_arrived(&mut self, applied: u64, at: Instant) -> Option<u64> {
        if applied == 0 || applied <= self.sampled || applied > self.highest_sent {
            return None;
        }
        let mut sample = None;
        // Retire this sequence and every earlier one: an older input's frame
        // cannot arrive after a newer one's on an ordered stream, so those send
        // times will never be matched and holding them is holding a leak.
        while let Some(&(seq, when)) = self.outstanding.front() {
            if seq > applied {
                break;
            }
            self.outstanding.pop_front();
            if seq == applied {
                sample = Some(at.saturating_duration_since(when).as_millis() as u64);
            }
        }
        self.sampled = applied;
        sample
    }

    /// Feed one round-trip sample: the estimate follows it, and the ladder may
    /// move.
    pub fn observe(&mut self, sample_ms: u64) {
        if self.settling {
            // The attach outlier. See [`RateController::settling`].
            self.settling = false;
            return;
        }
        self.estimate = Some(match self.estimate {
            None => sample_ms as f64,
            Some(estimate) => estimate + SAMPLE_WEIGHT * (sample_ms as f64 - estimate),
        });
        self.walk(sample_ms);
    }

    /// The hysteretic walk: whole milliseconds, whole counts, two asymmetries.
    ///
    /// A sample that is neither an overrun nor comfortable — the dead band a
    /// rung's own edge sits in — breaks **both** runs. That is what makes a
    /// link parked exactly on an edge leave the ladder where it is instead of
    /// flapping between two rungs, and it is the case the oscillation test
    /// drives.
    fn walk(&mut self, sample_ms: u64) {
        let interval = self.rung.interval_ms();
        if is_overrun(sample_ms, interval) {
            self.consecutive_comfortable = 0;
            self.consecutive_overruns = self.consecutive_overruns.saturating_add(1);
            if self.consecutive_overruns >= OVERRUNS_BEFORE_SLOWER {
                self.move_to(self.rung.slower());
            }
            return;
        }
        if is_comfortable(sample_ms, interval) {
            self.consecutive_overruns = 0;
            self.consecutive_comfortable = self.consecutive_comfortable.saturating_add(1);
            if self.consecutive_comfortable >= COMFORTABLE_BEFORE_FASTER {
                self.move_to(self.rung.faster());
            }
            return;
        }
        self.consecutive_overruns = 0;
        self.consecutive_comfortable = 0;
    }

    /// Adopt `rung`, counting the move and starting the evidence again.
    ///
    /// Both counters start over on every move, including the clamped ones that
    /// change nothing: evidence gathered against one rung's interval says
    /// nothing about another's, and a run that survived a move would let two
    /// rungs' worth of samples add up to one step.
    fn move_to(&mut self, rung: Rung) {
        self.consecutive_overruns = 0;
        self.consecutive_comfortable = 0;
        if rung == self.rung {
            return;
        }
        log::info!(
            "the frame rate is moving from {} to {}",
            describe(self.rung),
            describe(rung),
        );
        self.rung = rung;
        self.changes = self.changes.saturating_add(1);
    }

    /// A new attachment: keep what was learned about the **link**, start again
    /// on what is known about this **attachment**.
    ///
    /// The rung survives on purpose. A client that returned to the top of the
    /// ladder every time the human paused and re-attached would rediscover a
    /// bad link from scratch, which is a keyframe and several seconds of
    /// unusable cadence each time.
    pub fn attached(&mut self) {
        self.outstanding.clear();
        self.sampled = 0;
        self.highest_sent = 0;
        self.settling = true;
    }

    /// Whether the viewer is driving right now.
    ///
    /// [`talaria_protocol::wire::is_driving`] and nothing else: one rule, both
    /// ends, one place.
    pub fn driving(&self, now: Instant) -> bool {
        talaria_protocol::wire::is_driving(
            self.last_input.map(|at| now.saturating_duration_since(at)),
            self.idle,
        )
    }

    /// When the driven cadence lapses, so the loop can wake for it rather than
    /// waiting for a frame to remind it.
    pub fn idle_deadline(&self) -> Option<Instant> {
        self.last_input.map(|at| at + self.idle)
    }

    /// The rung to ask the server for right now.
    ///
    /// Driving: the rung this controller believes the link sustains — **not**
    /// always the fastest one, for the reason [`RateController::attached`]
    /// gives. Not driving: the ladder's slowest rung, which is what "passive"
    /// means, and asking for it changes nothing about what has been learned.
    pub fn requested(&self, now: Instant) -> Rung {
        match self.driving(now) {
            true => self.rung,
            false => Rung::SLOWEST,
        }
    }

    /// The rung the controller believes the link sustains.
    ///
    /// Test-only, like the two below it: everything a shipped path needs is on
    /// [`Report`], which is built once per frame and read by the interface.
    /// Three accessors beside it would be three more ways to read the same
    /// three fields.
    #[cfg(test)]
    pub fn rung(&self) -> Rung {
        self.rung
    }

    /// The smoothed input-to-photon estimate in milliseconds, once there is
    /// one. Test-only; see [`RateController::rung`].
    #[cfg(test)]
    pub fn estimate_ms(&self) -> Option<f64> {
        self.estimate
    }

    /// How many times the ladder has moved. The anti-oscillation property, made
    /// observable rather than argued. Test-only; the interface reads
    /// [`Report::changes`], which is where the end-to-end suite finds it.
    #[cfg(test)]
    pub fn changes(&self) -> u32 {
        self.changes
    }

    /// What to tell the human.
    pub fn report(&self) -> Report {
        let path = self.link.path();
        let degraded = self.rung != Rung::FASTEST;
        let rung = describe(self.rung);
        let (fact, next_step) = match (degraded, path) {
            (false, _) => (format!("Takeover is running at full speed: {rung}."), None),
            (true, LinkPath::Relayed) => (
                format!(
                    "Your connection to this server is relayed rather than direct, so \
                     frames are arriving at {rung}."
                ),
                Some(
                    "Clicks and keystrokes all still arrive — takeover works, it just \
                     will not feel immediate. A direct path between the two machines is \
                     what makes it feel immediate."
                        .to_owned(),
                ),
            ),
            (true, LinkPath::Direct) => (
                format!("This link cannot carry full-speed frames, so they are arriving at {rung}."),
                Some(
                    "Tailscale reports a direct path, so the limit is one of the two \
                     machines rather than the route between them. Clicks and keystrokes \
                     all still arrive."
                        .to_owned(),
                ),
            ),
            // The subprocess was missing, slow or unhappy. The line stays; the
            // clause that needed an answer goes.
            (true, LinkPath::Unknown) => (
                format!("This link cannot carry full-speed frames, so they are arriving at {rung}."),
                Some(
                    "Clicks and keystrokes all still arrive — takeover works, it just \
                     will not feel immediate."
                        .to_owned(),
                ),
            ),
        };
        Report {
            rung: self.rung,
            degraded,
            path,
            estimate_ms: self.estimate,
            changes: self.changes,
            fact,
            next_step,
            // The evidence behind every line above, and the figure 05-11's
            // two-machine script asks a human to write down.
            measurement: self
                .estimate
                .map(|estimate| format!("Input to picture: about {estimate:.0} ms.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> RateController {
        RateController::new(Link::known(LinkPath::Unknown))
    }

    /// A controller past its attach outlier, so a test's first meaningful
    /// sample is its first sample.
    fn settled() -> RateController {
        let mut controller = controller();
        controller.observe(1);
        controller
    }

    fn feed(controller: &mut RateController, sample_ms: u64, times: u32) {
        for _ in 0..times {
            controller.observe(sample_ms);
        }
    }

    // -- sampling ----------------------------------------------------------

    #[test]
    fn a_frame_echoing_a_sequence_this_client_sent_yields_one_sample() {
        let mut controller = controller();
        let start = Instant::now();
        controller.sent(1, start);
        let sample = controller.frame_arrived(1, start + Duration::from_millis(42));
        assert_eq!(sample, Some(42));
    }

    #[test]
    fn a_frame_echoing_a_sequence_this_client_never_sent_yields_nothing() {
        let mut controller = controller();
        let start = Instant::now();
        controller.sent(1, start);
        assert_eq!(controller.frame_arrived(9, start + Duration::from_millis(42)), None);
        // And the send it does hold is still there to be matched.
        assert_eq!(controller.frame_arrived(1, start + Duration::from_millis(50)), Some(50));
    }

    #[test]
    fn a_sequence_is_sampled_once_and_never_twice() {
        let mut controller = controller();
        let start = Instant::now();
        controller.sent(1, start);
        assert_eq!(controller.frame_arrived(1, start + Duration::from_millis(30)), Some(30));
        assert_eq!(
            controller.frame_arrived(1, start + Duration::from_millis(300)),
            None,
            "a second frame echoing the same input produced a second sample",
        );
    }

    #[test]
    fn a_server_that_has_applied_nothing_yet_yields_no_sample() {
        let mut controller = controller();
        controller.sent(1, Instant::now());
        assert_eq!(controller.frame_arrived(0, Instant::now()), None);
    }

    #[test]
    fn an_echo_retires_every_earlier_sequence_with_it() {
        let mut controller = controller();
        let start = Instant::now();
        controller.sent(1, start);
        controller.sent(2, start);
        controller.sent(3, start);
        assert_eq!(controller.frame_arrived(3, start + Duration::from_millis(20)), Some(20));
        assert!(
            controller.outstanding.is_empty(),
            "the send times of superseded inputs were kept and will never be matched",
        );
    }

    // -- the estimate ------------------------------------------------------

    #[test]
    fn the_estimate_moves_toward_a_sample_and_never_jumps_to_it() {
        let mut controller = settled();
        controller.observe(30);
        assert_eq!(controller.estimate_ms(), Some(30.0), "the first sample seeds the estimate");
        controller.observe(130);
        let Some(estimate) = controller.estimate_ms() else {
            panic!("an estimate exists after two samples");
        };
        assert!(estimate > 30.0 && estimate < 130.0, "the estimate jumped: {estimate}");
    }

    #[test]
    fn the_first_sample_of_an_attachment_is_kept_out_of_the_estimate() {
        let mut controller = controller();
        controller.observe(112);
        assert_eq!(
            controller.estimate_ms(),
            None,
            "05-02's attach outlier was smoothed into the estimate",
        );
        controller.observe(30);
        assert_eq!(controller.estimate_ms(), Some(30.0));

        // And re-attaching restores the rule, because the outlier recurs.
        controller.attached();
        controller.observe(112);
        assert_eq!(controller.estimate_ms(), Some(30.0));
    }

    // -- the boundary ------------------------------------------------------

    #[test]
    fn a_sample_exactly_equal_to_the_interval_is_not_an_overrun() {
        let interval = Rung::FASTEST.interval_ms();
        assert!(!is_overrun(u64::from(interval), interval));
        assert!(!is_overrun(u64::from(interval) - 1, interval));
    }

    #[test]
    fn a_sample_one_millisecond_above_the_interval_is_an_overrun() {
        let interval = Rung::FASTEST.interval_ms();
        assert!(is_overrun(u64::from(interval) + 1, interval));
    }

    /// The plan's own example, at sixty: sixty is fine, sixty-one is not.
    #[test]
    fn at_a_sixty_millisecond_rung_sixty_is_fine_and_sixty_one_is_an_overrun() {
        let rung = Rung::FullResolutionSteady;
        assert_eq!(rung.interval_ms(), 60);
        assert!(!is_overrun(59, 60));
        assert!(!is_overrun(60, 60));
        assert!(is_overrun(61, 60));
    }

    // -- the movement ------------------------------------------------------

    #[test]
    fn the_rung_steps_down_after_the_configured_run_of_overruns_and_not_before() {
        let mut controller = settled();
        let over = u64::from(Rung::FASTEST.interval_ms()) + 1;
        for _ in 0..OVERRUNS_BEFORE_SLOWER - 1 {
            controller.observe(over);
            assert_eq!(controller.rung(), Rung::FASTEST, "the ladder moved early");
        }
        controller.observe(over);
        assert_eq!(controller.rung(), Rung::FASTEST.slower());
        assert_eq!(controller.changes(), 1);
    }

    #[test]
    fn one_overrun_followed_by_a_good_sample_moves_nothing() {
        let mut controller = settled();
        let interval = Rung::FASTEST.interval_ms();
        for _ in 0..20 {
            controller.observe(u64::from(interval) + 1);
            // Comfortably under, which also breaks the run of overruns.
            controller.observe(1);
        }
        assert_eq!(
            controller.rung(),
            Rung::FASTEST,
            "a spike every other frame stepped the ladder down",
        );
    }

    #[test]
    fn the_rung_steps_up_after_the_configured_run_of_comfortable_samples_and_not_before() {
        let mut controller = settled();
        feed(&mut controller, u64::from(Rung::FASTEST.interval_ms()) + 1, OVERRUNS_BEFORE_SLOWER);
        let stepped = controller.rung();
        assert_ne!(stepped, Rung::FASTEST);

        for _ in 0..COMFORTABLE_BEFORE_FASTER - 1 {
            controller.observe(1);
            assert_eq!(controller.rung(), stepped, "the ladder recovered early");
        }
        controller.observe(1);
        assert_eq!(controller.rung(), stepped.faster());
    }

    /// A sample merely *under* the interval is not evidence of the faster rung.
    #[test]
    fn a_sample_under_the_interval_but_over_the_recovery_margin_does_not_step_up() {
        let mut controller = settled();
        feed(&mut controller, u64::from(Rung::FASTEST.interval_ms()) + 1, OVERRUNS_BEFORE_SLOWER);
        let stepped = controller.rung();
        // Under the interval, over the recovery margin.
        let lukewarm = u64::from(stepped.interval_ms()) - 1;
        assert!(!is_overrun(lukewarm, stepped.interval_ms()));
        assert!(!is_comfortable(lukewarm, stepped.interval_ms()));
        feed(&mut controller, lukewarm, COMFORTABLE_BEFORE_FASTER * 4);
        assert_eq!(controller.rung(), stepped, "a link just under the edge climbed anyway");
    }

    /// **The invariant the recovery margin exists for, walked over the whole
    /// ladder.** A sample good enough to step *up* from one rung must not be an
    /// overrun on the rung it steps up to. Without that, a link that never
    /// changed climbs, overruns, falls and climbs again forever — and every one
    /// of those moves costs a keyframe. A rung inserted later that broke the
    /// property fails here rather than oscillating in the field.
    #[test]
    fn a_sample_good_enough_to_step_up_is_never_an_overrun_on_the_rung_it_reaches() {
        for entry in talaria_protocol::wire::RUNG_LADDER {
            let faster = entry.rung.faster();
            if faster == entry.rung {
                continue;
            }
            // The slowest sample that still counts as comfortable on this rung,
            // which is the worst link that can trigger a step up from it.
            let worst = u64::from(entry.interval_ms) * RECOVERY_MARGIN_NUMERATOR
                / RECOVERY_MARGIN_DENOMINATOR;
            assert!(is_comfortable(worst, entry.interval_ms), "{}", entry.name);
            assert!(
                !is_overrun(worst, faster.interval_ms()),
                "a link at {worst} ms steps up from {} into an immediate overrun on {}, \
                 which is an oscillation the recovery margin was supposed to prevent",
                entry.name,
                faster.name(),
            );
        }
    }

    /// **The oscillation case.** A long run of samples sitting exactly on a
    /// rung's interval must leave the ladder exactly where it is — a rung
    /// change forces a keyframe, so flapping is expensive rather than untidy.
    #[test]
    fn a_long_run_of_samples_exactly_at_the_edge_moves_the_rung_in_neither_direction() {
        let mut controller = settled();
        let edge = u64::from(Rung::FASTEST.interval_ms());
        feed(&mut controller, edge, 500);
        assert_eq!(controller.rung(), Rung::FASTEST);
        assert_eq!(controller.changes(), 0, "the ladder oscillated on a link parked at an edge");
    }

    #[test]
    fn the_rung_never_steps_past_the_slowest_entry_in_the_table() {
        let mut controller = settled();
        feed(&mut controller, 100_000, 500);
        assert_eq!(controller.rung(), Rung::SLOWEST);
        let settled_changes = controller.changes();
        feed(&mut controller, 100_000, 500);
        assert_eq!(controller.rung(), Rung::SLOWEST);
        assert_eq!(controller.changes(), settled_changes, "clamping still counted as moving");
    }

    #[test]
    fn the_rung_never_steps_past_the_fastest_entry_in_the_table() {
        let mut controller = settled();
        feed(&mut controller, 1, 500);
        assert_eq!(controller.rung(), Rung::FASTEST);
        assert_eq!(controller.changes(), 0);
    }

    // -- what is asked for -------------------------------------------------

    #[test]
    fn entering_the_driven_cadence_asks_for_the_rung_the_link_sustains() {
        let mut controller = settled();
        feed(&mut controller, u64::from(Rung::FASTEST.interval_ms()) + 1, OVERRUNS_BEFORE_SLOWER);
        let sustained = controller.rung();
        assert_ne!(sustained, Rung::FASTEST);
        let now = Instant::now();
        controller.sent(1, now);
        assert_eq!(
            controller.requested(now),
            sustained,
            "the client returned to the top of the ladder rather than to what it had learned",
        );
    }

    #[test]
    fn returning_to_the_passive_cadence_asks_for_the_slowest_rung_and_forgets_nothing() {
        let mut controller = settled();
        feed(&mut controller, u64::from(Rung::FASTEST.interval_ms()) + 1, OVERRUNS_BEFORE_SLOWER);
        let sustained = controller.rung();
        let now = Instant::now();
        controller.sent(1, now);
        assert_eq!(controller.requested(now + controller.idle), Rung::SLOWEST);
        assert_eq!(controller.rung(), sustained, "going idle unlearned the link");
        assert_eq!(controller.requested(now), sustained, "an input did not re-enter the cadence");
    }

    /// The same boundary the server keeps, through the same function.
    #[test]
    fn the_driven_cadence_lapses_exactly_at_the_idle_threshold() {
        let mut controller = controller();
        let now = Instant::now();
        assert!(!controller.driving(now), "a viewer that never drove was driving");
        controller.sent(1, now);
        assert!(controller.driving(now));
        assert!(controller.driving(now + controller.idle - Duration::from_millis(1)));
        assert!(
            !controller.driving(now + controller.idle),
            "the threshold itself was still driven, so 'reaches' meant 'exceeds'",
        );
        assert!(!controller.driving(now + controller.idle + Duration::from_millis(1)));
        assert_eq!(controller.idle_deadline(), Some(now + controller.idle));
    }

    // -- the report --------------------------------------------------------

    #[test]
    fn the_report_names_the_rung_in_plain_words_at_every_rung() {
        for rung in [
            Rung::FullResolutionFast,
            Rung::FullResolutionSteady,
            Rung::HalfResolutionSteady,
            Rung::HalfResolutionSlow,
            Rung::PassiveOnly,
        ] {
            let words = describe(rung);
            assert!(words.contains("frames a second"), "{words}");
            assert!(
                words.contains("full detail") || words.contains("reduced detail"),
                "{words}",
            );
        }
        assert_eq!(describe(Rung::FASTEST), "full detail, about 33 frames a second");
        assert_eq!(describe(Rung::SLOWEST), "reduced detail, about 4 frames a second");
    }

    #[test]
    fn a_degraded_report_on_a_relayed_path_names_the_relay_and_says_driving_still_works() {
        let mut controller = RateController::new(Link::known(LinkPath::Relayed));
        controller.observe(1);
        feed(&mut controller, 100_000, OVERRUNS_BEFORE_SLOWER);
        let report = controller.report();
        assert!(report.degraded);
        assert!(report.fact.contains("relayed"), "{}", report.fact);
        assert!(report.fact.contains(&describe(report.rung)), "{}", report.fact);
        let Some(next_step) = report.next_step else {
            panic!("a degraded report gave the human no next step");
        };
        assert!(next_step.contains("Clicks and keystrokes all still arrive"), "{next_step}");
    }

    /// The platform could not be asked. The line stays and loses its clause.
    #[test]
    fn a_degraded_report_still_renders_when_the_path_could_not_be_looked_up() {
        let mut controller = settled();
        feed(&mut controller, 100_000, OVERRUNS_BEFORE_SLOWER);
        let report = controller.report();
        assert_eq!(report.path, LinkPath::Unknown);
        assert!(report.degraded);
        assert!(!report.fact.is_empty(), "the whole report vanished with the subprocess");
        assert!(!report.fact.contains("relayed"), "{}", report.fact);
        assert!(report.next_step.is_some());
    }

    #[test]
    fn an_undegraded_report_says_so_and_offers_no_next_step() {
        let report = settled().report();
        assert!(!report.degraded);
        assert!(report.fact.contains("full speed"), "{}", report.fact);
        assert_eq!(report.next_step, None);
    }

    #[test]
    fn the_report_carries_the_measured_millisecond_figure_once_there_is_one() {
        let mut controller = settled();
        assert_eq!(controller.report().measurement, None);
        controller.observe(47);
        assert_eq!(
            controller.report().measurement,
            Some("Input to picture: about 47 ms.".to_owned()),
        );
        assert_eq!(controller.report().estimate_ms, Some(47.0));
    }

    // -- the path ----------------------------------------------------------

    const STATUS: &str = r#"{"Peer": {
        "nodekey:aaa": {"DNSName": "thinkpad.tailnet.ts.net.",
                        "TailscaleIPs": ["100.118.105.121"],
                        "Relay": "", "CurAddr": "192.168.1.176:41641"},
        "nodekey:bbb": {"DNSName": "minipc.tailnet.ts.net.",
                        "TailscaleIPs": ["100.64.0.9"],
                        "Relay": "fra", "CurAddr": ""}
    }}"#;

    #[test]
    fn a_peer_with_a_current_address_is_on_a_direct_path() {
        assert_eq!(classify_path(STATUS, "thinkpad.tailnet.ts.net"), LinkPath::Direct);
        assert_eq!(classify_path(STATUS, "thinkpad"), LinkPath::Direct);
        assert_eq!(classify_path(STATUS, "100.118.105.121"), LinkPath::Direct);
    }

    #[test]
    fn a_peer_with_a_relay_and_no_current_address_is_on_a_relayed_path() {
        assert_eq!(classify_path(STATUS, "minipc"), LinkPath::Relayed);
        assert_eq!(classify_path(STATUS, "minipc.tailnet.ts.net."), LinkPath::Relayed);
    }

    #[test]
    fn anything_that_is_not_a_peer_of_this_tailnet_is_simply_unknown() {
        assert_eq!(classify_path(STATUS, "127.0.0.1"), LinkPath::Unknown);
        assert_eq!(classify_path(STATUS, "somebody-elses-machine"), LinkPath::Unknown);
        assert_eq!(classify_path("", "thinkpad"), LinkPath::Unknown);
        assert_eq!(classify_path("not json at all", "thinkpad"), LinkPath::Unknown);
        assert_eq!(classify_path("{}", "thinkpad"), LinkPath::Unknown);
        assert_eq!(classify_path("{\"Peer\": null}", "thinkpad"), LinkPath::Unknown);
    }

    #[test]
    fn every_input_variant_carries_its_sequence_where_this_module_reads_it() {
        use talaria_protocol::wire::{ButtonAction, PointerButton, WheelMode};
        let messages = [
            InputMessage::MouseMove { tab: 1, seq: 11, x: 0.0, y: 0.0 },
            InputMessage::MouseButton {
                tab: 1,
                seq: 12,
                x: 0.0,
                y: 0.0,
                button: PointerButton::Left,
                action: ButtonAction::Down,
            },
            InputMessage::Wheel {
                tab: 1,
                seq: 13,
                x: 0.0,
                y: 0.0,
                dx: 0.0,
                dy: 1.0,
                mode: WheelMode::Line,
            },
            InputMessage::Key {
                tab: 1,
                seq: 14,
                state: ButtonAction::Down,
                key: Some("a".into()),
                named: None,
            },
        ];
        let sequences: Vec<u64> = messages.iter().map(message_seq).collect();
        assert_eq!(sequences, vec![11, 12, 13, 14]);
    }

    /// The outstanding-send table has a ceiling, so a server that stopped
    /// echoing cannot make this client grow without bound.
    #[test]
    fn the_outstanding_send_table_has_a_ceiling() {
        let mut controller = controller();
        let now = Instant::now();
        for seq in 1..=(OUTSTANDING_SENDS as u64 * 3) {
            controller.sent(seq, now);
        }
        assert_eq!(controller.outstanding.len(), OUTSTANDING_SENDS);
        // And the ones kept are the recent ones, which are the measurable ones.
        assert_eq!(
            controller.outstanding.front().map(|(seq, _)| *seq),
            Some(OUTSTANDING_SENDS as u64 * 2 + 1),
        );
    }
}
