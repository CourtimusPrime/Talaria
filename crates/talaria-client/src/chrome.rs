//! The client's own interface: where the connection is, what the server says
//! its agent tabs are, and what a first-time user has to do about it.
//!
//! Three surfaces and no more. There is no design contract for this phase — the
//! interface gate did not fire, because this client is a new binary rather than
//! a change to the existing chrome — so the register is
//! `04-UI-SPEC.md`'s: plain sentences, the fact first and the next step second,
//! no exclamation, no reassurance, no jargon. A value a page supplied is fenced
//! and marked as a claim rather than repeated as a fact; the *server address*
//! is not, because it is a value the human typed.
//!
//! ## The connection copy
//!
//! Every one of [`crate::net::ConnectionState`]'s variants has its own line and
//! its own next step, because that is why they are separate variants at all.
//!
//! | State | Fact | Next step |
//! |-------|------|-----------|
//! | not started | `Not connected yet.` | `This line changes on its own when the connection opens.` |
//! | connecting | `Connecting to {server}…` | — |
//! | connected | `Connected to {server}.` | — (the tab list below is the next thing to read) |
//! | missing credential | `No credential is configured, so nothing has been tried yet.` | the pairing block below |
//! | unreachable | `Nothing answered at {server}.` | `Check the address, and check that remote access is turned on in the server's own window.` |
//! | untrusted | `The server's certificate did not verify, so no credential was sent.` | `Talaria checks the certificate against this machine's trust store, and nothing turns that off. Check that {server} is the name the certificate was issued for.` |
//! | refused | `{server} declined the connection.` | `The server does not say why, on purpose — a refusal that gave a reason would let anyone find out which credentials exist. Check that this client is still approved on the server, and that the token here is the current one.` |
//! | version mismatch | `The server speaks view protocol {server}; this client speaks {client}.` | `The two were built against different wires. Update whichever is older.` |
//! | dropped | `The connection to {server} ended.` | `Reconnect opens it again.` |
//!
//! ## The tab list
//!
//! | Element | Copy |
//! |---------|------|
//! | Row, primary | `"{sanitize_claim(truncate_chars(title, 48))}"` followed by a `Small` `(as claimed)` |
//! | Row, secondary (`Small`) | `"{sanitize_claim(truncate_chars(url, 48))}"` |
//! | Empty state | `No agent has opened a tab on this server.` |
//! | Empty state, next step (`Small`) | `Tabs appear here when an agent opens one. This client cannot open one itself.` |
//!
//! No count label anywhere, matching the design contract's rule.
//!
//! ## The pairing block
//!
//! | Element | Copy |
//! |---------|------|
//! | Fact | `Getting a token needs someone at the server machine.` |
//! | Detail (`Small`) | `Talaria asks for approval in its own window, on the computer running the server, so a person has to be there to approve this client the first time. After that the token works from here.` |
//! | Where it is read from (`Small`) | `Set TALARIA_CLIENT_TOKEN, or put the token in {path} and chmod 600 it.` |
//!
//! That constraint is stated rather than hidden, and it is not dressed up as a
//! security feature: it is a consequence of where the approval surface lives.
//! `05-01`'s `deferred-items.md` records it as an open product question with two
//! named alternatives — the device authorization grant, and a bespoke pairing
//! code — and this phase is written against accepting it. A reader who finds
//! this copy should be able to find that entry.

use talaria_protocol::{ChromeRect, TabInfo};

use crate::net::ConnectionState;
use crate::present::{Fit, Presenter};

/// How wide the client's own controls are, in logical points.
///
/// Fixed rather than proportional, and a **side** panel rather than a strip
/// above the page: the page area is then a large rectangle whose aspect ratio
/// has nothing to do with the server's, which is what keeps the letterboxed
/// margin a real place a pointer can be rather than a hairline that only exists
/// on some window sizes.
const CONTROLS_WIDTH: f32 = 340.0;

/// How many characters of a page-supplied string are rendered.
///
/// The same 48 the server's own claim rendering uses, for the same reason: a
/// 10 KB title must not be able to push anything off screen or widen a pane.
const CLAIM_LIMIT: usize = 48;

