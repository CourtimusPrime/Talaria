//! Credential vault: a JSON list of credential entries, encrypted at rest
//! with ChaCha20-Poly1305. The key lives in the OS keychain when one is
//! available; headless/keychain-less systems fall back to a 0600 key file
//! beside the vault (logged as a warning — same data-at-rest posture as a
//! browser profile on such systems).
//!
//! Per SPEC: this vault backs autofill suggestion and `cookies.read` session
//! reuse. It is *not* an agent-driven login mechanism — fresh logins happen
//! via human takeover.
//!
//! The vault is writable from inside the shell: [`Vault::upsert`] and
//! [`Vault::delete`] are the capture path a credentials panel drives, keyed on
//! an entry's URL host plus its username. Both persist immediately, so a
//! caller never has to remember a second step.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};
use talaria_protocol::CredentialEntry;

const MAGIC: &[u8; 8] = b"TALARIA1";

/// What happened to a plaintext credential file found at load.
///
/// Carried out of [`Vault::load`] so the chrome can say which of the two
/// outcomes the user got. A `source_removed` of `false` means a readable
/// plaintext copy of their passwords is still sitting on disk, which is
/// exactly the thing they must not be left believing was cleaned up.
pub struct ImportNotice {
    /// The plaintext file that was imported.
    pub path: PathBuf,
    /// Whether that file was removed after the encrypted copy was verified.
    pub source_removed: bool,
}

pub struct Vault {
    path: PathBuf,
    cipher: Option<ChaCha20Poly1305>,
    entries: Vec<CredentialEntry>,
    /// One-shot: set when a plaintext credential file was imported during
    /// this load. The chrome reads it once and clears it. It exists because
    /// `log::warn!` is not a user-facing surface — a user whose passwords
    /// were just moved, or just *not* moved, cannot be expected to have a
    /// terminal open to find out.
    import_notice: Option<ImportNotice>,
    /// One-shot: set when data-at-rest protection was downgraded to a key
    /// file sitting beside the ciphertext because the OS keychain could not
    /// be used. The chrome reads it once and clears it, for the same reason:
    /// a downgrade the user only ever sees in a log is a downgrade they never
    /// consented to.
    key_downgraded: bool,
}

/// The outcome of resolving the vault key: the key when one could be obtained,
/// and whether obtaining it meant dropping from the OS keychain to a key file.
struct KeyOutcome {
    key: Option<Key>,
    downgraded: bool,
}

/// The host an entry keys on: its URL's host, lowercased. `None` when the URL
/// does not parse into a host — a bare hostname a user typed into a
/// credentials panel is the common case, and it has no key to match on.
fn entry_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

/// A domain as the matching and delete paths compare it: trimmed, stripped of
/// a leading dot, lowercased.
fn normalise_domain(domain: &str) -> String {
    domain.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("talaria")
}

