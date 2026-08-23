//! The client's one connection to one server: where the credential comes from,
//! what the certificate has to satisfy, and every way the connection can end.
//!
//! One named thread owns the socket, and it reaches the event loop the only way
//! anything off-thread reaches it — by sending an [`Update`] down a cloned
//! `EventLoopProxy`. **This thread never touches interface state.** That is the
//! same one-way discipline the server keeps between its listener thread and its
//! engine thread, and it is what keeps the borrow hazards the intent convention
//! exists to prevent from ever arising here.
//!
//! The thread is spawned in the degrade-with-an-event shape rather than the
//! expect-and-die one (`crates/talaria-shell/src/http.rs`'s listener thread,
//! not `control.rs`'s): a client that cannot connect is still a window with a
//! readable message in it, and there is nothing about a failed connection that
//! should cost a human their window.
//!
//! ## The credential, and where it must not come from
//!
//! [`TOKEN_ENV`] first, then an owner-only file in the client's own
//! configuration directory. **Never a command-line argument**, and the reason
//! is not stylistic: on this platform every account on the machine can read
//! another process's arguments out of `/proc`, so a bearer token in `argv` is a
//! bearer token in a world-readable file that happens to be virtual. It is also
//! never interpolated into a log line at any level — the rule the server's
//! authorization module states next to the short-digest helper that satisfies
//! it. Nothing in this module needs to tell two tokens apart in a log, so
//! nothing here names one at all.
//!
//! ## The certificate, and why there is no way around it
//!
//! A `wss://` endpoint is verified against the **platform's trust roots**, and
//! there is no flag, no environment variable and no build feature that relaxes
//! that. This is not an omission to be filled in later. An escape hatch added
//! "just for local testing" is the one that ships, and the end-to-end suite has
//! a legitimate path that needs no hatch at all: it connects to the browser's
//! own loopback listener **without** transport security, rather than with
//! transport security disabled. Those are different things, and only the second
//! one is a hole.
//!
//! Mechanically the property falls out of the connector this module does not
//! configure: `tokio_tungstenite::connect_async` with no explicit connector
//! builds a `rustls` client from `rustls-native-certs` and verifies. There is
//! no argument to pass it, which is the strongest form the rule can take —
//! turning verification off would mean writing new code, not flipping a value.

use std::path::PathBuf;

use futures_util::{SinkExt as _, StreamExt as _};
use talaria_protocol::wire::{
    Channel, ClientView, FrameHeader, InputMessage, ServerView, TabList, FRAME_HEADER_LEN,
    PROTOCOL_VERSION,
};
use talaria_protocol::TabInfo;
use tokio_tungstenite::tungstenite;

/// The environment variable the bearer credential is read from.
pub const TOKEN_ENV: &str = "TALARIA_CLIENT_TOKEN";

/// The credential file's name inside the client's configuration directory.
const TOKEN_FILE: &str = "client-token";

/// The permission bits a credential file must not exceed.
///
/// `0o600` — readable and writable by its owner and by nobody else. The check
/// below is on the group and other bits rather than on equality, so a stricter
/// `0o400` is accepted and a `0o644` is not: this is a floor on secrecy, not a
/// demand for one exact spelling.
const OWNER_ONLY: u32 = 0o600;

/// The path segment the view endpoint is derived from the base URL by adding.
const VIEW_SEGMENT: &str = "view";

