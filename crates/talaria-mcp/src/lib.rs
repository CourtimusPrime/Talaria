//! The MCP tool surface, plus the seam between that surface and a running
//! Talaria shell.
//!
//! This crate defines the nine MCP tools exactly once — their structs, their
//! JSON schemas, the `TalariaTools` enum and `dispatch` — and exposes them as a
//! library so that every transport serves the identical set. Success Criterion
//! 1 of this phase requires an HTTP client to drive *the same* tool surface as
//! the stdio proxy, and a second definition of that surface anywhere is the
//! failure this file exists to prevent: a tool added here is gained by both
//! transports at once, because neither of them owns a copy.
//!
//! There are two ways to reach a running shell — the stdio proxy's Unix
//! control-socket wire (`socket`), and, from 04-03, an in-process hop inside
//! `talaria-shell`. [`CommandSink`] is the only thing the tool surface knows
//! about either of them.

pub mod socket;
pub mod tools;

pub use socket::ShellConnection;
pub use tools::{dispatch, TalariaTools};

/// The MCP protocol revision every Talaria transport advertises.
///
/// D-04-01 locks this phase to the revision `rust-mcp-sdk` 1.0.1 actually
/// implements rather than chasing `2026-07-28`. It is a constant here, in the
/// crate both transports link, rather than a literal repeated at each
/// `server_details` — because the stdio binary and the shell's HTTP listener
/// are two transports of *one* server, and a revision that drifted between
/// them would pass every test in this phase: 04-05's parity assertion compares
/// the tool *surface*, not the negotiated version, so nothing else would
/// notice. One definition means a dependency bump moves both or neither.
pub const PROTOCOL_VERSION: rust_mcp_sdk::schema::ProtocolVersion =
    rust_mcp_sdk::schema::ProtocolVersion::V2025_11_25;

/// One hop: hand a `Command` to a running Talaria shell and return that
/// shell's `Outcome`.
///
/// A sink is deliberately thin. It is *not* a place for retry, batching,
/// queueing, or policy: `dispatch` calls `request` exactly once per tool call
/// and renders whatever comes back. [`ShellConnection`]'s inherent `request`
/// already decides what is safe to re-send — a command the wire accepted is
/// never re-sent — and a sink that retried on top of that would re-create the
/// double-execution bug plan 02-05 fixed.
///
/// Two implementations exist or will exist:
///
/// - [`ShellConnection`], the stdio proxy's control-socket wire — an
///   out-of-process hop over `$XDG_RUNTIME_DIR/talaria.sock`.
/// - From 04-03, an in-process sink in `talaria-shell` that sends an
///   `AppEvent` down the winit `EventLoopProxy` and awaits the `oneshot`.
///
/// The shell's own sink must **never** be implemented by connecting to the
/// shell's own Unix control socket. That would work, and it would double every
/// round trip, re-enter the peer-UID check against itself, and put a
/// self-deadlock within reach (`04-RESEARCH.md`, Pattern 1).
///
/// The `Send + Sync` bounds are load-bearing rather than decorative: the HTTP
/// transport holds a sink behind an `Arc` and calls it across tokio tasks, so a
/// later reader should not simplify them away.
#[async_trait::async_trait]
pub trait CommandSink: Send + Sync {
    /// Send `command` to the shell on behalf of the agent named `client`, and
    /// await its outcome. The error string is human-readable and is surfaced
    /// to the calling agent verbatim.
    async fn request(
        &self,
        client: &str,
        command: talaria_protocol::Command,
    ) -> Result<talaria_protocol::Outcome, String>;
}
