//! Local control socket: the in-process precursor to the distributed
//! protocol. The `talaria-mcp` stdio proxy (and later, remote clients over
//! Tailscale) connect here; commands are ferried into the winit event loop
//! and executed against real webviews on the main thread.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use talaria_protocol::{
    socket_path, ClientMessage, Command, Outcome, ServerMessage,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

pub struct AgentRequest {
    pub session_id: u64,
    pub client: String,
    pub command: Command,
    pub reply: oneshot::Sender<Outcome>,
}

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

pub fn spawn(proxy: EventLoopProxy<AppEvent>) {
    std::thread::Builder::new()
        .name("talaria-control".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("control socket runtime");
            runtime.block_on(serve(proxy));
        })
        .expect("spawn control thread");
}

async fn serve(proxy: EventLoopProxy<AppEvent>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("control socket bind failed at {}: {error}", path.display());
            return;
        },
    };
    log::info!("control socket listening at {}", path.display());

    let proxy = Arc::new(proxy);
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let proxy = proxy.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, proxy).await {
                        log::debug!("control connection ended: {error}");
                    }
                });
            },
            Err(error) => {
                log::warn!("control accept error: {error}");
            },
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    proxy: Arc<EventLoopProxy<AppEvent>>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Handshake: first line must be Hello.
    let Some(first) = lines.next_line().await? else {
        return Ok(());
    };
    let client = match serde_json::from_str::<ClientMessage>(&first) {
        Ok(ClientMessage::Hello { client }) => client,
        _ => {
            write_line(
                &mut write_half,
                &ServerMessage::Reply {
                    id: 0,
                    outcome: Outcome::Error { message: "expected hello".into() },
                },
            )
            .await?;
            return Ok(());
        },
    };

    let session_id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    // Outbound messages (replies AND unsolicited events, e.g. tab_crashed)
    // funnel through one channel so a dedicated writer task can interleave
    // them safely on the socket.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let _ = proxy.send_event(AppEvent::SessionStarted {
        session_id,
        client: client.clone(),
        events: out_tx.clone(),
    });
    write_line(&mut write_half, &ServerMessage::HelloAck { session_id }).await?;

    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write_line(&mut write_half, &message).await.is_err() {
                break;
            }
        }
    });

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let (id, outcome) = match serde_json::from_str::<ClientMessage>(&line) {
            Ok(ClientMessage::Request { id, command }) => {
                let (tx, rx) = oneshot::channel();
                let request = AgentRequest {
                    session_id,
                    client: client.clone(),
                    command,
                    reply: tx,
                };
                if proxy.send_event(AppEvent::Agent(request)).is_err() {
                    break; // Event loop is gone; shell is shutting down.
                }
                // Commands that never complete (e.g. `evaluate` of a script
                // that never terminates — servo never fires the callback)
                // must not hang the agent forever.
                let timeout_secs = std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30);
                let outcome = match tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    rx,
                )
                .await
                {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(_)) => Outcome::Error {
                        message: "shell dropped the request".into(),
                    },
                    Err(_) => Outcome::Error {
                        message: format!(
                            "timed out after {timeout_secs}s (script still running?)"
                        ),
                    },
                };
                (id, outcome)
            },
            Ok(ClientMessage::Hello { .. }) => {
                (0, Outcome::Error { message: "duplicate hello".into() })
            },
            Err(error) => (0, Outcome::Error { message: format!("bad request: {error}") }),
        };
        if out_tx.send(ServerMessage::Reply { id, outcome }).is_err() {
            break; // Writer is gone; connection is dead.
        }
    }

    let _ = proxy.send_event(AppEvent::SessionEnded { session_id });
    drop(out_tx);
    writer.abort();
    Ok(())
}

async fn write_line(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &ServerMessage,
) -> std::io::Result<()> {
    let mut data = serde_json::to_vec(message).expect("serializable");
    data.push(b'\n');
    writer.write_all(&data).await
}
