//! Wire types for Talaria's local control socket.
//!
//! The shell listens on a Unix domain socket (see [`socket_path`]). Clients
//! (the `talaria-mcp` stdio proxy today, the distributed-mode client later)
//! speak newline-delimited JSON: one [`ClientMessage`] per line in, one
//! [`ServerMessage`] per line out. Replies carry the request's `id`; they may
//! arrive out of order because some commands (evaluate, screenshot) complete
//! asynchronously inside the engine.
//!
//! The socket is owner-only: it lives in a directory only its owner can enter,
//! and the shell refuses to serve if it cannot establish that. Anything that
//! reaches the socket can read the user's credentials, drive their logged-in
//! sessions, and run arbitrary JavaScript in them, so reachability by another
//! local user is itself the vulnerability.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Socket filename inside [`socket_dir`].
const SOCKET_FILE: &str = "talaria.sock";

/// Directory holding the control socket: `$XDG_RUNTIME_DIR` when it is set,
/// otherwise a per-UID `talaria-$UID` directory under the shared temp
/// directory. The fallback is a *directory* rather than a bare socket file so
/// that it can be made owner-only; see [`ensure_socket_dir`].
pub fn socket_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    std::env::temp_dir().join(format!("talaria-{}", current_uid()))
}

/// Default socket location: `$XDG_RUNTIME_DIR/talaria.sock`, falling back to
/// `/tmp/talaria-$UID/talaria.sock`. Always [`socket_dir`] joined with the
/// socket filename, so the two can never drift.
pub fn socket_path() -> PathBuf {
    socket_dir().join(SOCKET_FILE)
}

/// Make sure [`socket_dir`] exists and is owner-only, returning it.
///
/// `$XDG_RUNTIME_DIR` is created and protected at mode 0700 by the OS already,
/// so only the temp fallback is created and chmodded here. The chmod result is
/// checked rather than discarded: a directory we cannot make private is a
/// directory the socket should not live in.
pub fn ensure_socket_dir() -> std::io::Result<PathBuf> {
    let dir = socket_dir();
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

/// The current process's real user id. Exposed so the shell can compare a
/// connecting peer's credential against it without re-declaring the shim below
/// or pulling the libc crate into this deliberately serde-only crate.
pub fn current_uid() -> u32 {
    unsafe { libc_getuid() }
}

// Tiny libc shim so we don't pull the libc crate for one call.
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Must be the first message on a connection. `client` labels the agent
    /// session in the shell's Agents view (e.g. the MCP clientInfo.name).
    Hello { client: String },
    Request {
        id: u64,
        #[serde(flatten)]
        command: Command,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    TabsList,
    TabsOpen { url: String },
    TabsClose { tab_id: u64 },
    TabsFocus { tab_id: u64 },
    Navigate { tab_id: u64, url: String },
    Evaluate { tab_id: u64, script: String },
    /// PNG screenshot of the tab's viewport, base64-encoded in the reply.
    Screenshot { tab_id: u64 },
    /// Credential-vault lookup by domain (autofill/session reuse, per SPEC).
    CookiesRead { domain: String },
    Download { url: String, filename: String },
    /// Open a tab in the *human's* (Me) view and bring the window forward.
    /// Used by a second `talaria` launch to hand its URL to the running
    /// instance (single-instance behaviour); not exposed as an MCP tool.
    OpenForUser { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    HelloAck { session_id: u64 },
    Reply {
        id: u64,
        #[serde(flatten)]
        outcome: Outcome,
    },
    /// Unsolicited event, e.g. a tab's WebContent process crashed (surfaced
    /// to agents per SPEC's crash-recovery decision).
    Event {
        #[serde(flatten)]
        event: Event,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Ok { result: ResultPayload },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResultPayload {
    Tabs { tabs: Vec<TabInfo> },
    Tab { tab: TabInfo },
    Value { value: serde_json::Value },
    Screenshot { png_base64: String, width: u32, height: u32 },
    Credentials { entries: Vec<CredentialEntry> },
    Download { path: String, bytes: u64 },
    Empty {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: u64,
    pub url: String,
    pub title: String,
    /// "me" or the agent client name that owns the tab.
    pub owner: String,
    pub focused: bool,
    pub crashed: bool,
    /// True while the tab's page is still loading (`document.readyState`
    /// != complete). `tabs_open` / `navigate` normally reply only once this
    /// is false; a slow page can make them give up waiting and reply with
    /// `loading: true`, in which case poll `tabs_list`.
    #[serde(default)]
    pub loading: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialEntry {
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<Cookie>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    TabCrashed { tab_id: u64 },
    TabClosed { tab_id: u64 },
    /// A tab appeared in the session without the agent asking for it: a page
    /// it was driving called `window.open`, and the popup was adopted into
    /// the opener's session.
    ///
    /// Only adoption raises this. A tab the agent opened itself is already
    /// named in the reply to its own `tabs_open`, so `opener_tab_id` is the
    /// whole point of the notification — it is the only way to learn which
    /// page produced the tab.
    TabOpened { tab_id: u64, opener_tab_id: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_request() {
        let msg = ClientMessage::Request {
            id: 7,
            command: Command::Evaluate {
                tab_id: 3,
                script: "1+1".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"command\":\"evaluate\""));
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        match back {
            ClientMessage::Request { id: 7, command: Command::Evaluate { tab_id: 3, script } } => {
                assert_eq!(script, "1+1")
            },
            other => panic!("bad round trip: {other:?}"),
        }
    }

    #[test]
    fn reply_shapes() {
        let msg = ServerMessage::Reply {
            id: 1,
            outcome: Outcome::Ok {
                result: ResultPayload::Value { value: serde_json::json!(2) },
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ServerMessage::Reply { id: 1, .. }));
    }
}
