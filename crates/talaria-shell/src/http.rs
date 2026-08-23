//! The remote MCP transport: a Streamable-HTTP listener, off by default,
//! bound to loopback and nothing else.
//!
//! **Read the security posture before the mechanics, because that is the
//! order a later reader needs them in.** This module opens a TCP listener in
//! the process that holds the credential vault and the human's logged-in
//! browsing session. Everything Phase 2 relied on to make the control socket
//! safe is *lost* on TCP:
//!
//! - A Unix domain socket carries kernel-vouched peer credentials, which is
//!   what [`crate::control`]'s `peer_uid_ok` checks. **There is no TCP
//!   equivalent.** `SO_PEERCRED` does not exist for an AF_INET socket.
//! - That socket also lives at `0600` inside a `0700` directory, so the
//!   filesystem alone keeps other accounts out. `127.0.0.1:PORT` has no such
//!   property: **every local account on this machine can reach it.**
//!
//! So the bearer token is the entire boundary, and D-04-04's two constraints
//! are load-bearing rather than stylistic: the listener is **off by default**
//! (no configuration means no thread and no bound port at all — an absence,
//! not a flag consulted per request), and when on it binds [`BIND_HOST`] and
//! nothing else. OAuth 2.1 requires the secure scheme for authorization-server
//! endpoints *with a loopback exception*, which is what made shipping Phase 4
//! without transport security conformant rather than merely unfinished.
//!
//! **Phase 5 kept the bind exactly where it is and put transport security in
//! front of it.** D-05-03: a reverse proxy the operating system's own daemon
//! runs terminates it and forwards to loopback, which is the one proxy target
//! that daemon supports and exactly what this listener already binds. So the
//! loopback-only property is now *permanent* rather than provisional, this
//! process owns nothing about renewal, and what changes instead is that the
//! origin a client reaches this browser at stops being the address this
//! listener bound. That second fact is [`crate::oauth::AdvertisedIdentity`],
//! it comes from configuration and never from a request header, and it is what
//! the host allowlist and [`refuse_page_originated`] admit alongside the bound
//! address.
//!
//! **The token is now real.** 04-03 shipped this listener behind an interim
//! provider that refused every credential unconditionally, so the transport
//! could exist before its authentication did. That provider is gone;
//! [`crate::oauth::TalariaAuth`] stands in its place and a request can, for
//! the first time, succeed. Everything that decides whether one does lives in
//! that module: a live per-request lookup against the shared agent store, RFC
//! 8707 audience binding, and one indistinguishable refusal for every failure.
//!
//! Two things about that swap are worth saying here rather than there. The
//! [`RefuseOriginHeader`] middleware still runs **before** verification, so a
//! page-originated request carrying a stolen token is turned away on its
//! origin and never reaches the token check at all. And the store the provider
//! reads is the *same* store the browser chrome mutates — one
//! [`crate::oauth::SharedAgents`] constructed once at startup — so a
//! revocation performed in the window lands on the listener's very next
//! request rather than at the next restart.
//!
//! ## Why this builds the router rather than calling `create_axum_server`
//!
//! `rust-mcp-axum` offers two mount paths, and both are supported: the
//! all-in-one `create_axum_server` / `AxumServerOptions`, and the documented
//! BYO-server path (`mcp_routes` + `McpMountOptions`, its `byo-server`
//! example). This module takes the second, for three reasons that the first
//! cannot give:
//!
//! 1. **The `Origin` refusal has nowhere else to live.** `AxumServerOptions`
//!    exposes no middleware slot, and the SDK's own origin control is an
//!    *allowlist* — it demands the header be present and matching, which is
//!    the opposite of what a command-line MCP client does. See
//!    [`RefuseOriginHeader`].
//! 2. **The bound address is then a fact rather than a guess.** This module
//!    binds its own [`tokio::net::TcpListener`] and reports
//!    `local_addr()`, so every surface renders what was actually bound.
//! 3. **The process keeps its own signal handling.** `AxumServer::start_http`
//!    spawns a task that installs Ctrl+C and `SIGTERM` handlers of its own; a
//!    browser that stopped responding to `SIGTERM` because remote access was
//!    switched on would be a regression nobody asked for.
//!
//! Nothing else diverges: the [`McpAppState`] built here mirrors
//! `AxumServer::new`'s field for field, and the middleware chain is the same
//! one it composes, with ours added in front.

use std::collections::HashMap;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// The direct `axum` dependency rather than the `rust_mcp_axum` re-export, for
// the WebSocket extractor alone: `ws` is a feature this workspace enables on
// its own line and the re-export does not carry it. The same crate either way
// — cargo unifies them — so `axum::extract::State` here and
// `rust_mcp_axum::axum::extract::State` elsewhere are one type.
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use rust_mcp_axum::axum::extract::connect_info::Connected;
use rust_mcp_axum::axum::serve::IncomingStream;
use rust_mcp_axum::mcp_routes;
use rust_mcp_sdk::auth::{
    AuthProvider, OAUTH_PROTECTED_RESOURCE_BASE, WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER,
};
use rust_mcp_sdk::id_generator::{FastIdGenerator, UuidGenerator};
use rust_mcp_sdk::mcp_http::http::{
    header, HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Uri,
};
use rust_mcp_sdk::mcp_http::middleware::{AuthMiddleware, DnsRebindProtector};
use rust_mcp_sdk::mcp_http::{
    error_response, GenericBody, McpAppState, McpHttpHandler, McpHttpResult, McpMountOptions,
    Middleware, MiddlewareNext,
};
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::schema_utils::{CallToolError, SdkError};
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult, ListToolsResult,
    PaginatedRequestParams, RpcError, ServerCapabilities, ServerCapabilitiesTools,
};
use rust_mcp_sdk::session_store::InMemorySessionStore;
use rust_mcp_sdk::{McpServer, ToMcpServerHandler, MCP_SESSION_ID_HEADER};
use talaria_mcp::{dispatch, CommandSink, TalariaTools};
use talaria_protocol::{Command, Outcome};
use tokio::sync::oneshot;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::control::{command_timeout_secs, next_session_id, AgentRequest};
use crate::oauth::{AdvertisedIdentity, ConsentRaiser, SharedAgents, TalariaAuth};

/// The only address this listener ever binds.
///
/// A module constant used at exactly one place, deliberately: "bind somewhere
/// else, just for a test" is then a change someone has to make on purpose and
/// a reviewer can see in a diff, rather than a value that can arrive from a
/// configuration file. [`crate::settings::RemoteAccessConfig`] has no
/// bind-address field for the same reason — D-04-04 is enforced by the shape
/// of the program, not by a validator that could be relaxed.
const BIND_HOST: &str = "127.0.0.1";

/// How long a graceful shutdown waits for in-flight requests before the
/// runtime goes away underneath them.
///
/// Shorter than the command timeout on purpose: a request still running at
/// this point is one whose caller is going to be told the browser stopped
/// answering, and making the human wait the full command timeout for a switch
/// they just flicked off would read as a hang.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// The tab-owner label used when a request somehow arrives with no verified
/// identity attached.
///
/// Unreachable in practice — `AuthMiddleware` runs ahead of every handler on
/// this transport, so a tool call without an `AuthInfo` should not exist. It
/// is a label that claims nothing rather than a fallback to the name the peer
/// asked to be called: a check whose failure path silently accepts
/// attacker-chosen input is a check that has been made optional.
const UNVERIFIED_CLIENT: &str = "unverified-client";

/// What the listener is doing, as the rest of the shell sees it.
///
/// The single source of truth for the Access panel's status block, the
/// toolbar's connection glyph, and `parse_agent_url`'s own-origin refusal, so
/// those three cannot disagree about whether anything is listening or on what
/// address. Written only from the listener thread's own events — never from
/// the configured value, which is what someone *asked* for rather than what
/// happened.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RemoteAccess {
    /// Nothing is listening, and no thread exists. The default.
    #[default]
    Off,
    /// The thread has been spawned and the bind has not reported back yet.
    Starting,
    /// Listening, at the address the listener actually bound.
    Bound {
        /// `host:port`, from the socket's own `local_addr()`.
        addr: String,
    },
    /// The bind failed, or the listener stopped without being asked to.
    /// Remote access is off; the UI never claims a listener that is not there.
    Failed {
        /// The address that was attempted.
        addr: String,
        /// What went wrong, in the words the OS or the server used.
        error: String,
    },
}

impl RemoteAccess {
    /// The address actually bound, or `None` when nothing is listening.
    ///
    /// The refusal in `parse_agent_url` is keyed on this rather than on the
    /// configured port, so it invents no rule while the listener is off.
    pub fn bound_addr(&self) -> Option<&str> {
        match self {
            RemoteAccess::Bound { addr } => Some(addr),
            _ => None,
        }
    }

    /// Whether a listener already exists, or is on its way to existing.
    ///
    /// What makes a second click on "turn on" a no-op instead of a second
    /// bind — see `apply_ui_actions`.
    pub fn is_live(&self) -> bool {
        matches!(self, RemoteAccess::Starting | RemoteAccess::Bound { .. })
    }
}

/// A handle that shuts the listener thread down.
///
/// Held by `Shared`; sending on it (or dropping it) ends the serve loop,
/// which ends `block_on`, which ends the thread and drops its runtime.
pub struct ShutdownHandle(oneshot::Sender<()>);

impl ShutdownHandle {
    /// Ask the listener to stop. In-flight requests are resolved rather than
    /// stranded: each awaiting sink holds a `oneshot` receiver, so the runtime
    /// going away closes them and every pending request comes back to its
    /// caller as an error instead of never coming back at all.
    pub fn shutdown(self) {
        let _ = self.0.send(());
    }
}

/// The path a synthetic internal request names, and the only path the SDK's
/// Streamable-HTTP handler serves.
///
/// `pub(crate)` so that [`crate::oauth::canonical_resource`] appends *this*
/// path rather than a second spelling of it: the canonical resource identifier
/// is the MCP endpoint's own URL, and it is compared byte for byte.
pub(crate) const MCP_PATH: &str = "/mcp";

/// The remote view channel's path: a WebSocket upgrade, and the only route on
/// this listener that is not the SDK's.
///
/// **Not in [`PUBLIC_DISCOVERY_PATHS`], and the routing test names it.** It is
/// origin- and host-checked like everything else, which is what closes
/// cross-site WebSocket hijacking here (T-05-01): WebSockets are exempt from
/// the same-origin policy and have no preflight, so origin enforcement is
/// entirely the server's job — and a browser *always* sends `Origin`, so the
/// outermost layer makes a browser-based viewer structurally impossible rather
/// than merely unsupported.
pub(crate) const VIEW_PATH: &str = "/view";

/// The legacy HTTP-plus-event-stream transport's path.
///
/// **Live, whatever `sse_support: false` suggests.** `mcp_routes` mounts it
/// unconditionally in `rust-mcp-axum` 1.0.1, and the SDK's `sse` feature is
/// enabled *transitively* through that crate — `cargo tree -p talaria-shell
/// -e features -i rust-mcp-sdk` shows it, and it has to be, because
/// `sse_routes` calls the `#[cfg(feature = "sse")]` handler unconditionally
/// and the workspace would not otherwise compile. So a client can open a real
/// session and a real event stream here, and T-7 has to reach it: a revoke
/// that closed only `/mcp` streams would leave a revoked client receiving.
///
/// Taken from the SDK's own constant rather than spelled a second time, so
/// this and [`McpMountOptions::default`] cannot disagree about the path.
const SSE_PATH: &str = rust_mcp_sdk::mcp_http::DEFAULT_SSE_ENDPOINT;

/// The only paths this listener answers for a caller it has refused to
/// identify — and the whole of the exception [`refuse_page_originated`] makes.
///
/// **Read this before adding a route.** The default for every path this
/// process serves is *checked*: no `Origin` header, and a `Host` naming the
/// address actually bound. Discovery is the one place that default cannot
/// hold, and the reason is specific rather than general. RFC 9728 and RFC 8414
/// documents are what a client fetches **before** it has any credential, so
/// they cannot demand one; and a client that is itself a page is entitled to
/// read them cross-origin, so they cannot demand same-origin either. A
/// discovery document that refused those callers would make the flow it
/// describes undiscoverable.
///
/// Nothing else on this server has that property. `/register`, `/authorize`,
/// `/token` and `/revoke` mint, spend and destroy credentials for a browser
/// holding the human's logged-in session, and the argument above says nothing
/// whatever about them — 04-05 reasoned it for the metadata document alone and
/// was right to; 04-06/04-07/04-08 inherited the SDK's empty chain without the
/// reasoning being re-run, which is the hole this closes.
///
/// So: an allowlist, not a denylist. A path added to this server and not added
/// here is checked, which is the direction a mistake should fall in.
const PUBLIC_DISCOVERY_PATHS: [&str; 2] =
    [OAUTH_PROTECTED_RESOURCE_BASE, WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER];

