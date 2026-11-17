//! Authenticated credential encryption. Keep credentials.key with the private data volume.
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::Aead};
use std::{
    io::{Read, Write},
    path::Path,
};

pub struct Vault {
    cipher: ChaCha20Poly1305,
}
impl Vault {
    pub fn open(path: &Path, credentials_exist: bool) -> Result<Self> {
        if !path.exists() {
            ensure!(
                !credentials_exist,
                "Credential key is missing; restore credentials.key from your backup"
            );
            let mut key = [0_u8; 32];
            getrandom::fill(&mut key)
                .map_err(|_| anyhow::anyhow!("Cannot generate credential key"))?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            file.write_all(&key)?;
            file.sync_all()?;
            #[cfg(unix)]
            std::fs::File::open(path.parent().unwrap())?.sync_all()?;
        }
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Credential key must be a regular file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "Credential key must have mode 600"
            );
        }
        ensure!(metadata.len() == 32, "Invalid credential key length");
        let mut key = [0_u8; 32];
        std::fs::File::open(path)?.read_exact(&mut key)?;
        Ok(Self {
            cipher: ChaCha20Poly1305::new((&key).into()),
        })
    }
    pub fn encrypt(&self, username: &str, password: &str) -> Result<Vec<u8>> {
        let mut nonce = [0_u8; 12];
        getrandom::fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("Cannot generate credential nonce"))?;
        let plaintext = serde_json::to_vec(&(username, password))?;
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
            .map_err(|_| anyhow::anyhow!("Credential encryption failed"))?;
        let mut stored = nonce.to_vec();
        stored.extend(ciphertext);
        Ok(stored)
    }
    pub fn decrypt(&self, stored: &[u8]) -> Result<(String, String)> {
        ensure!(stored.len() >= 28, "Invalid encrypted credentials");
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&stored[..12]), &stored[12..])
            .map_err(|_| {
                anyhow::anyhow!("Credential decryption failed; check database and key backup")
            })?;
        serde_json::from_slice(&plaintext).context("Invalid credential record")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_tamper_and_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        let vault = Vault::open(&path, false).unwrap();
        let mut blob = vault.encrypt("user", "sensitive-password").unwrap();
        assert!(!blob.windows(18).any(|s| s == b"sensitive-password"));
        assert_eq!(
            Vault::open(&path, true).unwrap().decrypt(&blob).unwrap(),
            ("user".into(), "sensitive-password".into())
        );
        blob[15] ^= 1;
        assert!(vault.decrypt(&blob).is_err());
        assert!(Vault::open(&dir.path().join("missing"), true).is_err());
    }
}
