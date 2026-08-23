//! The local control socket's filesystem and credential helpers. Unix-only.
//!
//! What this module is **not** is part of the wire. A peer on another machine
//! has neither this filesystem nor this user id, and a crate that carried
//! `socket_path` and a raw `getuid` shim at its root was quietly claiming
//! otherwise — four of its public items were a local path and a local UID.
//! They live behind a compilation gate here so that the crate root can be what
//! it actually is: a vocabulary, with no transport in it.
//!
//! They are kept together rather than scattered because one security fact ties
//! them into a single mechanism. The socket is owner-only: it lives in a
//! directory only its owner can enter, and the shell refuses to serve if it
//! cannot establish that. Anything that reaches the socket can read the user's
//! credentials, drive their logged-in sessions, and run arbitrary JavaScript in
//! them, so reachability by another local user is itself the vulnerability —
//! and [`current_uid`] is how the shell checks the peer against itself.

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
