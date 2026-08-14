//! Credential vault: a JSON list of credential entries, encrypted at rest
//! with ChaCha20-Poly1305. The key lives in the OS keychain when one is
//! available; headless/keychain-less systems fall back to a 0600 key file
//! beside the vault (logged as a warning — same data-at-rest posture as a
//! browser profile on such systems).
//!
//! Per SPEC: this vault backs autofill suggestion and `cookies.read` session
//! reuse. It is *not* an agent-driven login mechanism — fresh logins happen
//! via human takeover.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};
use talaria_protocol::CredentialEntry;

const MAGIC: &[u8; 8] = b"TALARIA1";

pub struct Vault {
    path: PathBuf,
    cipher: Option<ChaCha20Poly1305>,
    entries: Vec<CredentialEntry>,
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("talaria")
}

fn load_or_create_key(dir: &PathBuf) -> Option<Key> {
    // Preferred: OS keychain.
    match keyring::Entry::new("talaria", "vault") {
        Ok(entry) => match entry.get_password() {
            Ok(hex_key) => {
                if let Ok(bytes) = hex::decode(&hex_key) {
                    if bytes.len() == 32 {
                        return Some(*Key::from_slice(&bytes));
                    }
                }
                log::warn!("keychain vault key malformed; falling back to key file");
            },
            Err(keyring::Error::NoEntry) => {
                let key = ChaCha20Poly1305::generate_key(&mut OsRng);
                if entry.set_password(&hex::encode(key)).is_ok() {
                    return Some(key);
                }
                log::warn!("could not store vault key in keychain; falling back to key file");
            },
            Err(error) => {
                log::warn!("keychain unavailable ({error}); falling back to key file");
            },
        },
        Err(error) => log::warn!("keyring init failed ({error}); falling back to key file"),
    }

    // Fallback: key file with owner-only permissions.
    let key_path = dir.join("vault.key");
    if let Ok(hex_key) = fs::read_to_string(&key_path) {
        if let Ok(bytes) = hex::decode(hex_key.trim()) {
            if bytes.len() == 32 {
                return Some(*Key::from_slice(&bytes));
            }
        }
        log::warn!("vault key file malformed; refusing to touch vault");
        return None;
    }
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
    if fs::create_dir_all(dir).is_err() {
        return None;
    }
    let Ok(mut file) = fs::File::create(&key_path) else {
        return None;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    if file.write_all(hex::encode(key).as_bytes()).is_err() {
        return None;
    }
    Some(key)
}

impl Vault {
    pub fn load() -> Self {
        let dir = config_dir();
        let path = dir.join("vault.enc");
        let Some(key) = load_or_create_key(&dir) else {
            return Self { path, cipher: None, entries: Vec::new() };
        };
        let cipher = ChaCha20Poly1305::new(&key);

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
                let plain_path = dir.join("vault.json");
                match fs::read(&plain_path) {
                    Ok(plain) => {
                        log::warn!("importing plaintext vault.json into encrypted vault");
                        serde_json::from_slice(&plain).unwrap_or_default()
                    },
                    Err(_) => Vec::new(),
                }
            },
        };

        let mut vault = Self { path, cipher: Some(cipher), entries };
        vault.save();
        vault
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

    /// Entries whose URL host matches `domain` (exact or subdomain).
    pub fn matching(&self, domain: &str) -> Vec<CredentialEntry> {
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                url::Url::parse(&entry.url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
                    .is_some_and(|host| {
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
    fn domain_matching() {
        let vault = Vault {
            path: PathBuf::new(),
            cipher: None,
            entries: vec![
                entry("https://accounts.google.com/signin"),
                entry("https://github.com/login"),
            ],
        };
        assert_eq!(vault.matching("google.com").len(), 1);
        assert_eq!(vault.matching("accounts.google.com").len(), 1);
        assert_eq!(vault.matching("github.com").len(), 1);
        assert_eq!(vault.matching("example.com").len(), 0);
    }
}
