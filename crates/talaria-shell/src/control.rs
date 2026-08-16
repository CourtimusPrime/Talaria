//! Local control socket: the in-process precursor to the distributed
//! protocol. The `talaria-mcp` stdio proxy (and later, remote clients over
//! Tailscale) connect here; commands are ferried into the winit event loop
//! and executed against real webviews on the main thread.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use talaria_protocol::{
    current_uid, ensure_socket_dir, socket_dir, socket_path, ClientMessage, Command, Outcome,
    ServerMessage,
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

/// If another Talaria already owns the control socket, hand it `url` to open
/// in the human's view and return `Ok(true)`; `Ok(false)` when no live
/// instance answers (a stale socket file is fine — bind removes it). Runs
/// before the event loop exists, so it uses blocking std sockets.
pub fn forward_to_running_instance(url: &str) -> std::io::Result<bool> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let path = socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        // Nothing listening (or never created): we're the first instance.
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false)
        },
        Err(error) => return Err(error),
    };
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut send = |message: &ClientMessage| -> std::io::Result<()> {
        let mut data = serde_json::to_vec(message).expect("serializable");
        data.push(b'\n');
        stream.write_all(&data)
    };
    send(&ClientMessage::Hello { client: "talaria-launcher".into() })?;
    send(&ClientMessage::Request { id: 1, command: Command::OpenForUser { url: url.to_owned() } })?;
    let mut lines = BufReader::new(stream).lines();
    let ack = lines.next().transpose()?;
    if !matches!(
        ack.as_deref().map(serde_json::from_str::<ServerMessage>),
        Some(Ok(ServerMessage::HelloAck { .. }))
    ) {
        return Ok(false); // Something else owns the path; start normally.
    }
    match lines.next().transpose()?.as_deref().map(serde_json::from_str::<ServerMessage>) {
        Some(Ok(ServerMessage::Reply { outcome: Outcome::Ok { .. }, .. })) => Ok(true),
        Some(Ok(ServerMessage::Reply { outcome: Outcome::Error { message }, .. })) => {
            Err(std::io::Error::other(message))
        },
        _ => Ok(false),
    }
}

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
    // A socket we cannot place in a private directory is not a socket we
    // should listen on: reaching it is equivalent to owning the session.
    if let Err(error) = ensure_socket_dir() {
        log::error!(
            "control socket directory {} unusable: {error}",
            socket_dir().display()
        );
        return;
    }
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("control socket bind failed at {}: {error}", path.display());
            return;
        },
    };
    // Fail closed on the same reasoning: this whole gate exists because the
    // socket used to be reachable by other local users.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let owner_only = std::fs::Permissions::from_mode(0o600);
        if let Err(error) = std::fs::set_permissions(&path, owner_only) {
            log::error!(
                "control socket {} could not be made owner-only: {error}",
                path.display()
            );
            drop(listener);
            let _ = std::fs::remove_file(&path);
            return;
        }
    }
    log::info!("control socket listening at {}", path.display());

    let proxy = Arc::new(proxy);
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                // Before the spawn, so a rejected peer never reaches the
                // handshake and never consumes a session id.
                if let Err(reason) = peer_uid_ok(&stream) {
                    // Nothing is written back: the peer gets a closed
                    // connection, not a hint about what the check was.
                    log::warn!("control connection refused: {reason}");
                    continue;
                }
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

/// Accept only peers running as the same OS user as the shell.
///
/// Scope: this stops *another local user* from driving the browser — the
/// `Hello` line is self-asserted and was the only identity on the wire. It is
/// deliberately not a per-agent permission layer: every accepted peer keeps
/// the same full tool surface it has today.
///
/// Fails closed. A peer whose credentials the kernel will not vouch for is
/// exactly the case this gate exists for, so a lookup error is a rejection.
fn peer_uid_ok(stream: &UnixStream) -> Result<(), String> {
    let ours = current_uid();
    match stream.peer_cred() {
        Ok(credential) if credential.uid() == ours => Ok(()),
        Ok(credential) => {
            Err(format!("peer uid {} is not ours ({ours})", credential.uid()))
        },
        Err(error) => Err(format!("peer credentials unavailable: {error}")),
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

    // Each write is bounded by the same clock a command is: a write that
    // cannot land within it means a peer that stopped reading, not a slow
    // one, and an unbounded write here would strand this task forever once
    // teardown starts awaiting it.
    let write_bound = std::time::Duration::from_secs(command_timeout_secs());
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            match tokio::time::timeout(write_bound, write_line(&mut write_half, &message)).await {
                Ok(Ok(())) => {},
                _ => break,
            }
        }
    });

    // One task per in-flight request, so a slow command on one tab does not
    // stop the next line being read. Tracked rather than detached: teardown
    // must not truncate a reply that is still outstanding.
    let mut inflight = tokio::task::JoinSet::new();

    while let Some(line) = lines.next_line().await? {
        // Reap finished requests so a long-lived session's set does not grow
        // with every command it has ever issued. Never blocks.
        while inflight.try_join_next().is_some() {}
        if line.trim().is_empty() {
            continue;
        }
        // A request's outcome is awaited off this loop; the other two arms
        // have nothing to wait for and answer inline.
        let immediate = match serde_json::from_str::<ClientMessage>(&line) {
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
                let replies = out_tx.clone();
                inflight.spawn(async move {
                    // Commands that never complete (e.g. `evaluate` of a script
                    // that never terminates — servo never fires the callback)
                    // must not hang the agent forever.
                    let timeout_secs = command_timeout_secs();
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
                    let _ = replies.send(ServerMessage::Reply { id, outcome });
                });
                None
            },
            Ok(ClientMessage::Hello { .. }) => {
                Some((0, Outcome::Error { message: "duplicate hello".into() }))
            },
            Err(error) => Some((0, Outcome::Error { message: format!("bad request: {error}") })),
        };
        if let Some((id, outcome)) = immediate {
            if out_tx.send(ServerMessage::Reply { id, outcome }).is_err() {
                break; // Writer is gone; connection is dead.
            }
        }
    }

    // Let every outstanding request answer before the socket goes away. Each
    // is already bounded by the command timeout, so this drain is bounded too.
    while inflight.join_next().await.is_some() {}

    let _ = proxy.send_event(AppEvent::SessionEnded { session_id });
    drop(out_tx);
    // Awaited, not aborted: aborting here is exactly what would truncate the
    // replies the drain above just queued.
    let _ = writer.await;
    Ok(())
}

/// Outer bound on one command: how long the shell waits for the event loop to
/// produce an outcome before answering the agent with an error, default 30s.
/// The same bound caps a single socket write, so a peer that stops reading
/// cannot strand a connection's writer task.
fn command_timeout_secs() -> u64 {
    std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30)
}

async fn write_line(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &ServerMessage,
) -> std::io::Result<()> {
    let mut data = serde_json::to_vec(message).expect("serializable");
    data.push(b'\n');
    writer.write_all(&data).await
}