/// The intents the interface produces, applied by the caller after the
/// interface's borrows drop.
///
/// **The interface mutates nothing.** That is the convention the server's
/// chrome follows and which this project treats as a rule rather than a
/// preference: an egui closure cannot write the state it was drawn from without
/// racing the frame it is in the middle of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// Open the connection again.
    ///
    /// The one control this interface has, and it exists because every failed
    /// state's next step ends the same way: fix the thing named, then try
    /// again. It is deliberately a **press**, never a timer — see
    /// [`crate::net`]'s no-retry-loop note. A client that reconnected on its own
    /// forever would hide all nine states behind a spinner.
    Reconnect,
    /// Watch this tab: ask the server for its frames.
    ///
    /// One tab at a time, because one window shows one picture. Attaching to a
    /// second detaches the first rather than opening a second stream nobody can
    /// see — an invisible stream is bandwidth a human cannot account for, and on
    /// the server side it is a render lease held on a tab nobody is watching.
    Attach(u64),
    /// Stop watching this tab, releasing the server's render lease.
    Detach(u64),
}

/// The first `limit` characters of `text`, with an ellipsis when there was more.
///
/// Counted in characters, not bytes: `String::truncate` takes a byte index and
/// panics when it lands inside a multibyte character, and a page title is
/// exactly where one turns up.
///
/// **A second copy of the server's helper, deliberately.** The client cannot
/// call the shell's: `talaria-shell` is a binary crate with the web engine
/// linked into it, and depending on it would undo the one property this whole
/// binary exists for. `talaria-protocol` is the shared vocabulary and is
/// deliberately dependency-light and free of display concerns, so it is not the
/// home for this either. Nine lines duplicated is the cheaper of the two costs.
fn truncate_chars(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

/// Characters that let a string lie about itself once it is a label.
///
/// Control characters take text out of the row it was given; the Unicode
/// bidirectional overrides and isolates reorder what follows them. Same two
/// classes, same reasoning, as the server's own predicate.
fn is_display_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// [`is_display_unsafe`] plus the delimiter rule: an ASCII double quote becomes
/// U+FFFD too.
///
/// **Here the quote is the delimiter.** A title is rendered inside literal
/// quotes, so a page titled `Talaria" — verified by Talaria "` would otherwise
/// render as though this client vouched for it. U+FFFD rather than removal, for
/// the server's stated reason: a dropped character lets a string shorten
/// silently into something else plausible, where the replacement character shows
/// the human that something was there.
fn sanitize_claim(text: &str) -> String {
    text.chars()
        .map(|character| match is_display_unsafe(character) || character == '"' {
            true => '\u{FFFD}',
            false => character,
        })
        .collect()
}

/// A page-supplied string, ready to render: truncated, sanitised and quoted.
fn claimed(text: &str) -> String {
    format!("\"{}\"", sanitize_claim(&truncate_chars(text, CLAIM_LIMIT)))
}

/// Note down where one named control was laid out this frame.
///
/// `rects` is `Some` only under the test hook, so a production frame does not
/// build a name, does not push a rect and has nothing to hand out — the absence
/// of the collection *is* the off switch. `index` names one row of a list; the
/// formatting happens inside the `Some` arm so a build without the hook does not
/// allocate a string per row per frame.
///
/// This exists so `05-09`'s and `05-10`'s suites can find real controls by name
/// instead of hardcoding coordinates, which is the mistake the server's own
/// geometry hook exists to have deleted.
fn record_rect(
    rects: Option<&mut Vec<ChromeRect>>,
    name: &str,
    index: Option<usize>,
    rect: egui::Rect,
) {
    let Some(rects) = rects else { return };
    rects.push(ChromeRect {
        name: match index {
            Some(index) => format!("{name}.{index}"),
            None => name.to_owned(),
        },
        x: rect.left(),
        y: rect.top(),
        width: rect.width(),
        height: rect.height(),
    });
}

/// Everything one frame needs to know, passed in rather than reached for.
pub struct View<'a> {
    /// The server address as the human typed it. Not a claim.
    pub server: &'a str,
    /// Where the connection is.
    pub state: &'a ConnectionState,
    /// The server's agent tabs, **in the server's order**.
    pub tabs: &'a [TabInfo],
    /// Where a credential file would be read from, for the pairing copy.
    pub credential_file: Option<&'a std::path::Path>,
    /// The tab whose frames are being shown, if any.
    ///
    /// The human needs to know what they are looking at and, once there is
    /// input, what their clicks are going into: a viewer attached to nothing
    /// must not look identical to one that is attached.
    pub attached: Option<u64>,
}

