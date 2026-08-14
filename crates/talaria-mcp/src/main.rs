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
    PaginatedRequestParams, ProtocolVersion, RpcError, ServerCapabilities,
    ServerCapabilitiesTools,
};
use rust_mcp_sdk::{error::SdkResult, McpServer, StdioTransport, TransportOptions};
use std::sync::Arc;

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
    let connection = ShellConnection::new();

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
    server.start().await
}

use rust_mcp_sdk::mcp_server::ToMcpServerHandler;