/// Best-effort mitigation for a file holding plaintext secrets that has to
/// stay on disk: make it owner-only, and say so at error level when even that
/// fails, because the alternative is a world-readable password file nobody is
/// told about.
fn restrict_to_owner(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            log::error!("could not make {} owner-only: {error}", path.display());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// How long the OS keychain gets to answer before the vault stops waiting.
///
/// This number is bounded on both sides. It must stay *below* the shortest
/// control-socket command timeout the e2e suite uses (3s, set by
/// `tests/e2e/run_all.py`), or a slow keychain stops being a logged downgrade
/// and starts being a failing test. It must stay far *above* a healthy keychain
/// lookup, which is single-digit milliseconds, or a working keychain gets
/// abandoned for no reason.
const KEYCHAIN_TIMEOUT_MS: u64 = 1500;

fn keychain_timeout() -> std::time::Duration {
    // Overridable because "your keychain is slower than 1.5s" is a real machine
    // configuration, and the answer to it should be a longer wait rather than a
    // forced downgrade to a key file.
    let millis = std::env::var("TALARIA_KEYCHAIN_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .unwrap_or(KEYCHAIN_TIMEOUT_MS);
    std::time::Duration::from_millis(millis)
}

/// Whether a session D-Bus exists to talk to at all.
///
/// This exists to avoid *provoking* D-Bus autolaunch. With no
/// `DBUS_SESSION_BUS_ADDRESS`, libdbus does not simply fail — it tries to
/// **start a session bus itself**, and on a machine where that cannot succeed
/// it blocks forever rather than returning an error. `Vault::load` runs on the
/// main thread during startup, so that hang is the whole browser: the control
/// socket keeps answering `hello` from its own thread while the event loop
/// never services a single command, which is precisely how it presented when CI
/// first ran on a machine with no desktop session.
///
/// A `stat` is enough to tell the hopeless case apart, and it is worth doing
/// because that case is the common one: headless servers, containers, SSH
/// sessions, and CI jobs that pin `XDG_RUNTIME_DIR` to a private empty
/// directory. It is *not* sufficient on its own — a bus address that is set but
/// dead still hangs — which is why the call is also time-bounded below.
#[cfg(target_os = "linux")]
fn session_bus_reachable() -> bool {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return true;
    }
    dirs::runtime_dir().is_some_and(|dir| dir.join("bus").exists())
}

/// Non-Linux keychains (macOS Keychain Services, Windows Credential Manager)
/// do not route through D-Bus, so there is nothing to pre-check. The timeout
/// still applies to them.
#[cfg(not(target_os = "linux"))]
fn session_bus_reachable() -> bool {
    true
}

/// Run `work` on a worker thread and give up on it after `timeout`.
///
/// Returns `None` if the work did not finish in time, or if the thread could
/// not be spawned at all.
///
/// A timed-out thread is **detached and leaked** — deliberately. The work this
/// bounds is a blocking FFI call into libdbus that owns no `Vault` state and
/// has no cancellation point, so there is nothing to interrupt and nothing that
/// can be corrupted by letting it run. One stranded thread for the life of the
/// process is the price of not hanging the browser, and it is a price paid only
/// on a machine that is already misconfigured.
fn with_timeout<T: Send + 'static>(
    timeout: std::time::Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("talaria-keychain".into())
        .spawn(move || {
            // The receiver is gone on timeout; that is the expected path, not
            // an error worth reporting from inside a thread nobody is reading.
            let _ = sender.send(work());
        });
    if let Err(error) = spawned {
        log::warn!("could not spawn keychain thread ({error}); falling back to key file");
        return None;
    }
    receiver.recv_timeout(timeout).ok()
}

/// What the OS keychain had to say, once it has said anything at all.
enum KeychainAnswer {
    /// A usable 32-byte key, either read back or freshly stored.
    Key(Key),
    /// The keychain was reachable but cannot supply a key. Carries the message
    /// to log, so the reason survives the trip back from the worker thread.
    Unusable(String),
}

/// The whole keychain conversation, in one place, so it can be handed to a
/// worker thread as a unit.
fn ask_keychain() -> KeychainAnswer {
    match keyring::Entry::new("talaria", "vault") {
        Ok(entry) => match entry.get_password() {
            Ok(hex_key) => {
                if let Ok(bytes) = hex::decode(&hex_key) {
                    if bytes.len() == 32 {
                        return KeychainAnswer::Key(*Key::from_slice(&bytes));
                    }
                }
                KeychainAnswer::Unusable("keychain vault key malformed".into())
            },
            Err(keyring::Error::NoEntry) => {
                let key = ChaCha20Poly1305::generate_key(&mut OsRng);
                if entry.set_password(&hex::encode(key)).is_ok() {
                    return KeychainAnswer::Key(key);
                }
                KeychainAnswer::Unusable("could not store vault key in keychain".into())
            },
            Err(error) => KeychainAnswer::Unusable(format!("keychain unavailable ({error})")),
        },
        Err(error) => KeychainAnswer::Unusable(format!("keyring init failed ({error})")),
    }
}