/// The client's interface.
pub struct Chrome {
    /// Present only under the test hook. Read once at construction rather than
    /// per frame: the variable cannot change under a running process, and a
    /// lock and an allocation per frame to learn the same answer is not free.
    chrome_rects: Option<Vec<ChromeRect>>,
    /// How the picture was laid out on the last drawn frame. Recorded here
    /// because the pointer mapping needs it and must not compute its own.
    fit: Option<Fit>,
}

impl Chrome {
    pub fn new() -> Self {
        Self {
            chrome_rects: match std::env::var("TALARIA_TEST_HOOKS").as_deref() == Ok("1") {
                true => Some(Vec::new()),
                false => None,
            },
            fit: None,
        }
    }

    /// Where every named control was laid out on the last drawn frame.
    pub fn chrome_rects(&self) -> &[ChromeRect] {
        self.chrome_rects.as_deref().unwrap_or_default()
    }

    /// How the picture was laid out on the last drawn frame, if one was.
    ///
    /// **The value [`crate::input`] inverts.** It is read from here rather than
    /// recomputed there, which is the whole of the one-transform rule: a change
    /// to how the picture is laid out cannot leave the pointer mapping behind,
    /// because there is no second mapping to leave behind.
    pub fn layout(&self) -> Option<Fit> {
        self.fit
    }

    /// Draw one frame and return what the human asked for.
    ///
    /// Rebuilds the rect collection from scratch, so a control that stopped
    /// being drawn stops being reported — a stale rect for a control that is no
    /// longer there would be worse than no rect at all.
    pub fn update(
        &mut self,
        ui: &mut egui::Ui,
        view: &View<'_>,
        present: &Presenter,
        sent: u64,
    ) -> Vec<UiAction> {
        let mut actions: Vec<UiAction> = Vec::new();
        let mut rects = self.chrome_rects.is_some().then(Vec::new);

        egui::Panel::left("talaria-client-controls")
            .exact_size(CONTROLS_WIDTH)
            .resizable(false)
            .show_inside(ui, |ui| {
                connection_surface(ui, view, rects.as_mut(), &mut actions);
                // **The tab surface exists only while connected**, and that
                // includes its empty state. "No agent has opened a tab on this
                // server" is a statement about the server, and a client that
                // never reached one is in no position to make it — drawing it
                // under a missing-credential line would tell a human two
                // contradictory things at once and invite them to go looking
                // for the agent rather than for the token.
                if matches!(view.state, ConnectionState::Connected) {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    let watching = match view.attached {
                        Some(tab) => format!("Watching tab {tab}."),
                        None => "Not watching any tab.".to_owned(),
                    };
                    let indication = ui.label(watching);
                    record_rect(rects.as_mut(), "attachment.state", None, indication.rect);
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        tab_surface(ui, view, rects.as_mut(), &mut actions);
                    });
                }
            });

        // Everything the controls did not take is the page.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let area = ui.max_rect();
            record_rect(rects.as_mut(), "page.area", None, area);
            self.fit = present.draw(ui, area);
            if let Some(fit) = self.fit {
                // The fitted picture, in the same logical points every other
                // recorded control is in. An end-to-end suite aiming a real
                // pointer at a page coordinate needs exactly this rectangle and
                // the page's own size, and reads both here rather than
                // recomputing the client's transform outside the client.
                record_rect(rects.as_mut(), "page.surface", None, fit.surface_rect());
            }
        });

        record_readings(rects.as_mut(), present, sent, ui.ctx().pixels_per_point());

        if let Some(rects) = rects {
            self.chrome_rects = Some(rects);
        }
        actions
    }
}

/// The attachment's numbers, put on the geometry channel under the test hook.
///
/// **These are readings, not rectangles**, and the convention is stated here
/// rather than left to be inferred: a `reading.` entry carries its value in
/// `width` (and, for a size, in `height`), with `x` and `y` zero. They ride the
/// geometry line because that line already exists, is already drained by the
/// harness, and is already gated on the same variable — a second channel for
/// four numbers would be a second thing to keep working.
///
/// Why they are exposed at all: "the client is connected" is not "the client is
/// showing the page", and an end-to-end suite that asserted only the former
/// would pass against a client whose picture never arrived.
fn record_readings(
    rects: Option<&mut Vec<ChromeRect>>,
    present: &Presenter,
    sent: u64,
    points_per_pixel: f32,
) {
    let Some(rects) = rects else { return };
    let mut reading = |name: &str, width: f32, height: f32| {
        rects.push(ChromeRect { name: name.to_owned(), x: 0.0, y: 0.0, width, height });
    };
    if let Some(tab) = present.attached() {
        reading("reading.attached_tab", tab as f32, 0.0);
    }
    if let Some(size) = present.size() {
        // The **page's** size in its own device pixels — what an input
        // coordinate is expressed in — not the texture's.
        reading("reading.page_size", size.page_width() as f32, size.page_height() as f32);
        reading("reading.denominator", f32::from(size.denominator), 0.0);
    }
    reading("reading.frame_seq", present.last_frame_seq() as f32, 0.0);
    reading("reading.last_applied_input", present.last_applied_input() as f32, 0.0);
    reading("reading.input_seq", sent as f32, 0.0);
    // Every rectangle above is in logical points. A caller converting one to a
    // screen coordinate needs this to finish the job, and it is the client's own
    // rather than the server's — the two windows may be on displays with
    // different scale factors, which is the ordinary case for the machine pair
    // this whole phase exists for.
    reading("reading.points_per_pixel", points_per_pixel, 0.0);
}

