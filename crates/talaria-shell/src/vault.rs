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

fn load_or_create_key(dir: &PathBuf) -> KeyOutcome {
    // Preferred: OS keychain. Every fall-through below is a downgrade in
    // data-at-rest protection, so each one records it — not just the branch
    // that happens to fire most often.
    let mut downgraded = false;
    match keyring::Entry::new("talaria", "vault") {
        Ok(entry) => match entry.get_password() {
            Ok(hex_key) => {
                if let Ok(bytes) = hex::decode(&hex_key) {
                    if bytes.len() == 32 {
                        return KeyOutcome { key: Some(*Key::from_slice(&bytes)), downgraded };
                    }
                }
                log::warn!("keychain vault key malformed; falling back to key file");
                downgraded = true;
            },
            Err(keyring::Error::NoEntry) => {
                let key = ChaCha20Poly1305::generate_key(&mut OsRng);
                if entry.set_password(&hex::encode(key)).is_ok() {
                    return KeyOutcome { key: Some(key), downgraded };
                }
                log::warn!("could not store vault key in keychain; falling back to key file");
                downgraded = true;
            },
            Err(error) => {
                log::warn!("keychain unavailable ({error}); falling back to key file");
                downgraded = true;
            },
        },
        Err(error) => {
            log::warn!("keyring init failed ({error}); falling back to key file");
            downgraded = true;
        },
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
        }
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