fn load_or_create_key(dir: &PathBuf) -> KeyOutcome {
    // Preferred: OS keychain. Every fall-through below is a downgrade in
    // data-at-rest protection, so each one records it — not just the branch
    // that happens to fire most often.
    let mut downgraded = false;
    if !session_bus_reachable() {
        log::warn!("no session D-Bus; skipping the OS keychain and using a key file");
        downgraded = true;
    } else {
        match with_timeout(keychain_timeout(), ask_keychain) {
            Some(KeychainAnswer::Key(key)) => return KeyOutcome { key: Some(key), downgraded },
            Some(KeychainAnswer::Unusable(reason)) => {
                log::warn!("{reason}; falling back to key file");
                downgraded = true;
            },
            None => {
                log::warn!(
                    "OS keychain did not answer within {:?}; falling back to key file",
                    keychain_timeout()
                );
                downgraded = true;
            },
        }
    }

    // Fallback: key file with owner-only permissions.
    let key_path = dir.join("vault.key");
    if let Ok(hex_key) = fs::read_to_string(&key_path) {
        if let Ok(bytes) = hex::decode(hex_key.trim()) {
            if bytes.len() == 32 {
                return KeyOutcome { key: Some(*Key::from_slice(&bytes)), downgraded };
            }
        }
        log::warn!("vault key file malformed; refusing to touch vault");
        return KeyOutcome { key: None, downgraded };
    }
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
    if fs::create_dir_all(dir).is_err() {
        return KeyOutcome { key: None, downgraded };
    }
    let Ok(mut file) = fs::File::create(&key_path) else {
        return KeyOutcome { key: None, downgraded };
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Checked, not discarded: a key file we could not make owner-only is
        // a worse posture than this module's docstring describes, so it is
        // reported rather than silently accepted.
        if let Err(error) = file.set_permissions(fs::Permissions::from_mode(0o600)) {
            log::error!("vault key file {} is not owner-only: {error}", key_path.display());
            downgraded = true;
        }
    }
    if file.write_all(hex::encode(key).as_bytes()).is_err() {
        return KeyOutcome { key: None, downgraded };
    }
    KeyOutcome { key: Some(key), downgraded }
}

impl Vault {
    pub fn load() -> Self {
        let dir = config_dir();
        let path = dir.join("vault.enc");
        let plain_path = dir.join("vault.json");
        let outcome = load_or_create_key(&dir);
        let Some(key) = outcome.key else {
            return Self {
                path,
                cipher: None,
                entries: Vec::new(),
                import_notice: None,
                key_downgraded: outcome.downgraded,
            };
        };
        let cipher = ChaCha20Poly1305::new(&key);

        // Decrypt-first, then-plaintext ordering is unchanged; the only
        // addition is remembering *which* of the two the entries came from,
        // because only the plaintext branch has a source file to clean up.
        let mut imported = false;
        let entries = match fs::read(&path) {
            Ok(data) if data.len() > MAGIC.len() + 12 && data.starts_with(MAGIC) => {
                let nonce = Nonce::from_slice(&data[MAGIC.len()..MAGIC.len() + 12]);
                match cipher.decrypt(nonce, &data[MAGIC.len() + 12..]) {
                    Ok(plain) => serde_json::from_slice(&plain).unwrap_or_else(|error| {
                        log::warn!("vault JSON malformed: {error}");
                        Vec::new()
                    }),
                    Err(_) => {
                        log::warn!("vault decryption failed (wrong key?); starting empty");
                        Vec::new()
                    },
                }
            },
            Ok(_) => {
                log::warn!("vault file malformed; starting empty");
                Vec::new()
            },
            Err(_) => {
                // First run: also accept a plaintext vault.json the user may
                // have hand-written, and encrypt it going forward.
                match fs::read(&plain_path) {
                    Ok(plain) => match serde_json::from_slice::<Vec<CredentialEntry>>(&plain) {
                        Ok(parsed) => {
                            log::warn!("importing plaintext vault.json into encrypted vault");
                            imported = true;
                            parsed
                        },
                        Err(error) => {
                            // Degrade, never abort — and a file we could not
                            // parse is not an import, so it is left exactly
                            // where it is rather than counted as migrated.
                            log::warn!("plaintext vault.json malformed ({error}); leaving it alone");
                            Vec::new()
                        },
                    },
                    Err(_) => Vec::new(),
                }
            },
        };

        let mut vault = Self {
            path,
            cipher: Some(cipher),
            entries,
            import_notice: None,
            key_downgraded: outcome.downgraded,
        };
        vault.save();
        if imported {
            vault.finish_import(plain_path);
        }
        vault
    }