/// Where a connection is, and the whole of what the interface knows about the
/// network.
///
/// **Nine variants rather than a boolean or a string, and the separation is the
/// point: the human has a different next step for each one.** Collapsing them
/// would turn "your credential is missing, and here is where it is read from"
/// into "something went wrong", which is the message that ends with a person
/// restarting the wrong machine.
///
/// Each variant carries only what its copy needs. None of them carries the
/// server address, because the chrome already holds the address the human
/// typed and a second copy of one fact is a second thing that can disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Before the connection thread has run. The window exists; nothing has
    /// been attempted.
    NotStarted,
    /// The upgrade is in flight.
    Connecting,
    /// The upgrade completed and the server's hello named a wire this client
    /// implements.
    Connected,
    /// No credential was found in either place one is looked for. This is the
    /// first-run state as well as the lost-my-token state, and the chrome shows
    /// the pairing copy for it.
    MissingCredential,
    /// The client never got as far as the server: nothing answered, the route
    /// is gone, the port is closed.
    Unreachable,
    /// Something answered, but its certificate did not verify against the
    /// platform's trust roots — so the credential was **not** sent. There is no
    /// configuration that makes this succeed anyway; see the module header.
    Untrusted,
    /// The server answered and declined the upgrade.
    ///
    /// **Carries no reason, and does not guess at one.** The server's refusal
    /// is byte-identical for an absent credential, an unknown token, a revoked
    /// client, a token minted for another resource and a token missing the
    /// scope floor — deliberately, so the endpoint is not an oracle for which
    /// tokens exist. Inferring a reason here would be inventing one.
    Refused,
    /// The server's hello named a wire version this client does not implement.
    /// Both numbers are carried because the copy names both: a human comparing
    /// them can tell which end is behind.
    VersionMismatch { server: u32, client: u32 },
    /// The connection was established and then ended.
    Dropped,
}

/// A frame's encoded bytes.
///
/// A newtype over the bytes for one reason: its [`std::fmt::Debug`] prints the
/// length rather than the contents. [`Update`] derives `Debug` and rides a winit
/// user event, so without this a single stray trace line would print three
/// megabytes of a page's pixels — which is both useless and a copy of somebody's
/// screen in a log file.
#[derive(Clone)]
pub struct FramePayload(pub Vec<u8>);

impl std::fmt::Debug for FramePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "FramePayload({} bytes)", self.0.len())
    }
}

/// Everything the connection thread tells the event loop.
#[derive(Debug, Clone)]
pub enum Update {
    /// The connection moved to a new state.
    State(ConnectionState),
    /// A tab snapshot, **in the order the server sent it**. Nothing between the
    /// socket and the chrome reorders this.
    Tabs(Vec<TabInfo>),
    /// The server accepted an attach and named the tab's viewport at that
    /// moment. A *starting* size and never a contract — the frame header stays
    /// the authority for what any particular frame actually is.
    Attached { tab: u64, width: u32, height: u32 },
    /// The attachment on `tab` is over, whether this client asked or the server
    /// decided — a tab closing underneath a viewer takes this exit too.
    Detached { tab: u64 },
    /// The server refused, and **says nothing about why**. Nothing here guesses:
    /// its refusal is byte-identical for a tab the human owns and a tab that
    /// never existed, deliberately.
    Refused,
    /// One frame, header and payload, exactly as it arrived.
    Frame(FrameHeader, FramePayload),
}

/// The messages this client may send, and the only way to send one.
///
/// Cloneable, because the loop hands one to the interface's intent handling and
/// one to the input path, and both are on the same thread as the loop. A send
/// is a push into an unbounded channel the connection thread drains — never a
/// socket write from the loop, because a socket write can block and the loop
/// draws the window.
///
/// Every method answers whether the message was **queued**, not whether it was
/// delivered. There is no delivery answer to give: the wire has no
/// acknowledgement for input, by design.
#[derive(Clone)]
pub struct Outbound {
    sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl Outbound {
    /// One control-channel message: attach, detach, viewport or cadence.
    pub fn control(&self, message: &ClientView) -> bool {
        let Ok(payload) = serde_json::to_vec(message) else { return false };
        self.channel(Channel::Control, &payload)
    }

    /// One input-channel message.
    ///
    /// **The wire's own well-formedness gate, reused rather than restated.**
    /// [`InputMessage::to_json`] refuses a coordinate that is not a finite
    /// number and a key naming both a character and a name or neither — which
    /// are exactly the two shapes the far end refuses. A second spelling of that
    /// rule here would be a second thing to keep in step.
    pub fn input(&self, message: &InputMessage) -> bool {
        let Some(payload) = message.to_json() else { return false };
        self.channel(Channel::Input, payload.as_bytes())
    }

    /// The tag byte, then the payload — the whole of this wire's framing.
    fn channel(&self, channel: Channel, payload: &[u8]) -> bool {
        let mut frame = Vec::with_capacity(payload.len() + 1);
        frame.push(channel.tag());
        frame.extend_from_slice(payload);
        self.sender.send(frame).is_ok()
    }
}

/// Which of the two places a credential was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// [`TOKEN_ENV`].
    Environment,
    /// The owner-only file at this path.
    File(PathBuf),
}