/// Whether `path` is one of the two uncredentialed discovery documents.
///
/// An exact match on the whole path, never a prefix. A discovery document is
/// one of two exact strings; a `starts_with` would also admit anything mounted
/// beneath them, which is a rule nobody wrote down and a later route could land
/// under by accident.
fn is_public_discovery(path: &str) -> bool {
    PUBLIC_DISCOVERY_PATHS.contains(&path)
}

/// `shutdown(2)`'s "both directions".
#[cfg(unix)]
const SHUT_RDWR: i32 = 2;

// The three POSIX calls this module needs and that no dependency in this
// workspace exposes. Declared here rather than by adding `libc`, which is the
// same call `talaria_protocol` makes for `getuid` and for the same reason: one
// extern block is a smaller thing to audit than a crate.
#[cfg(unix)]
extern "C" {
    fn dup(fd: i32) -> i32;
    fn shutdown(fd: i32, how: i32) -> i32;
    fn close(fd: i32) -> i32;
}

/// A duplicate of one connection's socket descriptor, closed when dropped.
///
/// **Duplicated rather than remembered as a bare number, and that is the whole
/// point of the type.** A descriptor number is recycled the instant its
/// connection closes, so a remembered number names a *different* thing a
/// moment later — and shutting that down would tear apart whatever took its
/// place, which in this process could be the control socket or one of the
/// engine's own connections. A duplicate keeps the underlying socket alive, so
/// the number this holds names that connection and nothing else for as long as
/// this value exists.
///
/// Unix only. The control plane is a Unix domain socket, so a non-Unix target
/// has never been a supported configuration; the other target compiles with
/// every operation a no-op rather than with a second implementation nobody
/// runs. A browser that cannot close a socket is still a browser, and the half
/// of a revoke that refuses the next request is untouched either way.
struct OwnedSocket(i32);

impl OwnedSocket {
    /// Duplicate `fd`, or `None` if the call failed.
    ///
    /// Only ever called from the middleware handling a request that arrived
    /// **on that connection**, so the descriptor is live at the moment of the
    /// call. That is what makes the duplication safe rather than hopeful.
    #[cfg(unix)]
    fn duplicate(fd: i32) -> Option<Self> {
        // SAFETY: `fd` belongs to this process and is serving the request that
        // reached this code, so it is open for the duration of the call.
        let copy = unsafe { dup(fd) };
        match copy >= 0 {
            true => Some(Self(copy)),
            false => None,
        }
    }

    #[cfg(not(unix))]
    fn duplicate(_fd: i32) -> Option<Self> {
        None
    }

    /// Break the connection in both directions.
    ///
    /// The client's read side sees the response body end, which for an event
    /// stream on a keep-alive connection is the only ending it can be given.
    #[cfg(unix)]
    fn disconnect(&self) {
        // SAFETY: `self.0` is this value's own descriptor, closed only in
        // `Drop`, so it is open for the duration of the call.
        unsafe { shutdown(self.0, SHUT_RDWR) };
    }

    #[cfg(not(unix))]
    fn disconnect(&self) {}
}

#[cfg(unix)]
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        // SAFETY: as `disconnect`, and this is the one place it is released.
        unsafe { close(self.0) };
    }
}

/// What a legacy `/sse` handshake produces that nothing else on the request
/// can tell us, gathered while that handshake is being served.
///
/// Two values, because the SDK withholds both from the `/sse` path and for two
/// different reasons — see [`RecordingIds`] and [`NoteVerifiedIdentity`].
#[derive(Default)]
struct SseHandshake {
    /// The session id `handle_sse_connection` minted while answering.
    session_id: Option<String>,
    /// The identity `AuthMiddleware` verified for this same request.
    identity: Option<rust_mcp_sdk::auth::AuthInfo>,
}

tokio::task_local! {
    /// Where a `/sse` handshake's two withheld values are left for
    /// [`note_streaming_connection`] to collect once the response head exists.
    ///
    /// A task-local, and that is the load-bearing choice. Everything that
    /// writes it runs on the **same task** that is serving the request —
    /// awaited inline through the SDK's middleware chain and its handler,
    /// before anything is spawned — so the values collected are this
    /// request's and no other's. The alternative, snapshotting the session
    /// store before and after and taking the difference, races a concurrent
    /// handshake and can name another client's session; a revoke that then
    /// tore down the wrong connection is the one failure this whole mechanism
    /// exists to avoid.
    ///
    /// An `Arc` the middleware keeps a second handle to, rather than a value
    /// the scope owns, because the scope's value is consumed along with the
    /// future and these have to be read *after* it resolves.
    static SSE_HANDSHAKE: Arc<Mutex<SseHandshake>>;
}

/// The SDK's own session-id generator, plus one side effect.
///
/// **Why a browser has to intercept this.** A Streamable-HTTP client names its
/// session in an `Mcp-Session-Id` request header, so
/// [`note_streaming_connection`] can read it straight off the request. The
/// legacy `/sse` handshake has no such header: the id is minted server-side
/// inside `handle_sse_connection` and appears only in the response body's
/// first `endpoint` event, which is a stream this listener must not buffer or
/// re-frame. Without the id there is nothing to key [`Connections::attach`]
/// on, and a revoked client's `/sse` stream keeps delivering — threat T-7, on
/// the transport `Connections` was built to close (CR-02).
///
/// Outside the scope [`note_streaming_connection`] enters, `try_with` fails
/// and this is `UuidGenerator` exactly. A Streamable-HTTP session id is minted
/// outside it and is not recorded, because nothing needs it recorded.
struct RecordingIds(UuidGenerator);

impl rust_mcp_sdk::id_generator::IdGenerator<rust_mcp_sdk::SessionId> for RecordingIds {
    fn generate(&self) -> rust_mcp_sdk::SessionId {
        let id: rust_mcp_sdk::SessionId = self.0.generate();
        let _ = SSE_HANDSHAKE.try_with(|handshake| {
            if let Ok(mut handshake) = handshake.lock() {
                handshake.session_id = Some(id.clone());
            }
        });
        id
    }
}

/// Carry the verified identity out of the SDK's chain, for the one request
/// shape that would otherwise lose it.
///
/// **The SDK drops it, and this is the measured reason a revoke could not
/// reach a `/sse` stream even once its connection was tracked.**
/// `McpHttpHandler::handle_sse_connection` reads the request's `AuthInfo`
/// extension *before* it composes the middleware chain:
///
/// ```text
/// let (request, auth_info) = request.take::<AuthInfo>();   // always None
/// ...
/// let handle = compose(&self.middlewares, final_handler);  // AuthMiddleware
/// ```
///
/// `AuthMiddleware` is what inserts that extension, and it runs inside
/// `compose`. So the value captured is always `None`, every `/sse` session is
/// created with no identity, and two things follow that are not obvious:
/// `terminate_matching` cannot select the session for a revoke — an
/// unidentified session is deliberately nobody's — and a tool call arriving
/// over that session is attributed to [`UNVERIFIED_CLIENT`] rather than to the
/// client that presented the token.
///
/// This middleware sits **last** in the chain, so it runs after
/// `AuthMiddleware` (`compose` folds in reverse, so the first entry runs
/// first) and sees the extension. It reads and forwards, and decides nothing:
/// a request that reaches it has already been admitted, and one that was
/// refused never gets here.
struct NoteVerifiedIdentity;

#[async_trait::async_trait]
impl Middleware for NoteVerifiedIdentity {
    async fn handle<'req>(
        &self,
        request: Request<&'req str>,
        state: Arc<McpAppState>,
        next: MiddlewareNext<'req>,
    ) -> McpHttpResult<Response<GenericBody>> {
        if let Some(identity) = request.extensions().get::<rust_mcp_sdk::auth::AuthInfo>() {
            let identity = identity.clone();
            let _ = SSE_HANDSHAKE.try_with(|handshake| {
                if let Ok(mut handshake) = handshake.lock() {
                    handshake.identity = Some(identity);
                }
            });
        }
        next(request, state).await
    }
}

/// The connection a request arrived on, as axum's own per-connection value.
///
/// **This replaces a waiting room, and the replacement is the security
/// property.** The first version of this mechanism noted every accepted
/// connection's descriptor in a `HashMap<SocketAddr, i32>` at `accept(2)`
/// time and looked it up again when a stream opened. Two things were wrong
/// with that and both were reachable by any local account, before any
/// authentication:
///
/// - The map had to be bounded, and the bound was enforced by clearing it. A
///   peer opening enough short-lived connections — each with its own ephemeral
///   port, so each its own key — flushed every legitimate entry, and a stream
///   whose entry was gone became one a later revoke could not close (WR-01).
/// - The lookup consumed the entry, so the *second* standalone stream on one
///   keep-alive connection was untracked. An event-stream reconnect over a
///   live connection is the ordinary case, not an exotic one (WR-02).
///
/// Carrying the descriptor on the connection itself deletes both. axum
/// computes this once per accepted connection and puts it in every request's
/// extensions, so there is no table to flood, nothing to evict, and nothing to
/// consume: every request on a connection sees that connection's descriptor,
/// however many streams it carries over its life.
///
/// The descriptor is a **number**, and it is safe as one for the same reason
/// [`OwnedSocket::duplicate`]'s doc gives — except that the reason is now
/// structural rather than argued. This value exists only while the connection
/// it was computed from is being served, so the number cannot have been
/// recycled onto something else by the time a handler reads it.
#[derive(Debug, Clone, Copy)]
struct PeerConnection {
    /// The peer address axum accepted from. Logging only: nothing is keyed on
    /// it any more, which is the whole of WR-01's fix.
    peer: SocketAddr,
    /// The accepted socket's descriptor number.
    fd: i32,
}

impl Connected<IncomingStream<'_, tokio::net::TcpListener>> for PeerConnection {
    fn connect_info(stream: IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        #[cfg(unix)]
        let fd = {
            use std::os::fd::AsRawFd as _;
            stream.io().as_raw_fd()
        };
        // Off Unix every operation on the descriptor is a no-op
        // ([`OwnedSocket`]), so the value is never read.
        #[cfg(not(unix))]
        let fd = -1;
        Self { peer: *stream.remote_addr(), fd }
    }
}

/// Which connection each live standalone stream is riding on.
///
/// **Why a browser has to know this at all.** `04-02-SPIKE.md`'s A6 finding
/// left one question open: whether deleting a session terminates a stream that
/// is already open. It does not, and the reason is in the SDK rather than in
/// this browser. `ServerRuntime::shutdown` cancels the *reader* — the
/// client-to-server direction — and the response body is fed from the other
/// half of a duplex the transport's message dispatcher still owns, held by a
/// task that outlives the session-store entry. So the body never reaches
/// end-of-stream and a revoked agent keeps receiving. Nothing in the published
/// API drops that half, and the fallback `04-RESEARCH.md` named — the stream
/// re-verifying its own token on each keep-alive tick — lives inside the SDK's
/// keep-alive task too, which is equally out of reach.
///
/// What *is* reachable is the connection. This listener accepts it, so it can
/// close it, and closing it is what the client's read side sees as the stream
/// ending. The bound is therefore immediate rather than one keep-alive
/// interval.
///
/// One map, keyed on the session, because the connection now arrives with the
/// request that names the session — see [`PeerConnection`], which is what
/// replaced the peer-address waiting room this type used to carry.
#[derive(Default)]
struct Connections {
    /// Session id → a duplicate of the descriptor its standalone stream rides
    /// on.
    ///
    /// **Bounded by the number of sessions the SDK's own store still holds,
    /// and pruned to that on every [`Connections::attach`]** — which is to
    /// say, on the one path that runs whenever a stream opens. An earlier
    /// version claimed the same bound while pruning only from
    /// [`terminate_matching`], whose two callers are a human's revoke and the
    /// listener's shutdown; in an ordinary session neither ever runs, so every
    /// reconnecting agent left one `dup`ed descriptor behind for the lifetime
    /// of a process that also holds the credential vault, the engine and the
    /// control socket (CR-03). The revoke-time prune is still there, and is
    /// now the second one rather than the only one.
    streaming: Mutex<HashMap<String, OwnedSocket>>,
}

