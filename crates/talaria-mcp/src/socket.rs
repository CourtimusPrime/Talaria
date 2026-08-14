//! Lazy connection to the Talaria shell's control socket.

use std::sync::atomic::{AtomicU64, Ordering};

use talaria_protocol::{socket_path, ClientMessage, Command, Outcome, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

pub struct ShellConnection {
    inner: Mutex<Option<Wire>>,
    next_id: AtomicU64,
}

struct Wire {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
}

impl ShellConnection {
    pub fn new() -> Self {
        Self { inner: Mutex::new(None), next_id: AtomicU64::new(1) }
    }

    /// Send one command, await its reply. Connects (with the given client
    /// identity) on first use; reconnects once if the shell restarted.
    pub async fn request(&self, client: &str, command: Command) -> Result<Outcome, String> {
        let mut guard = self.inner.lock().await;
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(connect(client).await?);
            }
            let wire = guard.as_mut().expect("connected above");
            match round_trip(wire, self.next_id.fetch_add(1, Ordering::Relaxed), &command).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if attempt == 0 => {
                    // Stale connection (shell restarted): drop and retry once.
                    *guard = None;
                    let _ = error;
                },
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}

async fn connect(client: &str) -> Result<Wire, String> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|error| {
        format!(
            "cannot reach Talaria at {} ({error}) — is the Talaria browser running?",
            path.display()
        )
    })?;
    let (read_half, write_half) = stream.into_split();
    let mut wire = Wire {
        reader: BufReader::new(read_half),
        writer: BufWriter::new(write_half),
    };
    write_message(&mut wire, &ClientMessage::Hello { client: client.to_owned() }).await?;
    match read_message(&mut wire).await? {
        ServerMessage::HelloAck { .. } => Ok(wire),
        other => Err(format!("unexpected handshake reply: {other:?}")),
    }
}

async fn round_trip(wire: &mut Wire, id: u64, command: &Command) -> Result<Outcome, String> {
    write_message(wire, &ClientMessage::Request { id, command: command.clone() }).await?;
    loop {
        match read_message(wire).await? {
            ServerMessage::Reply { id: reply_id, outcome } if reply_id == id => {
                return Ok(outcome);
            },
            ServerMessage::Reply { .. } | ServerMessage::Event { .. } | ServerMessage::HelloAck { .. } => {
                // Not ours (event or stray reply) — keep reading.
            },
        }
    }
}

async fn write_message(wire: &mut Wire, message: &ClientMessage) -> Result<(), String> {
    let mut data = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    data.push(b'\n');
    wire.writer.write_all(&data).await.map_err(|e| e.to_string())?;
    wire.writer.flush().await.map_err(|e| e.to_string())
}

async fn read_message(wire: &mut Wire) -> Result<ServerMessage, String> {
    let mut line = String::new();
    let read = wire
        .reader
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    if read == 0 {
        return Err("shell closed the connection".into());
    }
    serde_json::from_str(&line).map_err(|e| format!("bad reply: {e}"))
}