/// One decoded inbound message, in the three shapes this client acts on.
///
/// The event channel is decoded far enough to know it is well formed and then
/// discarded: surfacing events is `05-10`. Discarding is not the same as
/// refusing — see [`decode`].
#[derive(Debug, Clone)]
enum Decoded {
    /// A control-channel message from the server.
    Control(ServerView),
    /// The server's agent tabs, in the server's order.
    Tabs(Vec<TabInfo>),
    /// One frame: its fixed header, and the encoded bytes behind it.
    Frame(FrameHeader, FramePayload),
}

/// The `ws`/`wss` endpoint a base URL names, or `None` for one this client
/// will not use.
///
/// **Derived, never passed alongside the base URL**, so the address a human
/// typed and the address the socket opens cannot disagree — the same
/// single-source discipline the server applies to its own advertised identity.
///
/// Two schemes, and the asymmetry between them is deliberate:
///
/// - `https` becomes `wss`, always.
/// - `http` becomes `ws` **only for a loopback host**. That is the same
///   exception the specification makes for loopback and the same one the
///   end-to-end suite runs through; an `http` base naming any other host is
///   refused here rather than silently downgrading a real connection to
///   plaintext, which would carry a bearer token in the clear.
///
/// The path is appended rather than replaced, so a server proxied at a sub-path
/// works without a second argument.
pub fn view_endpoint(base: &str) -> Option<String> {
    let parsed = url::Url::parse(base).ok()?;
    let host = parsed.host_str()?;
    let scheme = match parsed.scheme() {
        "https" => "wss",
        "http" if is_loopback(host) => "ws",
        _ => return None,
    };
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let prefix = parsed.path().trim_end_matches('/');
    Some(format!("{scheme}://{authority}{prefix}/{VIEW_SEGMENT}"))
}

/// Whether a host string names this machine.
///
/// Literals only. A *name* that happens to resolve to a loopback address is not
/// accepted, because what it resolves to is not something this client decides
/// and a plaintext credential is not something to gamble on a resolver.
fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.parse::<std::net::IpAddr>().map(|address| address.is_loopback()).unwrap_or(false)
}

/// Where the credential file lives, or `None` on a machine with no
/// configuration directory.
pub fn credential_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("talaria").join(TOKEN_FILE))
}

/// The credential, and which of the two places it came from.
///
/// **The value is returned and never logged.** No caller of this function
/// writes it anywhere but into an `Authorization` header, and that header value
/// is marked sensitive at the point it is built.
fn read_credential() -> Option<(String, CredentialSource)> {
    let from_environment = std::env::var(TOKEN_ENV).ok();
    let path = credential_file();
    let from_file = path.as_deref().and_then(read_owner_only);
    choose_credential(from_environment, from_file, path)
}

/// The precedence rule, split out so it is testable without an environment.
///
/// The variable wins, because it is the one a human can set for one launch
/// without editing a file they then forget about. A blank or whitespace-only
/// value counts as absent in both places: an exported-but-empty variable is a
/// shell accident, not a credential, and treating it as one would produce a
/// refusal where the honest answer is "nothing is configured".
fn choose_credential(
    from_environment: Option<String>,
    from_file: Option<String>,
    path: Option<PathBuf>,
) -> Option<(String, CredentialSource)> {
    let usable = |value: Option<String>| {
        value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
    };
    if let Some(value) = usable(from_environment) {
        return Some((value, CredentialSource::Environment));
    }
    let value = usable(from_file)?;
    Some((value, CredentialSource::File(path?)))
}

