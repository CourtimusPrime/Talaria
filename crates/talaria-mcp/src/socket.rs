//! Lazy, pipelined connection to the Talaria shell's control socket.
//!
//! One connection carries every tool call this proxy makes, and many calls may
//! be outstanding at once: a background reader matches each reply to its
//! waiting caller by request id, and a dedicated writer task serialises the
//! outbound lines. Nothing holds a lock across a round trip, so a wedged
//! `evaluate` on one tab cannot delay a `tabs_list` or a `tabs_close` on
//! another.
//!
//! Retry is deliberately narrow. A command is re-sent only when it never
//! reached a live wire — the connection was absent, or its writer task was
//! already gone. A request the connection accepted is never re-sent: a
//! retried `tabs_open` or `download` that runs twice is worse than the
//! stale-connection error the retry was hiding.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use talaria_protocol::{socket_path, ClientMessage, Command, Outcome, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

/// Request id -> the one-shot that will deliver that request's outcome. Held
/// by both `request` (which registers) and the reader task (which delivers).
type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Outcome>>>>;

pub struct ShellConnection {
    state: Mutex<Option<Connected>>,
    next_id: AtomicU64,
}

/// A live wire plus the bookkeeping that lets many requests share it.
struct Connected {
    /// Outbound client messages; the writer task drains this. Its closed-ness
    /// is also how `request` detects a wire that has already died.
    outbound: mpsc::UnboundedSender<ClientMessage>,
    waiters: Waiters,
    reader: JoinHandle<()>,
    /// Shared with the reader task so whichever notices the connection is
    /// dead can stop the other.
    writer: Arc<JoinHandle<()>>,
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

/// Outcome of one attempt to hand a command to the wire.
enum Attempt {
    /// The request reached a live connection (or failed for a reason that
    /// re-sending cannot fix). Never retried.
    Done(Result<Outcome, String>),
    /// The outbound channel was closed, so the request was never handed to a
    /// live wire. This is the only retryable case.
    NotSent,
}

impl ShellConnection {
    pub fn new() -> Self {
        Self { state: Mutex::new(None), next_id: AtomicU64::new(1) }
    }

    /// Send one command, await its reply. Connects (with the given client
    /// identity) on first use; reconnects once if the shell restarted.
    ///
    /// Concurrent calls are genuinely concurrent: the lock is held for
    /// bookkeeping only, never across the round trip.
    pub async fn request(&self, client: &str, command: Command) -> Result<Outcome, String> {
        match self.attempt(client, &command).await {
            Attempt::Done(result) => result,
            Attempt::NotSent => match self.attempt(client, &command).await {
                Attempt::Done(result) => result,
                Attempt::NotSent => {
                    Err("shell closed the connection before the request was sent".into())
                },
            },
        }
    }

    async fn attempt(&self, client: &str, command: &Command) -> Attempt {
        // Bookkeeping under the lock; the round trip happens after it drops.
        let (outbound, waiters) = {
            let mut state = self.state.lock().await;
            if state.as_ref().is_none_or(|connected| connected.outbound.is_closed()) {
                // Replacing the value drops the dead one, aborting its tasks.
                match connect(client).await {
                    Ok(connected) => *state = Some(connected),
                    Err(error) => return Attempt::Done(Err(error)),
                }
            }
            match state.as_ref() {
                Some(connected) => (connected.outbound.clone(), connected.waiters.clone()),
                // Unreachable — the branch above just filled it. Returning a
                // message instead of panicking keeps this crate unwrap-free.
                None => return Attempt::Done(Err("control connection unavailable".into())),
            }
        };

        // Register before sending: the reply can arrive the instant the writer
        // flushes, and a reply with no waiter is discarded.
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        waiters.lock().await.insert(id, sender);

        let message = ClientMessage::Request { id, command: command.clone() };
        if outbound.send(message).is_err() {
            // The writer task is gone, so this message never reached a live
            // wire and re-sending it cannot execute the command twice.
            waiters.lock().await.remove(&id);
            let mut state = self.state.lock().await;
            // Only clear the state if it is still the wire we just tried; a
            // concurrent caller may already have reconnected.
            if state.as_ref().is_some_and(|c| c.outbound.same_channel(&outbound)) {
                *state = None;
            }
            return Attempt::NotSent;
        }

        // Past this point the request reached a live connection: whatever
        // happens next, it is never re-sent (D-10).
        match receiver.await {
            Ok(outcome) => Attempt::Done(Ok(outcome)),
            Err(_) => {
                Attempt::Done(Err("shell closed the connection before replying".into()))
            },
        }
    }
}

async fn connect(client: &str) -> Result<Connected, String> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|error| {
        format!(
            "cannot reach Talaria at {} ({error}) — is the Talaria browser running?",
            path.display()
        )
    })?;
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);
    write_message(&mut writer, &ClientMessage::Hello { client: client.to_owned() }).await?;
    match read_message(&mut reader).await? {
        ServerMessage::HelloAck { .. } => {},
        other => return Err(format!("unexpected handshake reply: {other:?}")),
    }

    // Same shape as the shell's own writer task: one task owns the write half
    // and drains a channel, so callers never contend on the socket.
    let (outbound, mut outbox) = mpsc::unbounded_channel::<ClientMessage>();
    let writer = Arc::new(tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            if write_message(&mut writer, &message).await.is_err() {
                break;
            }
        }
    }));
    let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
    let reader = tokio::spawn(read_loop(reader, waiters.clone(), writer.clone()));
    Ok(Connected { outbound, waiters, reader, writer })
}

/// Read replies until the connection dies, handing each outcome to the caller
/// that is waiting on its id.
async fn read_loop(
    mut reader: BufReader<OwnedReadHalf>,
    waiters: Waiters,
    writer: Arc<JoinHandle<()>>,
) {
    loop {
        match read_message(&mut reader).await {
            Ok(ServerMessage::Reply { id, outcome }) => {
                let waiter = waiters.lock().await.remove(&id);
                if let Some(waiter) = waiter {
                    // A caller that gave up is not an error.
                    let _ = waiter.send(outcome);
                }
            },
            Ok(ServerMessage::Event { .. }) => {
                // Dropped for now. Plan 02-08 turns this arm into an MCP
                // notification; the background reader existing at all is what
                // makes an unsolicited server-to-client message possible.
            },
            Ok(ServerMessage::HelloAck { .. }) => {
                // Only legal during the handshake, which `connect` consumed.
            },
            Err(_) => break,
        }
    }
    // The wire is gone: drop every outstanding one-shot so its caller sees a
    // closed channel instead of hanging until the MCP host gives up.
    waiters.lock().await.clear();
    // Closes the outbound channel, so the next `request` sees a dead wire.
    writer.abort();
}

async fn write_message(
    writer: &mut BufWriter<OwnedWriteHalf>,
    message: &ClientMessage,
) -> Result<(), String> {
    let mut data = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    data.push(b'\n');
    writer.write_all(&data).await.map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())
}

async fn read_message(reader: &mut BufReader<OwnedReadHalf>) -> Result<ServerMessage, String> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
    if read == 0 {
        return Err("shell closed the connection".into());
    }
    serde_json::from_str(&line).map_err(|e| format!("bad reply: {e}"))
}
