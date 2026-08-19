//! Stdio MCP server for Talaria.
//!
//! A thin proxy: MCP tool calls are translated into control-socket requests
//! against the running Talaria shell. Per the MCP spec (and Talaria's SPEC),
//! a locally-spawned stdio server does not go through OAuth — connection
//! access is implied by the ability to spawn it.

mod socket;
mod tools;

use async_trait::async_trait;
use rust_mcp_sdk::mcp_server::{server_runtime, ServerHandler};
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult, ListToolsResult,
    LoggingLevel, LoggingMessageNotificationParams, PaginatedRequestParams, ProtocolVersion,
    RpcError, ServerCapabilities, ServerCapabilitiesTools,
};
use rust_mcp_sdk::{error::SdkResult, McpServer, StdioTransport, TransportOptions};
use std::sync::Arc;
use talaria_protocol::Event;

use socket::ShellConnection;
use tools::TalariaTools;

struct Handler {
    connection: ShellConnection,
}

#[async_trait]
impl ServerHandler for Handler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: TalariaTools::tools(),
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let client = runtime
            .client_info()
            .map(|info| info.client_info.name.clone())
            .unwrap_or_else(|| "unknown-agent".to_owned());
        let tool: TalariaTools = TalariaTools::try_from(params).map_err(CallToolError::new)?;
        tools::dispatch(&self.connection, &client, tool).await
    }
}

#[tokio::main]
async fn main() -> SdkResult<()> {
    // Unsolicited shell events land here; the drain task below turns each one
    // into an MCP notification.
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let connection = ShellConnection::new(event_tx);

    let server_details = InitializeResult {
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
            // Tab lifecycle events are delivered as `notifications/message`,
            // which a spec-conformant client only accepts from a server that
            // declared `logging`. Without this the notification is protocol
            // noise the client may drop.
            logging: Some(serde_json::Map::new()),
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
        protocol_version: ProtocolVersion::V2025_11_25.into(),
    };

    let transport = StdioTransport::new(TransportOptions::default())?;
    let handler = Handler { connection };
    let server = server_runtime::create_server(rust_mcp_sdk::mcp_server::McpServerOptions {
        server_details,
        transport,
        handler: handler.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });
    tokio::spawn(notify_tab_events(server.clone(), event_rx));
    server.start().await
}

/// Turn each event the shell addressed to this session into an MCP
/// notification.
///
/// The events the shell raises — a tab crashing, a tab closing — are already
/// scoped to this session's own tabs, so every event that arrives here is
/// forwarded as-is. The serialized event carries both the event name and the
/// tab id, so a client can act without a follow-up `tabs_list`.
///
/// Nothing is written to standard output: stdio is the MCP transport, and a
/// stray line there corrupts the JSON-RPC stream. Diagnostics go to standard
/// error, and a notification that cannot be delivered is reported rather than
/// dropped in silence — an event path that discards without a trace is the
/// bug this whole change exists to fix.
async fn notify_tab_events(
    server: Arc<rust_mcp_sdk::mcp_server::ServerRuntime>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<Event>,
) {
    // No notification can be sent before the client has initialized. The
    // channel is unbounded, so an event raised in that window is delayed
    // rather than lost.
    server.wait_for_initialization().await;
    while let Some(event) = events.recv().await {
        let data = match serde_json::to_value(&event) {
            Ok(data) => data,
            Err(error) => {
                eprintln!("talaria-mcp: cannot serialize {event:?}: {error}");
                continue;
            },
        };
        let level = match event {
            Event::TabCrashed { .. } => LoggingLevel::Warning,
            Event::TabClosed { .. } => LoggingLevel::Info,
            Event::TabOpened { .. } => LoggingLevel::Info,
        };
        let params = LoggingMessageNotificationParams {
            data,
            level,
            logger: Some("talaria.tabs".to_owned()),
            meta: None,
        };
        if let Err(error) = server.notify_log_message(params).await {
            eprintln!("talaria-mcp: tab event notification not delivered: {error}");
        }
    }
}

use rust_mcp_sdk::mcp_server::ToMcpServerHandler;