impl Connections {
    /// Bind a session's standalone stream to the connection it arrived on.
    ///
    /// Called from the middleware serving a request **on that connection**, so
    /// `connection.fd` is open for the duration of the duplication. A second
    /// stream over the same connection simply overwrites the first session's
    /// entry or adds its own; nothing is consumed, so nothing goes untracked
    /// (WR-02).
    /// `live` is the session store's own list of ids, read for this request.
    /// Everything held for a session that is no longer in it is released here.
    fn attach(&self, session_id: &str, connection: PeerConnection, live: &[String]) {
        let Some(socket) = OwnedSocket::duplicate(connection.fd) else {
            // Worth a line rather than a silent return: a stream this browser
            // cannot close is one a revoke will not stop.
            log::warn!(
                "could not duplicate the socket carrying session {session_id}; \
                 that stream will not close on a revoke"
            );
            return;
        };
        log::debug!(
            "session {session_id} opened a stream on the connection from {}",
            connection.peer
        );
        if let Ok(mut streaming) = self.streaming.lock() {
            // Pruned *here*, before the insert, because this is the one path
            // that runs every time a stream opens. A reconnecting client gets
            // a new session id from the SDK's generator, so nothing overwrites
            // the entry it left behind and its duplicated descriptor is
            // released only by a `Drop` that a revoke may never cause. The
            // held id is spared unconditionally: it is the session being
            // attached and may not have reached the store's listing yet.
            streaming
                .retain(|held, _| held == session_id || live.iter().any(|id| id == held));
            streaming.insert(session_id.to_owned(), socket);
        }
    }

    /// How many streams are tracked.
    ///
    /// Test-only: nothing on a shipped path needs the number, and an accessor
    /// that exists only to be asserted on is better compiled out than left as
    /// a surface a later reader has to account for.
    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.streaming.lock().map(|streaming| streaming.len()).unwrap_or_default()
    }

    /// Close one session's stream connection. `true` if there was one.
    fn disconnect(&self, session_id: &str) -> bool {
        let socket = match self.streaming.lock() {
            Ok(mut streaming) => streaming.remove(session_id),
            Err(_) => None,
        };
        match socket {
            Some(socket) => {
                socket.disconnect();
                true
            },
            None => false,
        }
    }

    /// Forget every stream whose session the store no longer has, releasing
    /// its duplicated descriptor.
    fn retain_live(&self, live: &[String]) {
        if let Ok(mut streaming) = self.streaming.lock() {
            streaming.retain(|session_id, _| live.iter().any(|id| id == session_id));
        }
    }
}

/// The open `/view` WebSockets, addressable by the client that owns them.
///
/// **The stream registry's second half, and it is required rather than
/// preferred.** [`terminate_matching`] works over the SDK's *session
/// directory*: it walks the sessions the SDK's own store holds and ends each
/// one. A view socket is not an SDK session — it never runs `initialize`, it
/// mints no session id, and it is nowhere in that store — so no amount of
/// walking that directory reaches it. A revoke that closed only what the
/// directory can name would leave a revoked viewer watching, which is the
/// same failure [`StreamRegistry`] already exists for, in a new shape (T-05-07).
///
/// A `Vec` rather than a map keyed on the client, because one client may hold
/// several sockets and the numbers here are small: a handful of viewers, each
/// one entry, walked only on a revoke or a shutdown.
///
/// The close handle is a [`oneshot::Sender`] the socket task selects on, and
/// it is *taken* out of this table to fire — so a socket cannot be closed
/// twice and a completed connection leaves nothing behind to close later.
#[derive(Default)]
struct ViewSockets {
    open: Mutex<Vec<ViewSocketHandle>>,
}

/// One accepted view socket, as the registry sees it.
struct ViewSocketHandle {
    /// This connection's own id, unique for the life of the process, and the
    /// only key [`ViewSockets::remove`] uses. Never a descriptor number and
    /// never a peer address — see [`PeerConnection`] for why either would be
    /// a table an unauthenticated peer could confuse.
    connection: u64,
    /// The `client_id` the route verified before the socket was accepted. A
    /// socket whose identity could not be established is never registered,
    /// because it is never accepted.
    client_id: String,
    /// Fires once. The socket task is waiting on the other end, and stops.
    close: oneshot::Sender<()>,
}

impl ViewSockets {
    /// Track an accepted socket. Called once per connection, at accept time,
    /// before the client is told anything.
    fn register(&self, connection: u64, client_id: &str, close: oneshot::Sender<()>) {
        if let Ok(mut open) = self.open.lock() {
            open.push(ViewSocketHandle {
                connection,
                client_id: client_id.to_owned(),
                close,
            });
        }
    }

    /// Forget a socket that ended on its own. `true` if it was still tracked.
    ///
    /// The other half of `register`, and the reason a completed connection
    /// leaves nothing behind: this table is not a log of connections that
    /// happened, it is the set of connections that are open.
    fn remove(&self, connection: u64) -> bool {
        let Ok(mut open) = self.open.lock() else { return false };
        let before = open.len();
        open.retain(|handle| handle.connection != connection);
        open.len() != before
    }

    /// Close every socket belonging to `client_id`, and no other client's.
    /// Returns how many were closed.
    fn close_client(&self, client_id: &str) -> usize {
        self.close_matching(Some(client_id))
    }

    /// Close every open socket, whoever owns it — the listener's way out.
    fn close_all(&self) -> usize {
        self.close_matching(None)
    }

    /// Take the matching handles out and fire each. Taking is what makes a
    /// second termination of the same socket unrepresentable.
    fn close_matching(&self, client_id: Option<&str>) -> usize {
        let taken: Vec<ViewSocketHandle> = match self.open.lock() {
            Ok(mut open) => {
                let mut kept = Vec::with_capacity(open.len());
                let mut taken = Vec::new();
                for handle in open.drain(..) {
                    match client_id {
                        Some(wanted) if handle.client_id != wanted => kept.push(handle),
                        _ => taken.push(handle),
                    }
                }
                *open = kept;
                taken
            },
            // Degrade, never abort — and the store half of the revoke, which
            // is what refuses the next upgrade, is untouched either way.
            Err(_) => {
                log::error!("the view socket table is poisoned; no view socket was closed");
                return 0;
            },
        };
        let closed = taken.len();
        for handle in taken {
            // A receiver that is already gone means the task is on its way
            // out under its own power, which is the outcome asked for.
            let _ = handle.close.send(());
        }
        closed
    }

    /// How many sockets are tracked. Test-only, for the same reason
    /// [`Connections::tracked`] is.
    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.open.lock().map(|open| open.len()).unwrap_or_default()
    }
}

/// One live MCP session, paired with the client identity that was verified
/// when it was created.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionOwner {
    /// The id the SDK minted for the session.
    session_id: String,
    /// The `client_id` from the `AuthInfo` in force when the session started,
    /// or `None` for a session that somehow has none.
    ///
    /// `None` never matches a revoke. A revocation names one client, and a
    /// session that cannot say whose it is is not that client's — treating an
    /// absent identity as a match would let one revoke terminate every other
    /// agent's stream.
    client_id: Option<String>,
}

/// The bookkeeping [`StreamRegistry`] needs from whatever is actually serving
/// streams, expressed as a trait so the selection logic can be tested without
/// a bound port.
///
/// Two operations and no more: list, and end one. Everything interesting about
/// revocation is *which* sessions get ended, and that is
/// [`terminate_matching`], which is pure.
#[async_trait::async_trait]
trait SessionDirectory: Send + Sync {
    /// Every session currently being served, with its owner.
    async fn sessions(&self) -> Vec<SessionOwner>;
    /// End one session, closing whatever response stream it holds open.
    async fn end(&self, session_id: &str);
}

/// The live response streams this listener is serving, addressable by the
/// client that owns them.
///
/// **Why this exists at all.** Verification runs *per request*
/// ([`crate::oauth::TalariaAuth::verify_token`]), so a revoked client's next
/// request is refused for free — but an already-open `text/event-stream` has
/// no next request to fail. A revocation that only stopped new requests would
/// leave a revoked agent still receiving from this browser while the Access
/// panel showed its row gone: `04-RESEARCH.md`'s threat T-7, and the single
/// most likely way Success Criterion 3 gets marked done while being false.
/// [`crate::agents`] cannot close the gap — it deliberately has no clock, no
/// I/O and no knowledge of transports — so the second half of a revoke lives
/// here, and `UiAction::RevokeClient` performs both halves in one arm.
///
/// **The mechanism, and its bound.** `04-02-SPIKE.md`'s A6 finding settled
/// that a live session is addressable by client identity — `auth_info_cloned()`
/// on the session's runtime returns the `client_id` that was verified when it
/// was created — and left one thing open: whether ending the session also
/// terminates a stream that is already open. **It does not**, and this was
/// measured rather than reasoned about: the suite asserted the closure from
/// the client end and it failed. `ServerRuntime::shutdown` cancels the
/// *reader*, and the response body is fed from the other half of a duplex that
/// the transport's message dispatcher still owns, on a task that outlives the
/// session-store entry. Nothing in the published API drops that half; the
/// fallback `04-RESEARCH.md` named — the stream re-verifying its own token on
/// each keep-alive tick — is inside the SDK's own keep-alive task and is
/// equally out of reach.
///
/// So ending a session here is **two** actions, and only the first is one the
/// client can observe: closing the connection the stream is riding on, which
/// is what its read side sees as the body ending (see [`Connections`]); and
/// then the SDK's own session delete, which retires the session so its id is
/// dead and its runtime is released. The bound is **immediate** rather than
/// one keep-alive interval.
///
/// **Nothing on the network can reach the handler the second action uses.** It
/// is a second [`McpHttpHandler`] with an empty middleware chain, held
/// privately and mounted on no route. That is not a hole in authentication: it
/// is not reachable *at all*, in the same way `apply_ui_actions` is not
/// reachable. The one caller is the chrome's own revoke.
#[derive(Clone, Default)]
pub struct StreamRegistry {
    /// `None` until the listener has bound and built its state — the window
    /// between [`spawn`] returning a handle and [`serve`] having something to
    /// register. A revoke in that window terminates nothing because there is
    /// nothing yet to terminate.
    inner: Arc<Mutex<Option<Live>>>,
}

/// What a bound listener installs into its registry.
#[derive(Clone)]
struct Live {
    directory: Arc<dyn SessionDirectory>,
    /// The open view sockets — the half the directory cannot reach, because a
    /// WebSocket is not an SDK session. See [`ViewSockets`].
    views: Arc<ViewSockets>,
    /// The listener thread's runtime, so the main thread can hand it work
    /// without blocking on it. Terminating is fire-and-forget: the human's
    /// feedback is the row disappearing, which `apply_ui_actions` has already
    /// caused by the time this is called.
    runtime: tokio::runtime::Handle,
}

impl StreamRegistry {
    /// Attach a bound listener's session bookkeeping. Called once, from
    /// [`serve`], after the address is known.
    fn install(
        &self,
        directory: Arc<dyn SessionDirectory>,
        views: Arc<ViewSockets>,
        runtime: tokio::runtime::Handle,
    ) {
        match self.inner.lock() {
            Ok(mut inner) => *inner = Some(Live { directory, views, runtime }),
            // Degrade, never abort. A poisoned lock here costs stream
            // termination, not the browser — and the store half of a revoke,
            // which is the half that stops the next request, is untouched.
            Err(_) => log::error!("the stream registry lock is poisoned; streams will not close"),
        }
    }

    /// Detach it again, when the listener stops.
    fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = None;
        }
    }

    /// Close every stream **and every view socket** belonging to `client_id`,
    /// and no other client's.
    ///
    /// Both halves in one call, deliberately: a caller that had to remember to
    /// perform the second one is a caller that will eventually forget, and the
    /// thing forgotten would be a revoked viewer still watching. The view half
    /// runs first and synchronously, because it is a channel send rather than
    /// an engine round trip and there is no reason to make it wait.
    ///
    /// Returns immediately; the session work runs on the listener's own
    /// runtime. Safe to call from the winit main thread, which is the only
    /// caller.
    pub fn terminate_client(&self, client_id: &str) {
        let Some(live) = self.live() else { return };
        let views = live.views.close_client(client_id);
        if views > 0 {
            log::info!("closed {views} open view socket(s) belonging to client {client_id}");
        }
        let client_id = client_id.to_owned();
        live.runtime.spawn(async move {
            let closed = terminate_matching(live.directory.as_ref(), Some(&client_id)).await;
            if closed > 0 {
                log::info!("closed {closed} open stream(s) belonging to client {client_id}");
            }
        });
    }

    /// The `Live` half, or `None` when no listener is bound.
    fn live(&self) -> Option<Live> {
        match self.inner.lock() {
            Ok(inner) => inner.clone(),
            Err(_) => {
                log::error!("the stream registry lock is poisoned; no stream was closed");
                None
            },
        }
    }
}

