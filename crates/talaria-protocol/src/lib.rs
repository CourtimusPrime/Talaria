//! Talaria's shared vocabulary: the types the shell, the `talaria-mcp` stdio
//! proxy, the HTTP transport and the remote view client all speak.
//!
//! This crate deliberately carries no transport. It names the things —
//! [`ClientMessage`], [`Command`], [`Outcome`], [`TabInfo`], [`Event`] — and
//! leaves the carrying to whichever wire is in use:
//!
//! - a Unix domain control socket speaking newline-delimited JSON, one
//!   [`ClientMessage`] per line in and one [`ServerMessage`] per line out (the
//!   [`local`] module here, and `talaria-shell`'s `control` module);
//! - Streamable HTTP for authenticated remote MCP clients (`talaria-shell`'s
//!   `http` module);
//! - a multiplexed binary WebSocket for the remote viewer (the [`wire`] module
//!   here, and the shell's view route).
//!
//! What holds across all three is the correlation discipline: replies carry
//! their request's `id`, and they may arrive out of order because some
//! commands (evaluate, screenshot) complete asynchronously inside the engine.
//! Unsolicited events share the same stream and carry no `id` at all.

// `local` is gated because a Unix path and a Unix user id are not vocabulary:
// they mean nothing to a peer on another machine. See its own header.
#[cfg(unix)]
pub mod local;
pub mod wire;

use serde::{Deserialize, Serialize};

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
    /// **Test hook.** The logical rects of the chrome elements the shell is
    /// currently drawing — the toolbar's controls and the rows of whichever
    /// panel is open — so an e2e suite can click a real widget by name
    /// instead of hardcoding a coordinate that moves whenever the toolbar
    /// gains a button.
    ///
    /// Refused unless the shell was started with `TALARIA_TEST_HOOKS=1`, and
    /// refused the way an unrecognised command is refused: where the chrome
    /// is on screen is not something a production agent has any business
    /// reading. It is a map of the human's own controls, and an agent that
    /// could read it could aim synthetic input at the credentials button, the
    /// bookmark star or a downloads row — the human-only surfaces this whole
    /// phase kept off the tool surface on purpose. It is also a live
    /// description of what the human is looking at right now, which is the
    /// same class of leak as reading their history.
    ///
    /// Deliberately not exposed as an MCP tool, for the same reason: the
    /// control socket carries it for tests, the agent tool surface does not
    /// carry it at all.
    ChromeRects,
}

/// One named chrome element's rectangle, in egui's own logical points with
/// the window's top-left as the origin. Multiply by the
/// [`ResultPayload::ChromeRects::scale`] the reply carries to get physical
/// pixels, which is what a synthetic-input tool wants.
///
/// `name` is stable and structured: `toolbar.credentials`, `history.row.0`,
/// `downloads.open.1`. An indexed name counts from the row the panel draws
/// **first**, and every list panel draws newest-first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromeRect {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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
    /// Reply to [`Command::ChromeRects`]. `scale` is the window's scale
    /// factor, carried alongside rather than pre-multiplied into the rects so
    /// that what is reported is what egui itself laid out.
    ///
    /// Ahead of `Empty {}` because this enum is `untagged`: serde tries the
    /// variants in declaration order, and `Empty {}` ignores unknown fields,
    /// so it matches any object put after it.
    ChromeRects { rects: Vec<ChromeRect>, scale: f64 },
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

    /// The test hook's wire shape, both ways. `ChromeRects` sits inside an
    /// `untagged` payload enum next to `Empty {}`, which matches any object,
    /// so "it deserialises back into the variant it was written from" is the
    /// property that declaration order has to keep true.
    #[test]
    fn chrome_rects_round_trip() {
        let request = ClientMessage::Request { id: 4, command: Command::ChromeRects };
        let json = serde_json::to_string(&request).expect("serializable");
        assert!(json.contains("\"command\":\"chrome_rects\""), "{json}");

        let reply = ServerMessage::Reply {
            id: 4,
            outcome: Outcome::Ok {
                result: ResultPayload::ChromeRects {
                    rects: vec![ChromeRect {
                        name: "toolbar.credentials".into(),
                        x: 388.3,
                        y: 2.0,
                        width: 21.0,
                        height: 18.0,
                    }],
                    scale: 1.0,
                },
            },
        };
        let json = serde_json::to_string(&reply).expect("serializable");
        let back: ServerMessage = serde_json::from_str(&json).expect("deserializable");
        match back {
            ServerMessage::Reply {
                outcome: Outcome::Ok { result: ResultPayload::ChromeRects { rects, scale } },
                ..
            } => {
                assert_eq!(scale, 1.0);
                assert_eq!(rects.len(), 1);
                assert_eq!(rects[0].name, "toolbar.credentials");
                assert_eq!(rects[0].width, 21.0);
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