    /// Complete a plaintext import by removing the source — but only once the
    /// encrypted copy has been proven durable.
    ///
    /// The ordering is the whole safety argument. [`Vault::save`] returns
    /// early on any failure, so "it ran" is not evidence that anything landed;
    /// the encrypted file is therefore re-read, decrypted and deserialised,
    /// and only a matching entry count authorises the removal. A failed
    /// encryption can no longer destroy the user's only copy.
    ///
    /// It is also why two shell processes cannot half-migrate: the source
    /// disappears only after a verified write, and a process that finds no
    /// plaintext file performs no import at all, so the second one through
    /// takes the decrypt branch and leaves the first one's work alone.
    fn finish_import(&mut self, plain_path: PathBuf) {
        let source_removed = if self.verify_written() {
            match fs::remove_file(&plain_path) {
                Ok(()) => true,
                Err(error) => {
                    log::error!(
                        "plaintext credentials still readable at {} ({error})",
                        plain_path.display()
                    );
                    false
                },
            }
        } else {
            log::error!(
                "encrypted vault could not be verified; plaintext credentials still readable at {}",
                plain_path.display()
            );
            false
        };
        if !source_removed {
            restrict_to_owner(&plain_path);
        }
        self.import_notice = Some(ImportNotice { path: plain_path, source_removed });
    }

    /// Re-read, decrypt and deserialise the file [`Vault::save`] just wrote,
    /// confirming it holds as many entries as are in memory.
    fn verify_written(&self) -> bool {
        let Some(cipher) = &self.cipher else { return false };
        let Ok(data) = fs::read(&self.path) else { return false };
        if data.len() <= MAGIC.len() + 12 || !data.starts_with(MAGIC) {
            return false;
        }
        let nonce = Nonce::from_slice(&data[MAGIC.len()..MAGIC.len() + 12]);
        let Ok(plain) = cipher.decrypt(nonce, &data[MAGIC.len() + 12..]) else {
            return false;
        };
        let Ok(written) = serde_json::from_slice::<Vec<CredentialEntry>>(&plain) else {
            return false;
        };
        written.len() == self.entries.len()
    }