/// End every session whose owner matches `client_id`, or every session at all
/// when it is `None`. Returns how many were ended.
///
/// The whole of the revoke's stream half, and pure enough to unit-test: the
/// directory is a trait, so the interesting properties — one client's sessions
/// go and another's do not, a session that completed normally is not there to
/// be ended, and `None` takes everything — are assertable without a port.
async fn terminate_matching(directory: &dyn SessionDirectory, client_id: Option<&str>) -> usize {
    let mut closed = 0usize;
    for session in directory.sessions().await {
        let matches = match client_id {
            // An unidentified session is nobody's: see `SessionOwner::client_id`.
            Some(wanted) => session.client_id.as_deref() == Some(wanted),
            None => true,
        };
        if matches {
            directory.end(&session.session_id).await;
            closed += 1;
        }
    }
    closed
}

/// The real directory: the SDK's own session store, plus the SDK's own
/// handler for ending one.
struct McpSessions {
    state: Arc<McpAppState>,
    /// A handler with an **empty** middleware chain, on no route. See
    /// [`StreamRegistry`] for why that is not a bypass.
    internal: McpHttpHandler,
    /// Which connection each live standalone stream rides on — the half of a
    /// termination the SDK cannot express. See [`Connections`].
    connections: Arc<Connections>,
}

#[async_trait::async_trait]
impl SessionDirectory for McpSessions {
    async fn sessions(&self) -> Vec<SessionOwner> {
        let mut found = Vec::new();
        for session_id in self.state.session_store.keys().await {
            // A session that vanished between the listing and this lookup is
            // one that already ended, which is the outcome a revoke wanted.
            let Some(runtime) = self.state.session_store.get(&session_id).await else {
                continue;
            };
            let client_id = runtime.auth_info_cloned().await.and_then(|info| info.client_id);
            found.push(SessionOwner { session_id, client_id });
        }
        // The natural pruning point for the connection table: it runs on every
        // revoke and every shutdown, and it already holds the authoritative
        // list of sessions that still exist. A duplicated descriptor for a
        // session that is gone is a descriptor this process is holding open
        // for nothing.
        let live: Vec<String> =
            found.iter().map(|session| session.session_id.clone()).collect();
        self.connections.retain_live(&live);
        found
    }

    async fn end(&self, session_id: &str) {
        // **First, and it is the half that the client can actually observe.**
        // Closing the connection is what ends the response body, which is what
        // a revoked agent's read side sees as the stream ending; the session
        // delete below does not, for the reason [`Connections`] records.
        if self.connections.disconnect(session_id) {
            log::debug!("closed the connection carrying session {session_id}");
        }
        let Ok(value) = HeaderValue::from_str(session_id) else {
            // Unreachable for an id this store minted; answered rather than
            // asserted, because this runs on the listener's runtime.
            log::warn!("a session id could not be put into a header; its stream stays open");
            return;
        };
        let mut headers = HeaderMap::new();
        headers.insert(MCP_SESSION_ID_HEADER, value);
        let request = McpHttpHandler::create_request(
            Method::DELETE,
            Uri::from_static(MCP_PATH),
            headers,
            None,
        );
        // The SDK's `handle_http_delete` shuts the session's transport down
        // and then removes it from the store — which is exactly what a client
        // disconnecting does, and exactly what closes an open stream. The
        // response is discarded: nothing is waiting for it.
        if let Err(error) = self.internal.handle_streamable_http(request, self.state.clone()).await
        {
            log::warn!("could not end session {session_id}: {error}");
        }
    }
}

/// Start the listener on its own thread.
///
/// Two deliberate divergences from [`crate::control::spawn`], which this
/// otherwise copies:
///
/// - The runtime is multi-threaded, where the control thread's is a
///   current-thread one. The control socket must never be starved by an HTTP
///   request, and `rust-mcp-axum` is written against a full tokio.
/// - It is **conditional**. `control::spawn` runs unconditionally at startup;
///   this runs only when remote access is enabled, so a default install pays
///   nothing — no thread, no runtime, no bound port.
///
/// `proxy` is cloned by the caller *before* the spawn: `Shared` is `Rc`-based
/// and therefore not `Send`, so the proxy is the whole of what crosses the
/// thread boundary — with one deliberate exception. `agents` is an
/// [`std::sync::Arc`] over the *same* store the main thread holds, and that is
/// the point: the chrome's revoke and the listener's verification must see one
/// store, and two independently-loaded ones would give them different answers.
/// See [`crate::oauth::SharedAgents`].
///
/// Returns the registry as well as the shutdown handle, empty until the
/// listener has bound: the state a revoke terminates streams through does not
/// exist until [`serve`] has an address to build it around, and handing the
/// caller a handle that fills itself in is what keeps `Shared` from having to
/// learn about the bind.
pub fn spawn(
    proxy: EventLoopProxy<AppEvent>,
    agents: SharedAgents,
    port: u16,
    advertised: Option<String>,
) -> (ShutdownHandle, StreamRegistry) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let streams = StreamRegistry::default();
    let installed = streams.clone();
    let thread = std::thread::Builder::new()
        .name("talaria-http".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    // Degrade, never abort: a browser that cannot start a
                    // listener is still a browser.
                    log::error!("remote access runtime could not be built: {error}");
                    let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
                        addr: format!("{BIND_HOST}:{port}"),
                        error: error.to_string(),
                    });
                    return;
                },
            };
            runtime.block_on(serve(proxy, agents, port, advertised, shutdown_rx, installed));
        });
    if let Err(error) = thread {
        log::error!("remote access thread could not be spawned: {error}");
    }
    (ShutdownHandle(shutdown_tx), streams)
}

/// Bind, report, serve, and stop.
async fn serve(
    proxy: EventLoopProxy<AppEvent>,
    agents: SharedAgents,
    port: u16,
    advertised: Option<String>,
    shutdown: oneshot::Receiver<()>,
    streams: StreamRegistry,
) {
    let requested = format!("{BIND_HOST}:{port}");
    let address = match requested.parse::<SocketAddr>() {
        Ok(address) => address,
        Err(error) => {
            log::error!("remote access address {requested} is unusable: {error}");
            let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
                addr: requested,
                error: error.to_string(),
            });
            return;
        },
    };
    // Logged and reported, never a panic — the same shape
    // `control::serve`'s bind failure takes. **There is no fallback bind:** if
    // loopback at the configured port is unavailable, remote access stays off
    // rather than landing somewhere nobody asked for.
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("remote access bind failed at {requested}: {error}");
            let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
                addr: requested,
                error: error.to_string(),
            });
            return;
        },
    };
    // The address that was actually bound, from the socket itself. Everything
    // the human is shown, and the origin `parse_agent_url` refuses, comes from
    // here rather than from `requested`.
    let bound = match listener.local_addr() {
        Ok(bound) => bound.to_string(),
        Err(error) => {
            log::error!("remote access bound at {requested} but cannot name itself: {error}");
            let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
                addr: requested,
                error: error.to_string(),
            });
            return;
        },
    };

    // One session id, from the counter the control socket shares, allocated
    // once for the whole listener rather than per request — a per-request id
    // would make every tool call a different owner, and tab ownership is the
    // thing a session id decides.
    //
    // Per-*client* identity is the other half, and it arrives here: the tab
    // owner label a tool call carries is now the **verified** `client_id` from
    // the presented token rather than a name the peer typed. See
    // [`Handler::handle_call_tool_request`].
    // **The two facts, resolved once, here — the one place that knows both.**
    // `bound` is the socket: what was actually bound, what the chrome shows,
    // and what a local client addresses. `identity` is what this browser
    // *publishes about itself*, which is the same string only when nothing
    // fronts the listener. Everything downstream takes whichever of the two it
    // actually means, and neither is ever re-derived from a request.
    let identity = AdvertisedIdentity::new(&bound, advertised.as_deref());
    let sink = EventLoopSink { proxy: proxy.clone(), session_id: next_session_id() };
    let connections = Arc::new(Connections::default());
    let views = Arc::new(ViewSockets::default());
    let (app, directory) = build_router(
        proxy.clone(),
        sink,
        agents,
        &bound,
        &identity,
        Arc::clone(&connections),
        Arc::clone(&views),
    );
    // Installed before the first connection is accepted, so there is no window
    // in which a session exists and a revoke cannot reach it. Both halves go
    // in together for the same reason: a view socket accepted in a window
    // where only the directory was installed would be one a revoke missed.
    streams.install(
        Arc::clone(&directory),
        Arc::clone(&views),
        tokio::runtime::Handle::current(),
    );

    // The socket, not the advertised identity: this line is a statement about
    // what this process bound, and a log that named an origin the operator
    // configured would stop being evidence of what actually happened.
    log::info!("remote access listening at {BIND_HOST}:{port}{MCP_PATH} (bound {bound})");
    if identity.authority() != bound {
        log::info!("remote access is advertised as {}", identity.origin());
    }
    let _ = proxy.send_event(AppEvent::RemoteListenerBound { addr: bound.clone() });

    // Two clocks, because `axum`'s graceful shutdown waits for every open
    // connection and an event-stream connection may never close on its own:
    // the switch stops the listener accepting, and the grace period below is
    // what stops "turn it off" from hanging on a client that will not let go.
    // Dropping the server future drops those connections, which closes their
    // sinks' `oneshot` receivers, which is what resolves any request still in
    // flight as an error its caller receives rather than stranding it.
    //
    // Every open stream is closed *as the switch is flicked*, not after the
    // grace period expires: a `text/event-stream` is a connection that will
    // never close on its own, so ending each session's transport first is what
    // lets the graceful shutdown actually be graceful. It is the same
    // termination a revoke performs, aimed at every client instead of one.
    let (drained_tx, drained_rx) = oneshot::channel::<()>();
    let closing = Arc::clone(&directory);
    let closing_views = Arc::clone(&views);
    // Each accepted connection's descriptor travels with the connection, in
    // axum's own per-connection value, rather than through a table keyed on
    // the peer address. See [`PeerConnection`] for the two ways the table
    // could be defeated and why carrying the number instead deletes both.
    let server = rust_mcp_axum::axum::serve(
        listener,
        app.into_make_service_with_connect_info::<PeerConnection>(),
    )
        .with_graceful_shutdown(async move {
            let _ = shutdown.await;
            let closed = terminate_matching(closing.as_ref(), None).await;
            if closed > 0 {
                log::info!("remote access closed {closed} open stream(s) on its way out");
            }
            // The view half, which the directory above cannot reach. A
            // WebSocket is a connection axum's graceful shutdown waits for,
            // and a viewer that never lets go would otherwise be exactly the
            // client the grace period exists to stop hanging on.
            let sockets = closing_views.close_all();
            if sockets > 0 {
                log::info!("remote access closed {sockets} open view socket(s) on its way out");
            }
            let _ = drained_tx.send(());
        })
        .into_future();
    tokio::pin!(server);
    let outcome = tokio::select! {
        result = &mut server => result,
        _ = async move {
            let _ = drained_rx.await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => {
            log::warn!(
                "remote access did not finish its open requests within {SHUTDOWN_GRACE:?}; \
                 closing them"
            );
            Ok(())
        },
    };

    // Nothing is listening from here on, so nothing can be revoked from a
    // panel either — a registry still holding a dead runtime's handle would
    // hand a later revoke work that silently goes nowhere.
    streams.clear();

    // Reached both when the shutdown handle fired and when the server stopped
    // on its own. The second case must correct the UI rather than leave it
    // claiming a listener that is gone, so a server error is reported; an
    // ordinary shutdown is not, because `apply_ui_actions` has already written
    // `RemoteAccess::Off` by the time it happens.
    if let Err(error) = outcome {
        log::error!("remote access stopped: {error}");
        let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
            addr: bound,
            error: error.to_string(),
        });
    } else {
        log::info!("remote access stopped");
    }
}