impl Default for Chrome {
    fn default() -> Self {
        Self::new()
    }
}

/// Surface one: where the connection is, and what to do about it.
fn connection_surface(
    ui: &mut egui::Ui,
    view: &View<'_>,
    mut rects: Option<&mut Vec<ChromeRect>>,
    actions: &mut Vec<UiAction>,
) {
    let server = view.server;
    let (fact, next_step) = match view.state {
        ConnectionState::NotStarted => (
            "Not connected yet.".to_owned(),
            Some("This line changes on its own when the connection opens.".to_owned()),
        ),
        ConnectionState::Connecting => (format!("Connecting to {server}…"), None),
        ConnectionState::Connected => (format!("Connected to {server}."), None),
        ConnectionState::MissingCredential => (
            "No credential is configured, so nothing has been tried yet.".to_owned(),
            None,
        ),
        ConnectionState::Unreachable => (
            format!("Nothing answered at {server}."),
            Some(
                "Check the address, and check that remote access is turned on in the \
                 server's own window."
                    .to_owned(),
            ),
        ),
        // The one state whose next step is about the certificate. It says what
        // was checked and that nothing turns the check off, because a human who
        // does not know that will go looking for the switch.
        ConnectionState::Untrusted => (
            "The server's certificate did not verify, so no credential was sent.".to_owned(),
            Some(format!(
                "Talaria checks the certificate against this machine's trust store, and \
                 nothing turns that off. Check that {server} is the name the certificate \
                 was issued for."
            )),
        ),
        ConnectionState::Refused => (
            format!("{server} declined the connection."),
            Some(
                "The server does not say why, on purpose — a refusal that gave a reason \
                 would let anyone find out which credentials exist. Check that this client \
                 is still approved on the server, and that the token here is the current one."
                    .to_owned(),
            ),
        ),
        ConnectionState::VersionMismatch { server: theirs, client: ours } => (
            format!("The server speaks view protocol {theirs}; this client speaks {ours}."),
            Some("The two were built against different wires. Update whichever is older.".to_owned()),
        ),
        ConnectionState::Dropped => (
            format!("The connection to {server} ended."),
            Some("Reconnect opens it again.".to_owned()),
        ),
    };

    let fact_label = ui.label(&fact);
    record_rect(rects.as_deref_mut(), "connection.state", None, fact_label.rect);
    if let Some(next_step) = next_step {
        ui.label(egui::RichText::new(next_step).small());
    }

    if matches!(view.state, ConnectionState::MissingCredential) {
        pairing_surface(ui, view, rects.as_deref_mut());
    }

    // Offered only where it is the next step. A Reconnect on a live connection
    // would be a button whose only effect is to interrupt one.
    let can_reconnect = matches!(
        view.state,
        ConnectionState::MissingCredential
            | ConnectionState::Unreachable
            | ConnectionState::Untrusted
            | ConnectionState::Refused
            | ConnectionState::VersionMismatch { .. }
            | ConnectionState::Dropped
    );
    if can_reconnect {
        ui.add_space(8.0);
        let button = ui.button("Reconnect");
        record_rect(rects, "connection.reconnect", None, button.rect);
        if button.clicked() {
            actions.push(UiAction::Reconnect);
        }
    }
}

