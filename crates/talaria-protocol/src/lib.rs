//! Wire types for Talaria's local control socket.
//!
//! The shell listens on a Unix domain socket (see [`socket_path`]). Clients
//! (the `talaria-mcp` stdio proxy today, the distributed-mode client later)
//! speak newline-delimited JSON: one [`ClientMessage`] per line in, one
//! [`ServerMessage`] per line out. Replies carry the request's `id`; they may
//! arrive out of order because some commands (evaluate, screenshot) complete
//! asynchronously inside the engine.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default socket location: `$XDG_RUNTIME_DIR/talaria.sock`, falling back to
/// `/tmp/talaria-$UID.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("talaria.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from(format!("/tmp/talaria-{uid}.sock"))
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