    pub fn save(&mut self) {
        let Some(cipher) = &self.cipher else { return };
        let Ok(plain) = serde_json::to_vec(&self.entries) else { return };
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let Ok(ciphertext) = cipher.encrypt(&nonce, plain.as_slice()) else { return };
        let mut data = Vec::with_capacity(MAGIC.len() + 12 + ciphertext.len());
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&ciphertext);
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::write(&self.path, data) {
            log::warn!("could not write vault: {error}");
            return;
        }
        // The vault is encrypted, so this is defence in depth rather than the
        // only thing standing between the file and a reader — but the mode was
        // never set on this path at all, so the file landed at whatever the
        // umask allowed, which on a default Linux install is world-readable.
        // A ciphertext nobody else can read is still better than one they can
        // copy at leisure and attack offline.
        restrict_to_owner(&self.path);
    }

    /// Store `entry`, replacing the existing entry with the same URL host and
    /// username, and persist. The key is the host, not the whole URL, so
    /// re-saving the same login from a different page of the same site
    /// updates it rather than accumulating near-duplicates.
    ///
    /// An entry whose URL does not parse into a host has no key to match on,
    /// so it is appended rather than replacing anything. A user typing a bare
    /// hostname into a credentials panel is a real case, and keeping an
    /// unmatched entry is better than silently discarding their input.
    pub fn upsert(&mut self, entry: CredentialEntry) {
        let host = entry_host(&entry.url);
        let existing = host.and_then(|host| {
            self.entries.iter().position(|candidate| {
                candidate.username == entry.username
                    && entry_host(&candidate.url).is_some_and(|candidate| candidate == host)
            })
        });
        match existing {
            Some(index) => self.entries[index] = entry,
            None => self.entries.push(entry),
        }
        self.save();
    }

    /// Remove the entry keyed on `host` and `username`, reporting whether one
    /// was there. Persists only when something was actually removed.
    pub fn delete(&mut self, host: &str, username: &str) -> bool {
        let host = normalise_domain(host);
        let before = self.entries.len();
        self.entries.retain(|entry| {
            !(entry.username == username
                && entry_host(&entry.url).is_some_and(|candidate| candidate == host))
        });
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Every stored entry, borrowed so a UI can list them without cloning the
    /// whole vector. Pairs with [`Vault::matching`], which clones because its
    /// result crosses the control socket.
    pub fn entries(&self) -> &[CredentialEntry] {
        &self.entries
    }

    /// The one-shot plaintext-import notice, when this load imported one.
    pub fn import_notice(&self) -> Option<&ImportNotice> {
        self.import_notice.as_ref()
    }

    /// Whether data-at-rest protection was downgraded to a key file beside
    /// the encrypted vault during this load.
    pub fn key_downgraded(&self) -> bool {
        self.key_downgraded
    }

    /// Clear both one-shot notices, once the chrome has shown them.
    pub fn clear_notices(&mut self) {
        self.import_notice = None;
        self.key_downgraded = false;
    }

    /// Entries whose URL host matches `domain` (exact or subdomain).
    pub fn matching(&self, domain: &str) -> Vec<CredentialEntry> {
        let domain = normalise_domain(domain);
        self.entries
            .iter()
            .filter(|entry| {
                entry_host(&entry.url).is_some_and(|host| {
                    host == domain
                        || host.ends_with(&format!(".{domain}"))
                        || domain.ends_with(&format!(".{host}"))
                })
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str) -> CredentialEntry {
        CredentialEntry {
            url: url.to_owned(),
            username: "u".into(),
            password: "p".into(),
            cookies: Vec::new(),
        }
    }

    #[test]
    fn with_timeout_returns_work_that_finishes_in_time() {
        let answer = with_timeout(std::time::Duration::from_secs(30), || 7u8);
        assert_eq!(answer, Some(7));
    }

    /// The case the whole fix exists for: work that never returns must not
    /// become a caller that never returns. A generous margin either side —
    /// the wait is short, the work outlives the test — so this asserts the
    /// behaviour rather than a race.
    #[test]
    fn with_timeout_gives_up_on_work_that_never_finishes() {
        let started = std::time::Instant::now();
        let answer = with_timeout(std::time::Duration::from_millis(50), || {
            std::thread::sleep(std::time::Duration::from_secs(60));
            7u8
        });
        assert_eq!(answer, None);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "with_timeout waited {:?}, so it did not bound the call",
            started.elapsed()
        );
    }

    /// The default is pinned from both sides, because both sides are what
    /// make it correct: above the shortest e2e command timeout and a slow
    /// keychain becomes a failing test instead of a logged downgrade; below a
    /// healthy lookup and a working keychain gets abandoned for no reason.
    #[test]
    fn keychain_timeout_default_stays_inside_its_bounds() {
        let default = std::time::Duration::from_millis(KEYCHAIN_TIMEOUT_MS);
        assert!(default < std::time::Duration::from_secs(3), "must stay under the e2e command timeout");
        assert!(default > std::time::Duration::from_millis(100), "must not abandon a healthy keychain");
    }

    fn login(url: &str, username: &str, password: &str) -> CredentialEntry {
        CredentialEntry {
            url: url.to_owned(),
            username: username.to_owned(),
            password: password.to_owned(),
            cookies: Vec::new(),
        }
    }

    /// A vault with no cipher: [`Vault::save`] returns early, so the write
    /// paths below never touch disk or the keychain.
    fn detached(entries: Vec<CredentialEntry>) -> Vault {
        Vault {
            path: PathBuf::new(),
            cipher: None,
            entries,
            import_notice: None,
            key_downgraded: false,
        }
    }

    /// The encrypted vault is the one file here that actually holds
    /// credentials, and its write path set no mode at all until this test
    /// existed — it landed at whatever the umask allowed, world-readable on a
    /// default Linux install. Ciphertext or not, that is a file worth not
    /// handing to every other account on the machine.
    #[cfg(unix)]
    #[test]
    fn a_saved_vault_lands_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("talaria-vault-perm-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vault.enc");
        // Pre-create it world-readable, so a pass cannot come from a strict
        // umask having done the job before `save` ran.
        let _ = fs::write(&path, b"stale");
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));

        let key = ChaCha20Poly1305::generate_key(&mut OsRng);
        let mut vault = Vault {
            path: path.clone(),
            cipher: Some(ChaCha20Poly1305::new(&key)),
            entries: vec![login("https://example.com/in", "person", "secret")],
            import_notice: None,
            key_downgraded: false,
        };
        vault.save();

        let mode = fs::metadata(&path).expect("vault written").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "vault.enc is {:o}, not owner-only", mode & 0o777);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn domain_matching() {
        let vault = detached(vec![
            entry("https://accounts.google.com/signin"),
            entry("https://github.com/login"),
        ]);
        assert_eq!(vault.matching("google.com").len(), 1);
        assert_eq!(vault.matching("accounts.google.com").len(), 1);
        assert_eq!(vault.matching("github.com").len(), 1);
        assert_eq!(vault.matching("example.com").len(), 0);
    }

    #[test]
    fn upsert_appends_a_new_entry() {
        let mut vault = detached(Vec::new());
        vault.upsert(login("https://github.com/login", "court", "first"));
        assert_eq!(vault.entries().len(), 1);
        assert_eq!(vault.entries()[0].username, "court");
    }

    #[test]
    fn upsert_replaces_the_same_host_and_username() {
        let mut vault = detached(Vec::new());
        vault.upsert(login("https://github.com/login", "court", "first"));
        vault.upsert(login("https://github.com/login", "court", "second"));
        assert_eq!(vault.entries().len(), 1, "the repeated save appended a duplicate");
        assert_eq!(vault.entries()[0].password, "second");
    }

    #[test]
    fn upsert_appends_a_second_username_on_the_same_host() {
        let mut vault = detached(Vec::new());
        vault.upsert(login("https://github.com/login", "court", "first"));
        vault.upsert(login("https://github.com/login", "other", "second"));
        assert_eq!(vault.entries().len(), 2);
    }

    #[test]
    fn upsert_matches_across_a_path_difference() {
        let mut vault = detached(Vec::new());
        vault.upsert(login("https://github.com/login", "court", "first"));
        vault.upsert(login("https://github.com/session/new", "court", "second"));
        assert_eq!(vault.entries().len(), 1, "the key is the host, not the whole URL");
        assert_eq!(vault.entries()[0].password, "second");
    }

    #[test]
    fn delete_removes_one_entry_and_reports_it() {
        let mut vault = detached(vec![
            login("https://github.com/login", "court", "first"),
            login("https://github.com/login", "other", "second"),
        ]);
        assert!(vault.delete("github.com", "court"));
        assert_eq!(vault.entries().len(), 1);
        assert_eq!(vault.entries()[0].username, "other");
    }

    #[test]
    fn delete_of_an_absent_pair_changes_nothing() {
        let mut vault = detached(vec![login("https://github.com/login", "court", "first")]);
        assert!(!vault.delete("github.com", "nobody"));
        assert!(!vault.delete("example.com", "court"));
        assert_eq!(vault.entries().len(), 1);
        assert_eq!(vault.entries()[0].password, "first");
    }
}