/// Build the MCP router: the SDK's own routes, behind our middleware chain.
///
/// Mirrors what `AxumServer::new` composes, so a later reader comparing the
/// two finds one difference and not a rewrite. The state's fields are the
/// SDK's own defaults, spelled out rather than inherited because this is the
/// BYO-server path and there is no options struct to inherit them from.
///
/// Returns the session directory alongside the router, because the state the
/// router is built from is also the thing a revoke has to reach — see
/// [`StreamRegistry`].
#[allow(clippy::too_many_arguments)]
fn build_router(
    proxy: EventLoopProxy<AppEvent>,
    sink: EventLoopSink,
    agents: SharedAgents,
    bound: &str,
    identity: &AdvertisedIdentity,
    connections: Arc<Connections>,
    views: Arc<ViewSockets>,
) -> (rust_mcp_axum::axum::Router, Arc<dyn SessionDirectory>) {
    // Cloned before the consent closure below takes ownership of `proxy`: the
    // view route needs the same route onto the main thread, for the same
    // reason and with the same restriction — `Shared` is `Rc`-based, so an
    // `AppEvent` is the whole of what may travel.
    let view_proxy = proxy.clone();
    // The real provider. 04-03's interim refuse-everything one was deleted
    // rather than parked beside this line: a struct that refuses every
    // credential is harmless on its own and a footgun one line away from the
    // struct that does not, because a later edit only has to re-point a single
    // `Arc::new` to disable authentication entirely.
    // How the authorization endpoint asks a human. `Shared` is `Rc`-based and
    // cannot cross a thread boundary, so an `AppEvent` on this proxy is the
    // whole of what may travel — the same route `EventLoopSink` takes for a
    // tool call, and the same reason. The provider takes a callback rather
    // than the proxy itself so that the window loop stays this module's
    // business: `oauth.rs` owns what is asked and what the answer means.
    let raise: ConsentRaiser = Arc::new(move |request| {
        proxy.send_event(AppEvent::ConsentRequested(request)).map_err(|_| ())
    });
    let auth: Arc<dyn AuthProvider> = Arc::new(TalariaAuth::new(agents, raise, identity));
    // The **same** provider handle, called directly by the view route. See
    // [`view_upgrade`] for why a merged axum route cannot inherit the chain
    // built from it below.
    let view_auth = Arc::clone(&auth);
    // The two names this server answers to, enumerated once and shared by both
    // layers that check a `Host`. See [`admitted_hosts`].
    let admitted: Arc<[String]> = admitted_hosts(bound, identity).into();
    let state = Arc::new(McpAppState {
        session_store: Arc::new(InMemorySessionStore::default()),
        // `UuidGenerator` with one addition; see [`RecordingIds`] for why the
        // legacy SSE handshake cannot be tracked without it.
        id_generator: Arc::new(RecordingIds(UuidGenerator {})),
        stream_id_gen: Arc::new(FastIdGenerator::new(Some("s_"))),
        server_details: Arc::new(server_details()),
        handler: Handler { sink }.to_mcp_server_handler(),
        ping_interval: Duration::from_secs(12),
        transport_options: Default::default(),
        // Replies come back as an event stream, the SDK's own default.
        enable_json_response: false,
        // Resumability is not designed yet: an event store would mean
        // deciding how long a disconnected client may replay from, which is a
        // question this phase has not asked.
        event_store: None,
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    // Order matters, and `compose` runs the first entry first:
    //
    // 1. the `Origin` refusal, because a page-originated request should be
    //    turned away before anything else looks at it;
    // 2. the SDK's host validation, with `allowed_hosts` set **explicitly**
    //    from the address actually bound and the origin this browser
    //    advertises, rather than inferred from a request — this is on top of
    //    the refusal above, not instead of it (T-2);
    // 3. the auth middleware, which is what returns the 401 and builds the
    //    `WWW-Authenticate` challenge from the provider's answer;
    // 4. a read-only note of the identity the middleware above verified,
    //    **last** so that it runs after it. It admits nothing and refuses
    //    nothing; see [`NoteVerifiedIdentity`] for the SDK ordering defect it
    //    exists to work around.
    let middlewares: Vec<Arc<dyn Middleware>> = vec![
        Arc::new(RefuseOriginHeader),
        Arc::new(DnsRebindProtector::new(Some(admitted.to_vec()), None)),
        Arc::new(AuthMiddleware::new(auth.clone())),
        Arc::new(NoteVerifiedIdentity),
    ];
    let http_handler = McpHttpHandler::new(Some(auth), middlewares, None);
    // The SDK's default mount: `/mcp` for Streamable HTTP, no health
    // endpoint, the default 4 MiB body cap.
    //
    // Note for a later reader: `mcp_routes` also mounts the legacy `/sse` and
    // `/messages` endpoints unconditionally in 1.0.1 — `AxumServerOptions`'
    // `sse_support` flag only affects a log line there, so the all-in-one path
    // mounts them too. Both sit behind this same `middlewares` chain as
    // `/mcp`, so they are covered by the real provider exactly as they were by
    // the interim one: an unauthenticated request to either is refused by
    // `AuthMiddleware` before it reaches a handler, and one carrying an
    // `Origin` header is refused before that. What a revoke does to a stream
    // already open on one is answered by [`Connections`], which works on the
    // connection rather than on the route.
    //
    // `mcp_routes` also merges the SDK's auth routes, folded over the
    // endpoints the provider declares — which is how
    // `/.well-known/oauth-protected-resource` becomes a route without this
    // function naming it. Those routes run with an *empty* middleware chain
    // (`compose(&[], ..)`, measured in `04-02-SPIKE.md`), which is both
    // deliberate and required: a metadata document a client must fetch
    // *before* it has a token cannot itself demand one.
    let mount = McpMountOptions::default();
    // The directory's own handler is a *second* `McpHttpHandler`, built with
    // no auth provider and an empty middleware chain, and it is never handed
    // to `mcp_routes`. It exists to reach the SDK's session-delete path from
    // inside this process; see [`StreamRegistry`] for why that is not a
    // bypass of the chain above.
    let directory: Arc<dyn SessionDirectory> = Arc::new(McpSessions {
        state: Arc::clone(&state),
        internal: McpHttpHandler::new(None, Vec::new(), None),
        connections: Arc::clone(&connections),
    });

    // One axum-level layer, outside the SDK's chain, and it exists for one
    // reason: the SDK's route handlers rebuild their request from a `HeaderMap`
    // and a `Uri` alone, so the connection the request arrived on is knowable
    // *here* and nowhere further in. It records which connection each
    // standalone stream is riding on and changes nothing about the request.
    let tracking = StreamTracking { connections, state: Arc::clone(&state) };
    // **Merged before both layers below, so it inherits both**: the
    // stream-connection note (which ignores it — see [`view_upgrade`]) and,
    // outermost, the origin-and-host refusal. Do not give this route a layer
    // of its own and do not reorder the two that exist; a third layer would be
    // a second place the rule is written, and reordering would put
    // authentication in front of the refusal that is supposed to run first.
    let view = rust_mcp_axum::axum::Router::new()
        .route(VIEW_PATH, rust_mcp_axum::axum::routing::get(view_upgrade))
        .with_state(ViewRoute { auth: view_auth, views, proxy: view_proxy });
    let router = mcp_routes(state, &mount, http_handler)
        .merge(view)
        .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
            tracking,
            note_streaming_connection,
        ))
        // **Outermost, so it runs first and covers every route this process
        // serves** — including the authorization-server routes, which the SDK
        // dispatches on `compose(&[], ..)` and which therefore carry none of
        // the chain above. Applied here rather than pushed onto `middlewares`
        // because that chain is reachable only from the transport handlers;
        // see [`refuse_page_originated`] and [`PUBLIC_DISCOVERY_PATHS`].
        //
        // `RefuseOriginHeader` and `DnsRebindProtector` stay in the SDK chain
        // as well. This is defence in depth, not a replacement: `/mcp` is now
        // refused twice, and a future SDK that stopped composing the chain
        // would cost this browser nothing.
        .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
            admitted,
            refuse_page_originated,
        ));
    (router, directory)
}

/// The complete set of authorities this server answers to: the address it
/// bound, and the origin it advertises.
///
/// **Two entries, and never more.** The set is *enumerated from configuration*
/// before the first connection is accepted. Nothing grows it, and in
/// particular no request does — see [`crate::oauth::AdvertisedIdentity`] for
/// why reading the advertised host off a request header would be threat
/// T-05-09 rather than a shortcut.
///
/// Deduplicated when the two coincide, which is the no-advertised-URL case and
/// therefore the default: with nothing configured this returns exactly the
/// one-element list Phase 4 built, so the default path did not move.
///
/// The second entry is what makes a proxied request admissible at all.
/// `05-01-SPIKE.md` measured one arriving with `Host: <tailnet-name>:<port>` —
/// the client's own name and the proxy's port, not the loopback target — so a
/// server that admitted only the bound address would refuse every one of them.
/// Had the proxy rewritten `Host` to the loopback target instead, this entry
/// would be unnecessary for admission and still correct to carry: what a proxy
/// rewrites is not a property this browser controls, and admitting the name
/// this browser itself publishes is not a widening.
fn admitted_hosts(bound: &str, identity: &AdvertisedIdentity) -> Vec<String> {
    let mut hosts = vec![bound.to_owned()];
    if !identity.authority().eq_ignore_ascii_case(bound) {
        hosts.push(identity.authority().to_owned());
    }
    hosts
}

/// Refuse anything page-originated or misaddressed, on every route (T-2).
///
/// The same two questions `/mcp` was already asked, asked once for the whole
/// process instead of once per transport handler:
///
/// 1. **Does the request carry an `Origin` header?** A legitimate MCP client
///    is a program, and programs do not send one. A page always does. See
///    [`RefuseOriginHeader`], whose reasoning this shares and whose place in
///    the SDK chain it does not take.
/// 2. **Does `Host` name one of the two authorities this server answers to?**
///    This is the DNS rebinding answer: a page served from
///    `http://rebind.evil:PORT/` whose A record has been re-pointed at
///    loopback is *same-origin* with this server as far as the browser is
///    concerned, so it can read every response it gets — but it cannot change
///    the `Host` header the browser sends, and that header still says
///    `rebind.evil:PORT`.
///
/// **There are two admitted names rather than one, and the reason is
/// D-05-03.** A reverse proxy in front of a loopback listener means the
/// address a client addresses and the address this process bound are
/// legitimately different strings; refusing the former would refuse every
/// proxied request, which is exactly what this server would do today
/// (`05-01-SPIKE.md` measured the `Host` a proxied request carries). So the
/// set is the bound address plus the advertised authority, deduplicated when
/// they coincide.
///
/// **The set is enumerated from configuration, before the first connection is
/// accepted.** It is never grown by a request, and the header compared below
/// is never a *source* for it — only ever the thing being compared. Reading
/// the advertised host off `Host` would let a local page choose the identity
/// this browser publishes, which is threat T-05-09. See [`admitted_hosts`].
///
/// A missing `Host` is a refusal, not a pass, which is the SDK's rule too.
/// The one consequence worth naming: a client speaking HTTP/2 with prior
/// knowledge sends `:authority` and no `Host`, and is refused here. That is
/// already true of `/mcp` through `DnsRebindProtector`, so this makes the
/// server uniform rather than newly strict.
///
/// [`PUBLIC_DISCOVERY_PATHS`] is the whole of the exemption, and the reason
/// it exists is written there.
///
/// **The refusal says nothing.** No header value is echoed, nothing
/// distinguishes "wrong Host" from "had an Origin", and the log line names the
/// path and no attacker-chosen value.
async fn refuse_page_originated(
    rust_mcp_axum::axum::extract::State(admitted): rust_mcp_axum::axum::extract::State<
        Arc<[String]>,
    >,
    request: rust_mcp_axum::axum::extract::Request,
    next: rust_mcp_axum::axum::middleware::Next,
) -> rust_mcp_axum::axum::response::Response {
    if !is_public_discovery(request.uri().path()) {
        let headers = request.headers();
        let addressed_here = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|host| {
                admitted.iter().any(|admitted| host.eq_ignore_ascii_case(admitted))
            });
        if headers.contains_key(header::ORIGIN) || !addressed_here {
            log::debug!(
                "remote access refused a page-originated or misaddressed request to {}",
                request.uri().path()
            );
            return page_originated_refusal();
        }
    }
    next.run(request).await
}

/// The one body [`refuse_page_originated`] ever returns.
///
/// One expression, for the reason `crate::oauth::REFUSAL` is one: two refusals
/// that differ are two bits an attacker did not have. It reads as the
/// `RefuseOriginHeader` message does, because it is the same refusal.
fn page_originated_refusal() -> rust_mcp_axum::axum::response::Response {
    use rust_mcp_axum::axum::response::IntoResponse as _;
    (
        StatusCode::FORBIDDEN,
        "requests carrying an Origin header, or addressed to a host this server is not \
         bound to, are refused: this endpoint is for MCP clients, not for pages",
    )
        .into_response()
}

/// Everything the `/view` route needs, as one axum state value.
#[derive(Clone)]
struct ViewRoute {
    /// The **same** `Arc<dyn AuthProvider>` the SDK's chain was built from.
    /// One provider, so a revocation lands on both surfaces at once.
    auth: Arc<dyn AuthProvider>,
    /// Where an accepted socket registers itself for termination.
    views: Arc<ViewSockets>,
    /// The only route from this thread onto the winit main thread.
    proxy: EventLoopProxy<AppEvent>,
}