/// The contents of `path`, but only if nobody except its owner can read it.
///
/// A credential file the whole machine can read is the problem this module
/// avoids by refusing the command line in the first place, so it is refused
/// here for the same reason rather than being read with a warning. The refusal
/// is logged by path — never by content.
#[cfg(unix)]
fn read_owner_only(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt as _;
    let metadata = std::fs::metadata(path).ok()?;
    let mode = metadata.permissions().mode();
    if mode & !OWNER_ONLY & 0o777 != 0 {
        log::warn!(
            "ignoring the credential file at {}: mode {:o} lets more than its owner read it; \
             chmod 600 it",
            path.display(),
            mode & 0o777,
        );
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The non-Unix reader: no mode to check, so there is none to enforce.
///
/// Stated rather than silently skipped — a reader on a platform where this
/// compiles should know the file is protected by whatever that platform's own
/// per-user directory protection is and by nothing this module does.
#[cfg(not(unix))]
fn read_owner_only(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Split an inbound frame into its channel and act on the two channels this
/// plan carries.
///
/// **Unrecognised is discarded, never fatal.** A tag byte naming no channel is
/// a message from a newer server, and dropping a working connection over one
/// would make every future addition to the wire a breaking change; a
/// frame-channel message shorter than its own fixed header is a message that
/// cannot be trusted and goes the same way. Neither disturbs the connection.
///
/// Structural refusal beyond this belongs to the wire module, which owns it for
/// both ends. This is the reading loop that hands it bytes.
fn decode(frame: &[u8]) -> Option<Decoded> {
    let (tag, payload) = frame.split_first()?;
    match Channel::from_tag(*tag)? {
        Channel::Control => serde_json::from_slice(payload).ok().map(Decoded::Control),
        Channel::Tabs => {
            serde_json::from_slice::<TabList>(payload).ok().map(|list| Decoded::Tabs(list.tabs))
        },
        // The header is decoded through the wire type, which makes six
        // structural refusals of its own — a short slice, an unknown format
        // version, a kind naming no kind, a zero scale denominator, a tile with
        // no area, and a tile that does not fit inside the frame it declares.
        // The last matters most here: a header claiming a tile beyond its own
        // frame is a peer asking this client to composite outside its texture.
        Channel::Frame => {
            let header = frame_header(payload)?;
            let bytes = payload.get(FRAME_HEADER_LEN..)?;
            Some(Decoded::Frame(header, FramePayload(bytes.to_vec())))
        },
        // Events are `05-10`'s. Input only ever travels the other way.
        Channel::Event | Channel::Input => None,
    }
}

/// The fixed-width header at the front of a frame-channel payload.
///
/// Split out from [`decode`] so the two halves of "a short header is discarded"
/// — too few bytes to be a header, and enough bytes that are not one — are each
/// assertable, and so `05-09` has the seam it grows the presenter from.
fn frame_header(payload: &[u8]) -> Option<FrameHeader> {
    FrameHeader::from_bytes(payload.get(..FRAME_HEADER_LEN)?)
}

/// The state a failed connection attempt is in.
///
/// Three answers, and the split matters because the next steps differ: check
/// the address, check the certificate, check the credential.
fn classify(error: &tungstenite::Error) -> ConnectionState {
    match error {
        // `rustls` reaches this crate as an `io::Error` carrying the real
        // cause, so the cause is what is read rather than the wrapper's kind.
        tungstenite::Error::Io(io) => match io
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            Some(rustls::Error::InvalidCertificate(_)) => ConnectionState::Untrusted,
            _ => ConnectionState::Unreachable,
        },
        // A name the certificate could never be checked against is the same
        // answer as a certificate that failed the check: the identity of the
        // far end was not established.
        tungstenite::Error::Tls(_) => ConnectionState::Untrusted,
        // The server answered and said no, in any of the shapes that means.
        // No reason is inferred; see [`ConnectionState::Refused`].
        _ => ConnectionState::Refused,
    }
}

/// Build the upgrade request: the derived endpoint, plus the bearer credential.
///
/// The header value is marked sensitive, which is what asks every layer that
/// might otherwise print a header map to print a placeholder instead. It is
/// belt-and-braces next to the rule that nothing here logs the value at all.
fn view_request(
    endpoint: &str,
    credential: &str,
) -> Option<tungstenite::handshake::client::Request> {
    use tungstenite::client::IntoClientRequest as _;
    let mut request = endpoint.into_client_request().ok()?;
    let mut value =
        tungstenite::http::HeaderValue::from_str(&format!("Bearer {credential}")).ok()?;
    value.set_sensitive(true);
    request.headers_mut().insert(tungstenite::http::header::AUTHORIZATION, value);
    Some(request)
}

/// Spawn the connection thread, and answer with the way to send on it.
///
/// Called from the entry point with a cloned loop proxy, **before** the loop
/// runs, so the first state the window can draw is already on its way.
///
/// The returned [`Outbound`] is usable immediately, before the socket has
/// opened: a message queued now is written when the connection is up. Nothing
/// depends on that — the loop refuses to send anything at all while the
/// connection is not established — but a queue that only existed after the fact
/// would be a second state to reason about for no gain.
pub fn spawn(endpoint: String, report: impl Fn(Update) + Send + 'static) -> Outbound {
    let (sender, commands) = tokio::sync::mpsc::unbounded_channel();
    let thread = std::thread::Builder::new().name("talaria-view".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                // Degrade, never abort: a client that cannot start a runtime is
                // still a window, and from the human's side the server is
                // exactly as unreachable as if the route were gone.
                log::error!("the view connection runtime could not be built: {error}");
                report(Update::State(ConnectionState::Unreachable));
                return;
            },
        };
        runtime.block_on(connect(endpoint, &report, commands));
    });
    if let Err(error) = thread {
        log::error!("the view connection thread could not be spawned: {error}");
    }
    // Handed back whether or not the thread started. A send on a channel with
    // no receiver answers `false`, which is the same answer a send on a closed
    // connection gives — one failure shape rather than two.
    Outbound { sender }
}

