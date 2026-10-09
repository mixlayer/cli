use std::collections::HashMap;

use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultEntry {
    pub token: String,
    pub value: String,
    pub kind: String,
}

pub trait Vault {
    fn ensure_entry(&mut self, kind: &str, value: &str) -> Result<VaultEntry>;
    fn save_entry(&mut self, e: VaultEntry) -> Result<()>;
    fn token_map(&self) -> Result<HashMap<String, String>>;
}

pub fn value_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.trim().as_bytes()))
}

#[derive(Default)]
pub struct MemoryVault {
    by_hash: HashMap<String, VaultEntry>,
}

impl Vault for MemoryVault {
    fn ensure_entry(&mut self, kind: &str, value: &str) -> Result<VaultEntry> {
        let hash = value_hash(value);
        if let Some(e) = self.by_hash.get(&hash) {
            return Ok(e.clone());
        }
        let tokens = self.token_map()?;
        for _ in 0..10 {
            let mut suffix = [0u8; 4];
            getrandom::fill(&mut suffix).map_err(|e| anyhow!("vault entropy: {e}"))?;
            let token = format!("{}_{}", kind, hex::encode(suffix));
            if !tokens.contains_key(&token) {
                let entry = VaultEntry {
                    token,
                    value: value.into(),
                    kind: kind.into(),
                };
                self.by_hash.insert(hash, entry.clone());
                return Ok(entry);
            }
        }
        Err(anyhow!("could not allocate unique vault token for {kind}"))
    }

    fn save_entry(&mut self, e: VaultEntry) -> Result<()> {
        let hash = value_hash(&e.value);
        if self
            .by_hash
            .iter()
            .any(|(other_hash, old)| *other_hash != hash && old.token == e.token)
        {
            return Err(anyhow!("vault token already exists: {}", e.token));
        }
        if let Some(old) = self.by_hash.get_mut(&hash) {
            old.token = e.token;
            old.kind = e.kind;
        } else {
            self.by_hash.insert(hash, e);
        }
        Ok(())
    }

    fn token_map(&self) -> Result<HashMap<String, String>> {
        Ok(self
            .by_hash
            .values()
            .map(|e| (e.token.clone(), e.value.clone()))
            .collect())
    }
}