/// Surface three: the first-run copy, shown when no credential is configured.
///
/// It states the pairing constraint plainly and says what to do about it. It
/// does not apologise for it and does not present it as a feature.
fn pairing_surface(ui: &mut egui::Ui, view: &View<'_>, rects: Option<&mut Vec<ChromeRect>>) {
    ui.add_space(8.0);
    let heading = ui.label("Getting a token needs someone at the server machine.");
    record_rect(rects, "pairing.constraint", None, heading.rect);
    ui.label(
        egui::RichText::new(
            "Talaria asks for approval in its own window, on the computer running the \
             server, so a person has to be there to approve this client the first time. \
             After that the token works from here.",
        )
        .small(),
    );
    let where_from = match view.credential_file {
        Some(path) => format!(
            "Set {} in this client's environment, or put the token in {} and chmod 600 it.",
            crate::net::TOKEN_ENV,
            path.display(),
        ),
        None => format!("Set {} in this client's environment.", crate::net::TOKEN_ENV),
    };
    ui.label(egui::RichText::new(where_from).small());
}

/// Surface two: the agent tab list, and the empty state that is not an error.
fn tab_surface(
    ui: &mut egui::Ui,
    view: &View<'_>,
    mut rects: Option<&mut Vec<ChromeRect>>,
    actions: &mut Vec<UiAction>,
) {
    if view.tabs.is_empty() {
        // **The connected-and-empty case specifically**, and it is distinct
        // from every not-connected state above it: a human who cannot tell
        // "this server has no agent tabs" from "this client never connected"
        // will restart the wrong thing. The connection line says which, which
        // is why this line does not have to.
        let empty = ui.label("No agent has opened a tab on this server.");
        record_rect(rects.as_deref_mut(), "tabs.empty", None, empty.rect);
        ui.label(
            egui::RichText::new(
                "Tabs appear here when an agent opens one. This client cannot open one itself.",
            )
            .small(),
        );
        return;
    }

    // `enumerate` over the slice as given. **No sort, no reverse.** The server
    // keeps its own insertion order for a reason, and reordering here would
    // reintroduce exactly the instability that avoids — two tabs created in the
    // same millisecond would swap places between reads.
    for (row, tab) in view.tabs.iter().enumerate() {
        let watching = view.attached == Some(tab.tab_id);
        let fill = match (watching, row % 2) {
            (true, _) => ui.visuals().selection.bg_fill,
            (false, 1) => ui.visuals().faint_bg_color,
            (false, _) => egui::Color32::TRANSPARENT,
        };
        egui::Frame::new().fill(fill).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    // Both the title and the address come from a page, so both
                    // are rendered as claims — truncated, sanitised, quoted —
                    // exactly as the server's own lists treat one.
                    let title = ui.label(claimed(&tab.title));
                    record_rect(rects.as_deref_mut(), "tabs.row", Some(row), title.rect);
                    ui.label(egui::RichText::new("(as claimed)").small());
                });
                ui.label(egui::RichText::new(claimed(&tab.url)).small());
                // The one control per row, and its label says which of the two
                // states this row is in — a viewer attached to nothing must not
                // look identical to one that is watching.
                let control = match watching {
                    true => ui.button("Stop watching"),
                    false => ui.button("Watch"),
                };
                record_rect(
                    rects.as_deref_mut(),
                    match watching {
                        true => "tabs.detach",
                        false => "tabs.attach",
                    },
                    Some(row),
                    control.rect,
                );
                if control.clicked() {
                    actions.push(match watching {
                        true => UiAction::Detach(tab.tab_id),
                        false => UiAction::Attach(tab.tab_id),
                    });
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_title_is_cut_to_the_claim_limit() {
        let long = "a".repeat(200);
        let rendered = claimed(&long);
        assert!(rendered.starts_with('"') && rendered.ends_with('"'));
        assert_eq!(rendered.chars().count(), CLAIM_LIMIT + 3);
    }

    #[test]
    fn a_title_cannot_close_the_quotes_it_is_rendered_inside() {
        assert_eq!(claimed("Talaria\" verified by Talaria \""), "\"Talaria\u{FFFD} verified by Talaria \u{FFFD}\"");
    }

    #[test]
    fn a_title_cannot_take_a_second_line_or_reorder_what_follows_it() {
        assert_eq!(claimed("invoice\n\nfrom your bank"), "\"invoice\u{FFFD}\u{FFFD}from your bank\"");
        assert_eq!(claimed("report\u{202E}fdp.exe"), "\"report\u{FFFD}fdp.exe\"");
    }

    #[test]
    fn truncation_counts_characters_rather_than_bytes() {
        // Four bytes each; a byte-index truncate would land inside one.
        let emoji = "🌍".repeat(60);
        assert_eq!(truncate_chars(&emoji, 3), "🌍🌍🌍…");
    }
}