/// One connection, from credential to close.
///
/// **No retry loop.** A connection that ended puts a named state on the screen
/// and stops there, because every one of those states has a next step a human
/// takes — and a client that silently reconnected forever would hide all of
/// them behind a spinner.
async fn connect(
    endpoint: String,
    report: &(impl Fn(Update) + ?Sized),
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let Some((credential, source)) = read_credential() else {
        report(Update::State(ConnectionState::MissingCredential));
        return;
    };
    // The source, never the value.
    match &source {
        CredentialSource::Environment => log::info!("using the credential from {TOKEN_ENV}"),
        CredentialSource::File(path) => {
            log::info!("using the credential from {}", path.display())
        },
    }
    report(Update::State(ConnectionState::Connecting));

    let Some(request) = view_request(&endpoint, &credential) else {
        // The endpoint was already derived and parsed before the window opened,
        // so reaching here means bytes a header cannot carry. Nothing was sent,
        // so the far end is exactly as unreached as an address that answered
        // nothing.
        log::error!("the view endpoint could not be turned into a request");
        report(Update::State(ConnectionState::Unreachable));
        return;
    };
    drop(credential);

    log::info!("connecting to {endpoint}");
    let socket = match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _response)) => socket,
        Err(error) => {
            let state = classify(&error);
            log::warn!("the view connection did not open: {state:?}");
            report(Update::State(state));
            return;
        },
    };
    // Split so the reading half and the writing half can be awaited at once.
    // Without this the loop below could not do both: reading borrows the socket
    // for as long as it waits, and a viewer that could not send while waiting
    // for a frame could not send at all — a page that is not changing sends
    // nothing, which is exactly when a human is about to click on it.
    let (mut writer, mut reader) = socket.split();

    // The server sends its hello first on every connection, before anything
    // else, so a client always learns the wire version before it is handed a
    // message it would need that version to interpret.
    match next_binary(&mut reader).await.as_deref().and_then(decode) {
        Some(Decoded::Control(ServerView::Hello { protocol })) if protocol == PROTOCOL_VERSION => {
            report(Update::State(ConnectionState::Connected));
        },
        Some(Decoded::Control(ServerView::Hello { protocol })) => {
            report(Update::State(ConnectionState::VersionMismatch {
                server: protocol,
                client: PROTOCOL_VERSION,
            }));
            return;
        },
        // Anything else first — including nothing at all — is not a wire this
        // client can read. It opened and then ended, which is what `Dropped`
        // says.
        _ => {
            report(Update::State(ConnectionState::Dropped));
            return;
        },
    }

    // Both directions at once, on one thread, for the life of the connection.
    //
    // `quiet` guards the send arm rather than breaking the loop when the
    // command channel closes: every [`Outbound`] being dropped means the window
    // is gone, but the reading half may still have messages worth draining, and
    // an unguarded `recv` that answers `None` forever would spin this thread at
    // the speed of the scheduler.
    let mut quiet = false;
    loop {
        tokio::select! {
            inbound = next_binary(&mut reader) => {
                let Some(frame) = inbound else { break };
                match decode(&frame) {
                    Some(Decoded::Tabs(tabs)) => report(Update::Tabs(tabs)),
                    Some(Decoded::Frame(header, payload)) => {
                        report(Update::Frame(header, payload))
                    },
                    Some(Decoded::Control(ServerView::Attached { tab, width, height })) => {
                        report(Update::Attached { tab, width, height })
                    },
                    Some(Decoded::Control(ServerView::Detached { tab })) => {
                        report(Update::Detached { tab })
                    },
                    Some(Decoded::Control(ServerView::Refused)) => report(Update::Refused),
                    // A second hello, and anything this build does not know:
                    // discarded, for [`decode`]'s reason.
                    Some(Decoded::Control(ServerView::Hello { .. })) | None => {},
                }
            },
            outgoing = commands.recv(), if !quiet => {
                let Some(frame) = outgoing else {
                    quiet = true;
                    continue;
                };
                if writer.send(tungstenite::Message::Binary(frame.into())).await.is_err() {
                    break;
                }
            },
        }
    }
    report(Update::State(ConnectionState::Dropped));
}