/// Connection ids for view sockets, unique for the life of the process.
///
/// A counter rather than a descriptor number or a peer address, for the reason
/// [`PeerConnection`]'s doc gives about both: a recycled number names a
/// different thing a moment later, and a peer address is chosen by the peer.
static NEXT_VIEW_CONNECTION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

fn next_view_connection() -> u64 {
    NEXT_VIEW_CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl ViewRoute {
    /// The verified `client_id` behind an upgrade request, or `None`.
    ///
    /// **This route authenticates in its own right, and that is the whole
    /// point of this function.** The middleware vector `build_router` composes
    /// — the origin refusal, the DNS-rebinding protector,
    /// [`AuthMiddleware`] and the identity note — is handed to
    /// [`McpHttpHandler`], and the SDK composes it **only for the transport
    /// handlers it dispatches**. A route merged into the axum router does not
    /// pass through it. That is not a guess: it is the same sentence
    /// `build_router` already writes about the origin refusal at the layer
    /// below — *"Applied here rather than pushed onto `middlewares` because
    /// that chain is reachable only from the transport handlers"* — and the
    /// half nobody had written down is that it applies verbatim to
    /// `AuthMiddleware`. A view route that assumed inheritance would be
    /// origin-checked and **unauthenticated**, which is the shape of the
    /// defect Phase 4 shipped as CR-01 and found in review (T-05-02).
    ///
    /// **No second audience comparison.** The provider compares the token's
    /// audience byte for byte against [`crate::oauth::canonical_resource`],
    /// so a token minted for a different resource server is already refused by
    /// code this function calls. A comparison here would be a second spelling
    /// of one rule, which is exactly what `canonical_resource`'s doc argues
    /// against — two spellings can disagree, and the one that is wrong is the
    /// one nobody is looking at.
    ///
    /// The scope floor *is* checked here, because it is the resource server's
    /// own rule and `AuthMiddleware` is what would otherwise apply it. Same
    /// provider, same list, same answer.
    async fn verified_client(&self, headers: &HeaderMap) -> Option<String> {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim().to_owned())?;
        let identity = self.auth.verify_token(presented).await.ok()?;
        if let Some(required) = self.auth.required_scopes() {
            let held = identity.scopes.clone().unwrap_or_default();
            if !required.iter().all(|scope| held.iter().any(|have| have == scope)) {
                return None;
            }
        }
        // A socket whose owner cannot be named is one a revoke could never
        // close, so it is refused rather than accepted as nobody's — the same
        // rule `SessionOwner::client_id` states, applied one step earlier
        // because here there is still the option of not accepting at all.
        identity.client_id
    }

    /// Serve one accepted socket until it ends, is closed by a revoke, or the
    /// listener shuts down.
    ///
    /// This task owns the socket and closes it by dropping it. **That is why
    /// [`note_streaming_connection`] does not need to know about this route**:
    /// its file-descriptor duplication exists because the SDK's handlers
    /// rebuild a request from a `HeaderMap` and a `Uri` and lose the
    /// connection, so the only way to end one of their streams is to shut the
    /// underlying socket down from outside. Nothing here is out of reach, so
    /// the asymmetry is considered rather than an omission.
    async fn run(self, mut socket: WebSocket, client_id: String) {
        let connection = next_view_connection();
        let (close_tx, mut close_rx) = oneshot::channel::<()>();
        // Registered before the client is told anything, so there is no window
        // in which a socket is live and a revoke cannot reach it.
        self.views.register(connection, &client_id, close_tx);
        log::info!("view connection {connection} opened by client {client_id}");

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let opened = match crate::view::hello_frame() {
            Some(hello) => socket.send(Message::Binary(Bytes::from(hello))).await.is_ok(),
            None => false,
        };
        if opened {
            // The main thread learns about the connection only once the hello
            // is away, so the first thing a client reads is always the wire
            // version — never a snapshot it has no version to interpret.
            if self
                .proxy
                .send_event(AppEvent::ViewOpened {
                    connection,
                    client_id: client_id.clone(),
                    out: out_tx,
                })
                .is_ok()
            {
                self.pump(&mut socket, &mut out_rx, &mut close_rx, connection).await;
            }
        }

        // Removed here and only here, so a connection that ended on its own
        // leaves nothing tracked. `remove` is a no-op for a socket a revoke
        // already took out of the table, which is the ordinary race and not a
        // failure.
        self.views.remove(connection);
        let _ = self.proxy.send_event(AppEvent::ViewClosed { connection });
        let _ = socket.send(Message::Close(None)).await;
        log::debug!("view connection {connection} ended");
    }

    /// The connection's life: outbound frames out, inbound frames onto the
    /// main thread, and three ways to stop.
    ///
    /// The select's futures are dropped at the end of the `let`, which is what
    /// lets the body below use the socket again — a send inside a select arm
    /// would be a second mutable borrow of it.
    async fn pump(
        &self,
        socket: &mut WebSocket,
        out_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        close_rx: &mut oneshot::Receiver<()>,
        connection: u64,
    ) {
        loop {
            let step = tokio::select! {
                _ = &mut *close_rx => ViewStep::Terminated,
                outgoing = out_rx.recv() => match outgoing {
                    Some(frame) => ViewStep::Send(frame),
                    // The main thread dropped this connection's channel.
                    None => ViewStep::Ended,
                },
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Binary(frame))) => ViewStep::Received(frame.to_vec()),
                    // Ping and pong are answered by axum itself; nothing on
                    // this wire is text, so a text message is a client
                    // speaking a protocol this server does not, and the
                    // connection ends rather than being guessed at (T-05-11).
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => ViewStep::Idle,
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Close(_))) | None => {
                        ViewStep::Ended
                    },
                    Some(Err(_)) => ViewStep::Ended,
                },
            };
            match step {
                ViewStep::Idle => {},
                ViewStep::Send(frame) => {
                    if socket.send(Message::Binary(Bytes::from(frame))).await.is_err() {
                        return;
                    }
                },
                ViewStep::Received(frame) => {
                    if self
                        .proxy
                        .send_event(AppEvent::ViewMessage { connection, frame })
                        .is_err()
                    {
                        return;
                    }
                },
                ViewStep::Terminated => {
                    log::info!("view connection {connection} closed by a revoke or shutdown");
                    return;
                },
                ViewStep::Ended => return,
            }
        }
    }
}

/// One turn of a view connection's loop, named so the socket's borrow ends
/// with the `select!` that produced it.
enum ViewStep {
    /// Nothing to do; go round again.
    Idle,
    /// A frame from the main thread, to be written out.
    Send(Vec<u8>),
    /// A frame from the client, to be handed to the main thread.
    Received(Vec<u8>),
    /// A revoke or a shutdown fired this connection's close handle.
    Terminated,
    /// The connection is over, however it got there.
    Ended,
}

/// `GET /view` — the remote view channel's WebSocket upgrade.
///
/// Two gates, in this order, and neither is this handler's own invention:
///
/// 1. The **outermost axum layer** has already refused the request if it
///    carried an `Origin` header or named a host this server neither bound nor
///    advertises. The route is merged inside that layer precisely so it
///    inherits it — see [`refuse_page_originated`] and [`VIEW_PATH`].
/// 2. The **bearer token**, checked here by a direct call to the shared
///    provider, because the SDK's middleware chain does not cover a merged
///    axum route — see [`ViewRoute::verified_client`], which is the whole
///    argument.
///
/// Every failure produces [`view_refusal`], one expression: an absent
/// credential, an unknown token, a revoked client, a token minted for another
/// resource and a token missing the scope floor are indistinguishable from
/// outside.
///
/// `WebSocketUpgrade` extracts last, so a request that is not an upgrade is
/// turned away by axum's own rejection before this body runs. That costs
/// nothing here: every credential-bearing shape reaches the check.
async fn view_upgrade(
    State(route): State<ViewRoute>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> rust_mcp_axum::axum::response::Response {
    let Some(client_id) = route.verified_client(&headers).await else {
        log::debug!("remote access refused an unauthenticated view upgrade");
        return view_refusal();
    };
    upgrade.on_upgrade(move |socket| route.run(socket, client_id))
}

/// The one body a refused view upgrade ever returns.
///
/// One expression, for the reason [`page_originated_refusal`] is one and
/// `crate::oauth::REFUSAL` is one: two refusals that differ are two bits an
/// attacker did not have. In particular an upgrade with *no* credential and an
/// upgrade with a token that was never issued get the same status and the same
/// bytes, so the endpoint is not an oracle for which tokens exist.
fn view_refusal() -> rust_mcp_axum::axum::response::Response {
    use rust_mcp_axum::axum::response::IntoResponse as _;
    (StatusCode::UNAUTHORIZED, "the presented credential is not accepted").into_response()
}

/// Note which connection a standalone event stream is riding on.
///
/// Runs on every request and acts on almost none of them. **Two shapes open a
/// stream, and they learn their session id at opposite ends of the request:**
///
/// - `GET` [`MCP_PATH`] naming a session in `Mcp-Session-Id`. The client
///   already knows the id, so it is read off the request.
/// - `GET` [`SSE_PATH`], the legacy transport, which mints its session while
///   being served and returns the id only inside the response body. That is
///   what [`RecordingIds`] and the scope below exist for — without them this
///   middleware gated on `path == MCP_PATH` and a revoked client's `/sse`
///   stream was never closed (CR-02).
///
/// Waiting for the response is deliberate in both cases — the head is ready
/// before the body streams, so the connection is still there to be noted, and
/// a request that was refused has no stream to close.
///
/// [`VIEW_PATH`] is deliberately **not** a third shape, and the two gates
/// below are exact-path comparisons so it cannot become one by accident. A
/// view socket needs nothing from this mechanism: the task serving it owns its
/// own socket and closes it by dropping it, where the SDK's handlers rebuild a
/// request from a `HeaderMap` and a `Uri` and lose the connection entirely.
/// See [`ViewRoute::run`].
///
/// **The connection is the listener's own, never the client's word for it.**
/// It comes from `ConnectInfo`, which axum computes from the accepted socket;
/// nothing a caller sends can reach it. That matters because the value decides
/// which connection a later revoke tears down.
async fn note_streaming_connection(
    rust_mcp_axum::axum::extract::State(tracking): rust_mcp_axum::axum::extract::State<
        StreamTracking,
    >,
    request: rust_mcp_axum::axum::extract::Request,
    next: rust_mcp_axum::axum::middleware::Next,
) -> rust_mcp_axum::axum::response::Response {
    let get = request.method() == Method::GET;
    let streaming = get && request.uri().path() == MCP_PATH;
    let sse_handshake = get && request.uri().path() == SSE_PATH;
    let named_session = request
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let connection = request
        .extensions()
        .get::<rust_mcp_axum::axum::extract::ConnectInfo<PeerConnection>>()
        .map(|info| info.0);
    // The scope is entered only for the legacy handshake, so nothing else's
    // session id or identity is ever recorded — a Streamable-HTTP
    // `initialize` mints a session too, and it is named by its own client on
    // every later request and given its identity by the SDK itself.
    let handshake: Arc<Mutex<SseHandshake>> = Arc::default();
    let response = match sse_handshake {
        true => SSE_HANDSHAKE.scope(Arc::clone(&handshake), next.run(request)).await,
        false => next.run(request).await,
    };
    if response.status() != StatusCode::OK {
        return response;
    }
    let (minted, identity) = match handshake.lock() {
        Ok(handshake) => (handshake.session_id.clone(), handshake.identity.clone()),
        Err(_) => (None, None),
    };
    let session_id = match (streaming, sse_handshake) {
        (true, _) => named_session,
        // Collected from this request's own scope: it is per task, and both
        // writes happened inline inside `next.run` above.
        (_, true) => minted,
        _ => None,
    };
    let Some(session_id) = session_id else { return response };

    // **Give the `/sse` session the identity the SDK did not.** Without this
    // the session's `auth_info` is `None` for its whole life, which makes it
    // unselectable by a revoke — an unidentified session is nobody's, and
    // deliberately so — and makes its tool calls arrive as
    // `UNVERIFIED_CLIENT`. The value is the one `AuthMiddleware` verified for
    // this same request, on this same task, applied to the session that same
    // request minted; there is no path by which another client's identity can
    // reach here. See [`NoteVerifiedIdentity`].
    if sse_handshake {
        match (identity, tracking.state.session_store.get(&session_id).await) {
            (Some(identity), Some(runtime)) => {
                runtime.update_auth_info(Some(identity)).await;
            },
            // Degrade toward *deny*: with no identity the session stays
            // nobody's, which costs its owner a revoke that closes the stream
            // and costs no other client anything. Never the other way round.
            _ => log::warn!(
                "a legacy /sse session carries no verified identity; it will not be \
                 closed by a revoke"
            ),
        }
    }

    if let Some(connection) = connection {
        // The live list is read after the handler, so a session this request
        // created is in it. This is the prune CR-03 is about: it is what every
        // held descriptor is checked against, and it runs on the one path a
        // stream always takes rather than on a revoke that may never happen.
        let live = tracking.state.session_store.keys().await;
        tracking.connections.attach(&session_id, connection, &live);
    }
    response
}

/// What [`note_streaming_connection`] needs to do its two jobs.
///
/// The table it writes, and the session store it prunes that table against.
/// One value rather than two because `from_fn_with_state` takes one, and a
/// named struct rather than a tuple because "the second field is the thing
/// that bounds the first" is worth saying out loud.
#[derive(Clone)]
struct StreamTracking {
    connections: Arc<Connections>,
    state: Arc<McpAppState>,
}

/// What this server tells a client about itself.
///
/// Deliberately the same description and instructions the stdio binary gives:
/// it is the same server, reached a different way, and a client that got a
/// different answer depending on transport would be right to be confused.
fn server_details() -> InitializeResult {
    InitializeResult {
        server_info: Implementation {
            name: "talaria".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: Some("Talaria browser".into()),
            description: Some(
                "Drive the user's Talaria browser: open tabs, navigate, run JavaScript, \
                 take screenshots, read matching stored credentials, download files."
                    .into(),
            ),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Tabs you open belong to your agent session and appear in the browser's \
             Agents view, where the user can watch and take over at any time. Prefer \
             `evaluate` (arbitrary JS in the page) for interaction; there are no \
             separate click/type tools. If a page needs a login or challenge, ask the \
             user to take over in the Talaria window rather than trying to bypass it."
                .into(),
        ),
        // The shared constant, never the SDK's default and never a second
        // literal: see `talaria_mcp::PROTOCOL_VERSION`.
        protocol_version: talaria_mcp::PROTOCOL_VERSION.into(),
    }
}

/// The MCP handler this listener serves — the same tool surface the stdio
/// binary serves, because it is the same `TalariaTools` and the same
/// `dispatch`, linked rather than redefined.
struct Handler {
    sink: EventLoopSink,
}

#[async_trait::async_trait]
impl ServerHandler for Handler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult { meta: None, next_cursor: None, tools: TalariaTools::tools() })
    }

    /// The tab-owner label a tool call carries is the **verified** `client_id`
    /// from the token that was presented, not the name the peer asked to be
    /// called.
    ///
    /// This is the first transport where the shell knows who is calling, and
    /// spending that knowledge here is the difference this phase buys.
    /// `runtime.client_info()` — the `clientInfo.name` an MCP client sends in
    /// `initialize` — is self-asserted and anything can claim to be anything;
    /// `client_id` was minted by this browser at registration and approved by
    /// a human. The Agents view still labels *stdio* sessions with the
    /// self-asserted `Hello` string, because over that transport there is
    /// nothing better to use. The two are deliberately different, and a reader
    /// comparing the two views should expect an opaque id on one side and a
    /// friendly name on the other.
    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        // The `AuthInfo` the auth middleware produced when this session was
        // created. Absent only if a session existed without ever having been
        // authenticated, which the middleware makes unreachable — answered
        // with a label that claims nothing rather than falling back to the
        // self-asserted name, because a fallback to attacker-chosen input is
        // how a check quietly becomes optional.
        let client = runtime
            .auth_info_cloned()
            .await
            .and_then(|info| info.client_id)
            .unwrap_or_else(|| UNVERIFIED_CLIENT.to_owned());
        let tool: TalariaTools = TalariaTools::try_from(params).map_err(CallToolError::new)?;
        dispatch(&self.sink, &client, tool).await
    }
}