/// The next binary message, or `None` once the socket is over.
///
/// Text is discarded rather than fatal: nothing on this wire is text, so a text
/// message is a server saying something in a language this client does not
/// speak, and that is the same situation as an unknown channel tag. Ping and
/// pong are answered by the socket itself.
async fn next_binary<S>(reader: &mut S) -> Option<Vec<u8>>
where
    S: futures_util::Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    loop {
        match reader.next().await? {
            Ok(tungstenite::Message::Binary(frame)) => return Some(frame.to_vec()),
            Ok(tungstenite::Message::Close(_)) => return None,
            Ok(_) => {},
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(tab_id: u64, title: &str) -> TabInfo {
        TabInfo {
            tab_id,
            url: format!("https://example.com/{tab_id}"),
            title: title.to_owned(),
            owner: "agent-one".to_owned(),
            focused: false,
            crashed: false,
            loading: false,
        }
    }

    fn framed(channel: Channel, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![channel.tag()];
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn a_secure_base_url_yields_a_secure_socket() {
        assert_eq!(
            view_endpoint("https://thinkpad.example.ts.net"),
            Some("wss://thinkpad.example.ts.net/view".to_owned()),
        );
    }

    #[test]
    fn a_port_and_a_sub_path_both_survive_the_derivation() {
        assert_eq!(
            view_endpoint("https://thinkpad.example.ts.net:8443/talaria/"),
            Some("wss://thinkpad.example.ts.net:8443/talaria/view".to_owned()),
        );
    }

    #[test]
    fn the_loopback_insecure_form_yields_a_plain_socket() {
        assert_eq!(
            view_endpoint("http://127.0.0.1:41234"),
            Some("ws://127.0.0.1:41234/view".to_owned()),
        );
        assert_eq!(view_endpoint("http://localhost:9"), Some("ws://localhost:9/view".to_owned()));
    }

    #[test]
    fn an_insecure_base_url_to_anywhere_else_is_refused_rather_than_downgraded() {
        assert_eq!(view_endpoint("http://thinkpad.example.ts.net"), None);
        assert_eq!(view_endpoint("ftp://example.com"), None);
        assert_eq!(view_endpoint("not a url"), None);
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let chosen = choose_credential(
            Some("  from-the-environment  ".to_owned()),
            Some("from-the-file".to_owned()),
            Some(PathBuf::from("/tmp/client-token")),
        );
        assert_eq!(
            chosen,
            Some(("from-the-environment".to_owned(), CredentialSource::Environment)),
        );
    }

    #[test]
    fn a_blank_variable_is_absent_rather_than_a_credential() {
        let chosen = choose_credential(
            Some("   ".to_owned()),
            Some("from-the-file\n".to_owned()),
            Some(PathBuf::from("/tmp/client-token")),
        );
        assert_eq!(
            chosen,
            Some((
                "from-the-file".to_owned(),
                CredentialSource::File(PathBuf::from("/tmp/client-token")),
            )),
        );
    }

    #[test]
    fn neither_place_holding_one_is_the_missing_credential_case() {
        assert_eq!(choose_credential(None, None, Some(PathBuf::from("/tmp/x"))), None);
        assert_eq!(choose_credential(None, Some(String::new()), None), None);
    }

    #[test]
    fn an_unknown_channel_tag_is_discarded_and_nothing_else() {
        assert!(decode(&[0x7f, 1, 2, 3]).is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn a_frame_shorter_than_its_own_header_is_discarded() {
        let short = vec![0u8; FRAME_HEADER_LEN - 1];
        assert!(frame_header(&short).is_none());
        assert!(decode(&framed(Channel::Frame, &short)).is_none());
    }

    #[test]
    fn a_header_sized_run_of_bytes_that_is_not_a_header_is_discarded_too() {
        let bogus = vec![0u8; FRAME_HEADER_LEN];
        assert!(frame_header(&bogus).is_none());
        assert!(decode(&framed(Channel::Frame, &bogus)).is_none());
    }

    #[test]
    fn a_well_formed_frame_decodes_into_its_header_and_the_bytes_behind_it() {
        use talaria_protocol::wire::FrameKind;
        let header = FrameHeader {
            kind: FrameKind::Tile,
            scale_denominator: 1,
            tab_id: 12,
            frame_seq: 44,
            last_applied_input: 43,
            tile_x: 64,
            tile_y: 0,
            tile_width: 64,
            tile_height: 64,
            frame_width: 256,
            frame_height: 128,
        };
        let mut payload = header.to_bytes().to_vec();
        payload.extend_from_slice(b"\x89PNG\r\n\x1a\n and then some");
        let Some(Decoded::Frame(decoded, bytes)) = decode(&framed(Channel::Frame, &payload)) else {
            panic!("a frame carrying a well-formed header must decode");
        };
        assert_eq!(decoded, header, "the header did not survive the round trip");
        assert_eq!(bytes.0, b"\x89PNG\r\n\x1a\n and then some");
    }

    #[test]
    fn a_frame_payload_prints_its_length_rather_than_a_page_of_pixels() {
        assert_eq!(format!("{:?}", FramePayload(vec![0u8; 3_840_000])),
                   "FramePayload(3840000 bytes)");
    }

    #[test]
    fn a_tab_snapshot_decodes_in_the_order_the_server_sent_it() {
        let tabs = vec![tab(9, "Nine"), tab(2, "Two"), tab(5, "Five")];
        let Ok(payload) = serde_json::to_vec(&TabList { tabs: tabs.clone() }) else {
            panic!("a TabList of plain fields must serialise");
        };
        let Some(Decoded::Tabs(decoded)) = decode(&framed(Channel::Tabs, &payload)) else {
            panic!("a well-formed tabs frame must decode");
        };
        assert_eq!(
            decoded.iter().map(|tab| tab.tab_id).collect::<Vec<_>>(),
            vec![9, 2, 5],
            "the client must not reorder what the server sent",
        );
    }

    #[test]
    fn a_hello_naming_this_wire_decodes_as_a_hello() {
        let Ok(payload) = serde_json::to_vec(&ServerView::Hello { protocol: PROTOCOL_VERSION })
        else {
            panic!("a ServerView of plain fields must serialise");
        };
        assert!(matches!(
            decode(&framed(Channel::Control, &payload)),
            Some(Decoded::Control(ServerView::Hello { protocol })) if protocol == PROTOCOL_VERSION,
        ));
    }

    #[test]
    fn a_certificate_that_did_not_verify_is_its_own_state() {
        let inner = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        let error = tungstenite::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            inner,
        ));
        assert_eq!(classify(&error), ConnectionState::Untrusted);
    }

    #[test]
    fn a_server_that_never_answered_is_unreachable_rather_than_untrusted() {
        let error =
            tungstenite::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        assert_eq!(classify(&error), ConnectionState::Unreachable);
    }

    #[test]
    fn a_server_that_answered_and_declined_is_a_refusal_with_no_reason_attached() {
        let Ok(response) = tungstenite::http::Response::builder()
            .status(tungstenite::http::StatusCode::UNAUTHORIZED)
            .body(None)
        else {
            panic!("a response builder with one status and no body cannot fail");
        };
        assert_eq!(classify(&tungstenite::Error::Http(Box::new(response))), ConnectionState::Refused);
    }

    /// A real peer, stood up in the test rather than borrowed from the shell.
    ///
    /// Accepts one plain WebSocket, sends the frames it was given, and closes.
    /// Returns the loopback base URL to point the client at.
    async fn peer(frames: Vec<Vec<u8>>) -> Option<(String, tokio::task::JoinHandle<()>)> {
        use futures_util::SinkExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
        let port = listener.local_addr().ok()?.port();
        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else { return };
            let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return };
            for frame in frames {
                if socket.send(tungstenite::Message::Binary(frame.into())).await.is_err() {
                    return;
                }
            }
            let _ = socket.close(None).await;
        });
        Some((format!("http://127.0.0.1:{port}"), handle))
    }

    /// Drive `connect` against a peer and collect every update it reported.
    async fn updates_against(frames: Vec<Vec<u8>>) -> Vec<Update> {
        std::env::set_var(TOKEN_ENV, "a-test-credential");
        let Some((base, handle)) = peer(frames).await else {
            panic!("a loopback listener on port 0 must bind");
        };
        let Some(endpoint) = view_endpoint(&base) else {
            panic!("a loopback base URL must derive an endpoint");
        };
        let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = collected.clone();
        // A receiver whose sender is dropped at once: this client sends nothing
        // in these cases, and the send arm must sit quiet rather than spin.
        let (_sender, commands) = tokio::sync::mpsc::unbounded_channel();
        connect(
            endpoint,
            &move |update: Update| {
                if let Ok(mut collected) = sink.lock() {
                    collected.push(update);
                }
            },
            commands,
        )
        .await;
        handle.abort();
        let Ok(collected) = collected.lock() else {
            panic!("the collector lock cannot be poisoned by a closure that cannot panic");
        };
        collected.clone()
    }

    #[tokio::test]
    async fn a_hello_naming_another_wire_is_a_version_mismatch_rather_than_a_misparse() {
        let Ok(payload) = serde_json::to_vec(&ServerView::Hello { protocol: 999 }) else {
            panic!("a ServerView of plain fields must serialise");
        };
        let updates = updates_against(vec![framed(Channel::Control, &payload)]).await;
        let states: Vec<_> = updates
            .iter()
            .filter_map(|update| match update {
                Update::State(state) => Some(state.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            states,
            vec![
                ConnectionState::Connecting,
                ConnectionState::VersionMismatch { server: 999, client: PROTOCOL_VERSION },
            ],
        );
    }

    #[tokio::test]
    async fn a_hello_then_a_snapshot_connects_and_delivers_the_tabs_in_order() {
        let Ok(hello) = serde_json::to_vec(&ServerView::Hello { protocol: PROTOCOL_VERSION })
        else {
            panic!("a ServerView of plain fields must serialise");
        };
        let tabs = vec![tab(4, "Four"), tab(1, "One")];
        let Ok(list) = serde_json::to_vec(&TabList { tabs }) else {
            panic!("a TabList of plain fields must serialise");
        };
        let updates = updates_against(vec![
            framed(Channel::Control, &hello),
            // An unknown tag between two good messages: the connection must
            // carry on through it rather than end.
            framed(Channel::Tabs, &list),
            vec![0x7f, 0, 0],
        ])
        .await;
        let mut seen_tabs = Vec::new();
        let mut states = Vec::new();
        for update in updates {
            match update {
                Update::State(state) => states.push(state),
                Update::Tabs(tabs) => {
                    seen_tabs = tabs.iter().map(|tab| tab.tab_id).collect::<Vec<_>>()
                },
                _ => {},
            }
        }
        assert_eq!(
            states,
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                ConnectionState::Dropped,
            ],
        );
        assert_eq!(seen_tabs, vec![4, 1]);
    }
}