/// The in-process hop: a `Command` onto the winit main thread and back.
///
/// This is [`crate::control`]'s request round trip minus the socket framing.
/// It must **never** be implemented by connecting to the shell's own Unix
/// control socket: that would work, and it would double every round trip,
/// re-enter the peer-credential check against this process itself, and put a
/// self-deadlock within reach.
struct EventLoopSink {
    /// `Shared` is `Rc`-based and not `Send`; this proxy is the whole of what
    /// this side of the thread boundary may hold.
    proxy: EventLoopProxy<AppEvent>,
    session_id: u64,
}

#[async_trait::async_trait]
impl CommandSink for EventLoopSink {
    async fn request(&self, client: &str, command: Command) -> Result<Outcome, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let request = AgentRequest {
            session_id: self.session_id,
            client: client.to_owned(),
            command,
            reply: reply_tx,
        };
        if self.proxy.send_event(AppEvent::Agent(request)).is_err() {
            // The event loop is gone; the browser is shutting down.
            return Err("the browser is no longer accepting commands".to_owned());
        }
        // The same clock the control socket uses, from the same function, so
        // the two transports cannot drift to different timeouts. The three
        // outcomes and their wording are the control socket's too, so an agent
        // gets the same specific message on either transport.
        let timeout_secs = command_timeout_secs();
        let outcome =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), reply_rx).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => Outcome::Error { message: "shell dropped the request".into() },
                Err(_) => Outcome::Error {
                    message: format!("timed out after {timeout_secs}s (script still running?)"),
                },
            };
        Ok(outcome)
    }
}

/// Refuse any request that carries an `Origin` header (T-2).
///
/// A legitimate MCP client is a program, and programs do not send `Origin`. A
/// page always does, on every cross-origin request, whether or not it can read
/// the response. So "has an `Origin` header at all" is a cheap and complete
/// answer to a local web page reaching this endpoint — including the DNS
/// rebinding case, where the `Host` header has been made to look right.
///
/// This sits **on top of** the SDK's host validation rather than instead of
/// it. It cannot be expressed through that mechanism: the SDK's origin control
/// is an allowlist, so configuring it would *require* an `Origin` header on
/// every request and turn every real client away. It is a middleware rather
/// than a check inside the auth provider because a provider only ever sees the
/// token string — and because 04-05 replaces that provider, and this refusal
/// must survive it.
struct RefuseOriginHeader;

#[async_trait::async_trait]
impl Middleware for RefuseOriginHeader {
    async fn handle<'req>(
        &self,
        request: Request<&'req str>,
        state: Arc<McpAppState>,
        next: MiddlewareNext<'req>,
    ) -> McpHttpResult<Response<GenericBody>> {
        if request.headers().contains_key(header::ORIGIN) {
            // The header's *value* is deliberately not echoed back or logged
            // at anything louder than debug: it is attacker-chosen, and the
            // refusal does not need it to be correct.
            log::debug!("remote access refused a request carrying an Origin header");
            return error_response(
                StatusCode::FORBIDDEN,
                SdkError::bad_request().with_message(
                    "requests carrying an Origin header are refused: this endpoint is for \
                     MCP clients, not for pages",
                ),
            );
        }
        next(request, state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The origin `05-01-SPIKE.md` measured a Serve-proxied request actually
    /// arriving at — the tailnet name *and the proxy's own port*, which is why
    /// an advertised identity has to be able to carry a port at all.
    const ADVERTISED: &str = "https://thinkpad.tailcd3cc6.ts.net:8449";

    /// A session directory with no server behind it.
    ///
    /// The point of the trait: every property worth asserting about
    /// revocation's stream half is *which* sessions get ended, and that is
    /// decidable without a bound port, a runtime handle or an MCP client. What
    /// the real directory adds — the SDK's own session store and its own
    /// session-delete path — is exercised end to end by
    /// `tests/e2e/revocation_test.py`, which asserts the closure from the
    /// client end rather than from this bookkeeping.
    #[derive(Default)]
    struct FakeSessions {
        open: Mutex<Vec<SessionOwner>>,
        ended: Mutex<Vec<String>>,
    }

    impl FakeSessions {
        /// A directory holding one session per `(session_id, client_id)` pair.
        fn with(sessions: &[(&str, Option<&str>)]) -> Arc<Self> {
            let directory = Self::default();
            if let Ok(mut open) = directory.open.lock() {
                for (session_id, client_id) in sessions {
                    open.push(SessionOwner {
                        session_id: (*session_id).to_owned(),
                        client_id: client_id.map(str::to_owned),
                    });
                }
            }
            Arc::new(directory)
        }

        /// A stream ending on its own, the way a completed request's does.
        fn completes(&self, session_id: &str) {
            if let Ok(mut open) = self.open.lock() {
                open.retain(|session| session.session_id != session_id);
            }
        }

        /// Which sessions were ended, in the order they were ended.
        fn ended(&self) -> Vec<String> {
            self.ended.lock().map(|ended| ended.clone()).unwrap_or_default()
        }

        /// Which sessions are still open.
        fn still_open(&self) -> Vec<String> {
            self.open
                .lock()
                .map(|open| open.iter().map(|session| session.session_id.clone()).collect())
                .unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl SessionDirectory for FakeSessions {
        async fn sessions(&self) -> Vec<SessionOwner> {
            self.open.lock().map(|open| open.clone()).unwrap_or_default()
        }

        async fn end(&self, session_id: &str) {
            if let Ok(mut ended) = self.ended.lock() {
                ended.push(session_id.to_owned());
            }
            self.completes(session_id);
        }
    }

    /// A descriptor a test may duplicate and shut down.
    ///
    /// A bound listening socket rather than a connected one: `dup(2)` is what
    /// the mechanism under test calls and it is indifferent to the socket's
    /// state, while `shutdown(2)` on a listener answers `ENOTCONN` — which
    /// [`OwnedSocket::disconnect`] already ignores, because a peer that
    /// vanished first is not a failure.
    #[cfg(unix)]
    fn a_real_connection() -> (std::net::TcpListener, PeerConnection) {
        use std::os::fd::AsRawFd as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let peer = listener.local_addr().expect("a bound address");
        let fd = listener.as_raw_fd();
        (listener, PeerConnection { peer, fd })
    }

    /// WR-01 and WR-02, which were one table's two failure modes.
    ///
    /// **WR-01:** `attach` succeeds with no prior bookkeeping of any kind.
    /// There is no accepted-connection table left to fill, so there is nothing
    /// an unauthenticated peer can flush to make a legitimate agent's stream
    /// invisible to a later revoke.
    ///
    /// **WR-02:** the connection is *read*, never consumed, so the second and
    /// every later standalone stream over one keep-alive connection is tracked
    /// as well as the first. An event-stream reconnect over a live connection
    /// is the ordinary case; under the old waiting room every one of them
    /// after the first survived a revoke.
    #[cfg(unix)]
    #[test]
    fn every_stream_on_one_connection_is_tracked_not_only_the_first() {
        let (_listener, connection) = a_real_connection();
        let connections = Connections::default();

        let live = vec!["s-1".to_owned(), "s-2".to_owned()];
        connections.attach("s-1", connection, &live);
        connections.attach("s-2", connection, &live);

        assert!(connections.disconnect("s-1"), "the first stream was not tracked");
        assert!(connections.disconnect("s-2"), "the second stream on the same connection \
                                                was not tracked");
        assert!(!connections.disconnect("s-1"), "a closed stream stayed in the table");
    }

    /// CR-03: the table is bounded by the session store, on the path that runs.
    ///
    /// A well-behaved MCP client that reconnects a dropped stream is all it
    /// took. Each reconnect gets a **new** session id from the SDK's
    /// generator, so nothing overwrites the previous entry, and the
    /// `OwnedSocket` it holds is released only by a `Drop` — which used to
    /// happen only on a revoke or a shutdown, neither of which an ordinary
    /// session performs. One leaked descriptor per reconnect, in the process
    /// that also holds the credential vault.
    ///
    /// Both halves are asserted: a session the store has dropped is released,
    /// and a session the store still has is **not** — a prune that took live
    /// entries would trade a descriptor leak for a revoke that closes nothing.
    #[cfg(unix)]
    #[test]
    fn a_reconnecting_stream_releases_the_descriptor_the_last_one_held() {
        let (_listener, connection) = a_real_connection();
        let connections = Connections::default();

        for attempt in 0..64u32 {
            let session = format!("s-{attempt}");
            // The store's view: the previous session ended when its stream did.
            connections.attach(&session, connection, std::slice::from_ref(&session));
            assert_eq!(
                connections.tracked(),
                1,
                "reconnect {attempt} left the previous stream's descriptor behind"
            );
        }

        // And a session the store still holds survives the next attach.
        let live = vec!["s-63".to_owned(), "s-64".to_owned()];
        connections.attach("s-64", connection, &live);
        assert_eq!(connections.tracked(), 2, "a live session's stream was pruned");
        assert!(connections.disconnect("s-63"), "the surviving stream is not closable");
    }

    /// **The allowlist is the boundary, so the allowlist is what is pinned.**
    ///
    /// A path added to this server and *not* added to
    /// [`PUBLIC_DISCOVERY_PATHS`] is refused a page-originated or misaddressed
    /// request. This asserts both halves: that the two documents a client must
    /// fetch before it has a credential stay public, and that every other path
    /// this process serves — the four credential endpoints most of all — does
    /// not. Widening the allowlist fails here.
    #[test]
    fn discovery_is_public_and_every_other_route_is_origin_and_host_checked() {
        for path in [OAUTH_PROTECTED_RESOURCE_BASE, WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER] {
            assert!(is_public_discovery(path), "{path} lost its discovery exemption");
        }
        for path in [
            "/register",
            "/authorize",
            "/authorize/status",
            "/token",
            "/revoke",
            MCP_PATH,
            SSE_PATH,
            // Named explicitly, not covered by accident. The view channel is
            // a WebSocket, and a WebSocket is exempt from the same-origin
            // policy and has no preflight — origin enforcement is entirely
            // this server's job. Exempting this path would reopen
            // cross-site WebSocket hijacking (T-05-01), and this line is what
            // makes such a change fail a test rather than pass silently.
            VIEW_PATH,
            "/messages",
            "/",
            "/.well-known",
            // Not a discovery document, and a `starts_with` would say it was.
            "/.well-known/oauth-protected-resource/../token",
            "/.well-known/oauth-authorization-server/token",
        ] {
            assert!(
                !is_public_discovery(path),
                "{path} is exempt from the Origin and Host refusal"
            );
        }
        // And the set of names those checked routes admit: exactly two when an
        // identity is advertised, exactly one when none is, and never a third.
        let bound = "127.0.0.1:8779";
        let advertised = AdvertisedIdentity::new(bound, Some(ADVERTISED));
        let admitted = admitted_hosts(bound, &advertised);
        assert_eq!(
            admitted,
            vec![bound.to_owned(), "thinkpad.tailcd3cc6.ts.net:8449".to_owned()],
            "the host allowlist is not the two names the advertised identity implies"
        );
        // A third plausible name — the same tailnet node on the port an
        // unrelated service is already served at — is not in the set.
        assert!(
            !admitted.iter().any(|name| name == "thinkpad.tailcd3cc6.ts.net:8443"),
            "a name nobody configured is admitted: {admitted:?}"
        );
    }

    /// **The zero-regression half.** With nothing advertised, the allowlist is
    /// exactly the one entry Phase 4 built — deduplicated rather than doubled,
    /// so the default path did not move.
    #[test]
    fn with_no_advertised_url_the_host_allowlist_is_the_one_bound_address() {
        let bound = "127.0.0.1:8779";
        let admitted = admitted_hosts(bound, &AdvertisedIdentity::new(bound, None));
        assert_eq!(admitted, vec![bound.to_owned()]);
    }

    /// The allowlist is enumerated from configuration and closed. Neither the
    /// tailnet name without its Serve port, nor a rebound DNS name, nor a
    /// second tailnet node is in it — which is what stands between a rebound
    /// name and the authorization server (T-05-09).
    #[test]
    fn the_host_allowlist_admits_two_names_and_refuses_every_other() {
        let bound = "127.0.0.1:8779";
        let admitted = admitted_hosts(bound, &AdvertisedIdentity::new(bound, Some(ADVERTISED)));
        assert_eq!(admitted.len(), 2);
        for refused in [
            // The advertised name without the proxy's port: a different
            // authority, and the spike measured the port as present.
            "thinkpad.tailcd3cc6.ts.net",
            "thinkpad.tailcd3cc6.ts.net:8443",
            "rebind.evil:8779",
            "localhost:8779",
            "127.0.0.1:8780",
            "",
        ] {
            assert!(
                !admitted.iter().any(|name| name.eq_ignore_ascii_case(refused)),
                "{refused:?} is admitted"
            );
        }
    }

    /// A close handle, and a way to ask whether it fired.
    fn a_view_socket() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    /// Whether a socket's task would have seen its close handle fire.
    ///
    /// `try_recv` on a `oneshot` distinguishes the three states this needs:
    /// a value (closed), empty (still open), and a dropped sender — which
    /// also means closed, because dropping the handle is what a table that
    /// forgot the socket would do and the task treats either as an ending.
    fn was_closed(receiver: &mut oneshot::Receiver<()>) -> bool {
        !matches!(receiver.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty))
    }

    /// The registry's second half tracks a socket and lets go of it again.
    ///
    /// A completed connection must leave **nothing** behind: this table is the
    /// set of sockets that are open, not a log of sockets that happened, and
    /// the difference is a process that also holds the credential vault
    /// accumulating handles for the life of the browser.
    #[test]
    fn a_view_socket_that_ends_normally_leaves_nothing_tracked() {
        let views = ViewSockets::default();
        let (close, mut fired) = a_view_socket();
        views.register(1, "client-a", close);
        assert_eq!(views.tracked(), 1);
        assert!(views.remove(1), "the socket was not tracked");
        assert_eq!(views.tracked(), 0);
        assert!(!views.remove(1), "a socket that already ended was removed twice");
        // Removing is not closing: the connection ended under its own power.
        assert!(was_closed(&mut fired), "the handle outlived the table entry");
    }

    /// The property a revoke needs: one client's sockets close and no other
    /// client's does.
    ///
    /// This is the half `terminate_matching` cannot reach at all. It walks the
    /// SDK's session directory, and a WebSocket is not an SDK session — so a
    /// revoke that only did that would close a client's streams and leave its
    /// viewer watching (T-05-07).
    #[test]
    fn terminating_a_client_closes_its_view_sockets_and_no_others() {
        let views = ViewSockets::default();
        let (first, mut first_fired) = a_view_socket();
        let (second, mut second_fired) = a_view_socket();
        let (other, mut other_fired) = a_view_socket();
        views.register(1, "client-a", first);
        views.register(2, "client-a", second);
        views.register(3, "client-b", other);

        assert_eq!(views.close_client("client-a"), 2);
        assert!(was_closed(&mut first_fired));
        assert!(was_closed(&mut second_fired));
        assert!(!was_closed(&mut other_fired), "another client's view socket was closed");
        assert_eq!(views.tracked(), 1, "the surviving socket left the table");

        // And a second revoke of the same client closes nothing, because the
        // handles were *taken* rather than copied.
        assert_eq!(views.close_client("client-a"), 0);
    }

    /// Revoking a client with no view socket is not an error.
    #[test]
    fn terminating_a_client_with_no_view_socket_closes_nothing() {
        let views = ViewSockets::default();
        let (close, mut fired) = a_view_socket();
        views.register(1, "client-a", close);
        assert_eq!(views.close_client("client-z"), 0);
        assert!(!was_closed(&mut fired));
        assert_eq!(views.tracked(), 1);
    }

    /// The listener's way out closes every socket, whoever owns it.
    #[test]
    fn shutting_the_listener_down_closes_every_view_socket() {
        let views = ViewSockets::default();
        let (first, mut first_fired) = a_view_socket();
        let (second, mut second_fired) = a_view_socket();
        views.register(1, "client-a", first);
        views.register(2, "client-b", second);
        assert_eq!(views.close_all(), 2);
        assert!(was_closed(&mut first_fired));
        assert!(was_closed(&mut second_fired));
        assert_eq!(views.tracked(), 0);
    }

    /// A revoke reaches **both** halves through one call, which is what stops
    /// a caller from performing one and forgetting the other.
    #[tokio::test]
    async fn one_revoke_closes_the_clients_streams_and_its_view_sockets() {
        let directory = FakeSessions::with(&[("s-1", Some("client-a")), ("s-2", Some("client-b"))]);
        let views = Arc::new(ViewSockets::default());
        let (close, mut fired) = a_view_socket();
        let (other, mut other_fired) = a_view_socket();
        views.register(1, "client-a", close);
        views.register(2, "client-b", other);

        let registry = StreamRegistry::default();
        registry.install(
            directory.clone(),
            Arc::clone(&views),
            tokio::runtime::Handle::current(),
        );
        registry.terminate_client("client-a");
        // The view half is synchronous; the session half is spawned.
        assert!(was_closed(&mut fired), "the revoked client's view socket stayed open");
        assert!(!was_closed(&mut other_fired));
        for _ in 0..16 {
            tokio::task::yield_now().await;
            if !directory.ended().is_empty() {
                break;
            }
        }
        assert_eq!(directory.ended(), vec!["s-1".to_owned()]);
    }

    #[tokio::test]
    async fn a_revoke_ends_every_stream_of_that_client_and_no_other() {
        let directory = FakeSessions::with(&[
            ("s-1", Some("client-a")),
            ("s-2", Some("client-b")),
            ("s-3", Some("client-a")),
        ]);
        let closed = terminate_matching(directory.as_ref(), Some("client-a")).await;
        assert_eq!(closed, 2);
        assert_eq!(directory.ended(), vec!["s-1".to_owned(), "s-3".to_owned()]);
        assert_eq!(directory.still_open(), vec!["s-2".to_owned()]);
    }

    #[tokio::test]
    async fn a_stream_that_ended_normally_is_not_there_for_a_revoke_to_end() {
        // The registry does not grow with every completed request because it
        // is not a registry of requests: it is the live session table, and a
        // session that finished has removed itself from it.
        let directory = FakeSessions::with(&[("s-1", Some("client-a")), ("s-2", Some("client-a"))]);
        directory.completes("s-1");
        let closed = terminate_matching(directory.as_ref(), Some("client-a")).await;
        assert_eq!(closed, 1);
        assert_eq!(directory.ended(), vec!["s-2".to_owned()]);
        assert!(directory.still_open().is_empty());
    }

    #[tokio::test]
    async fn revoking_a_client_with_no_open_stream_ends_nothing_and_is_not_an_error() {
        let directory = FakeSessions::with(&[("s-1", Some("client-a"))]);
        assert_eq!(terminate_matching(directory.as_ref(), Some("client-z")).await, 0);
        assert!(directory.ended().is_empty());
        assert_eq!(directory.still_open(), vec!["s-1".to_owned()]);
    }

    #[tokio::test]
    async fn shutting_the_listener_down_ends_every_stream() {
        let directory = FakeSessions::with(&[
            ("s-1", Some("client-a")),
            ("s-2", Some("client-b")),
            // Including one nothing could name in a revoke.
            ("s-3", None),
        ]);
        assert_eq!(terminate_matching(directory.as_ref(), None).await, 3);
        assert!(directory.still_open().is_empty());
    }

    #[tokio::test]
    async fn a_session_with_no_verified_client_is_nobodys() {
        // Treating an absent identity as a match would let one revoke close
        // every other agent's stream.
        let directory = FakeSessions::with(&[("s-1", None), ("s-2", Some("client-a"))]);
        assert_eq!(terminate_matching(directory.as_ref(), Some("client-a")).await, 1);
        assert_eq!(directory.still_open(), vec!["s-1".to_owned()]);
    }

    #[tokio::test]
    async fn an_installed_registry_hands_the_work_to_the_listeners_runtime() {
        let directory = FakeSessions::with(&[("s-1", Some("client-a")), ("s-2", Some("client-b"))]);
        let registry = StreamRegistry::default();
        registry.install(
            directory.clone(),
            Arc::new(ViewSockets::default()),
            tokio::runtime::Handle::current(),
        );
        registry.terminate_client("client-a");
        // Fire and forget: the human's feedback is the row disappearing, which
        // has already happened by the time this is called. Yielding is what
        // lets the spawned task run on this single-threaded test runtime.
        for _ in 0..16 {
            tokio::task::yield_now().await;
            if !directory.ended().is_empty() {
                break;
            }
        }
        assert_eq!(directory.ended(), vec!["s-1".to_owned()]);
        assert_eq!(directory.still_open(), vec!["s-2".to_owned()]);
    }

    #[tokio::test]
    async fn a_registry_with_no_listener_terminates_nothing_and_does_not_panic() {
        // The window between `spawn` returning and `serve` binding, and every
        // moment after the listener stops. A revoke here still removes the
        // client from the store — the half that refuses the next request —
        // and there is no stream to close because none can exist.
        let registry = StreamRegistry::default();
        registry.terminate_client("client-a");
        let directory = FakeSessions::with(&[("s-1", Some("client-a"))]);
        registry.install(
            directory.clone(),
            Arc::new(ViewSockets::default()),
            tokio::runtime::Handle::current(),
        );
        registry.clear();
        registry.terminate_client("client-a");
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(directory.ended().is_empty());
    }
}
