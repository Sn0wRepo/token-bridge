//! TOML configuration. See `relayer.example.toml`.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

use crate::types::Address;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Where scan progress is persisted between runs.
    #[serde(default = "default_state_file")]
    pub state_file: PathBuf,
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    pub chains: Vec<Chain>,
    #[serde(default)]
    pub solana: Option<Solana>,
    #[serde(default)]
    pub tokens: Vec<TokenRoute>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chain {
    pub name: String,
    pub chain_id: u64,
    pub rpc_url: String,
    pub vault: Address,
    /// Blocks to wait before a source event is considered final.
    #[serde(default = "default_confirmations")]
    pub confirmations: u64,
    /// First block to scan when no state exists yet (usually the vault deployment block).
    #[serde(default)]
    pub start_block: u64,
    #[serde(default = "default_range")]
    pub max_block_range: u64,
}

/// Solana is a destination only: jobs are appended to a JSONL outbox for an external executor.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Solana {
    pub chain_id: u64,
    pub outbox: PathBuf,
}

/// Maps a token on one chain to its counterpart on another.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRoute {
    pub src_chain_id: u64,
    pub src_token: Address,
    pub dst_chain_id: u64,
    /// EVM token address on the destination (zero = native coin). Ignored for Solana.
    #[serde(default)]
    pub dst_token: Option<Address>,
    /// SPL mint for Solana destinations (base58), passed through to the outbox.
    #[serde(default)]
    pub dst_mint: Option<String>,
}

fn default_state_file() -> PathBuf {
    PathBuf::from("relayer-state.json")
}
fn default_poll() -> u64 {
    12
}
fn default_confirmations() -> u64 {
    12
}
fn default_range() -> u64 {
    2_000
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.chains.is_empty(),
            "at least one [[chains]] entry is required"
        );
        let mut ids = std::collections::HashSet::new();
        for c in &self.chains {
            ensure!(ids.insert(c.chain_id), "duplicate chain_id {}", c.chain_id);
            ensure!(!c.vault.is_zero(), "{}: vault address is zero", c.name);
            ensure!(
                c.max_block_range > 0,
                "{}: max_block_range must be > 0",
                c.name
            );
        }
        if let Some(s) = &self.solana {
            ensure!(
                !ids.contains(&s.chain_id),
                "solana chain_id collides with an EVM chain"
            );
        }
        for t in &self.tokens {
            if self.is_solana(t.dst_chain_id) {
                ensure!(t.dst_mint.is_some(), "token route to Solana needs dst_mint");
            } else if self.chain(t.dst_chain_id).is_none() {
                bail!("token route to unknown chain {}", t.dst_chain_id);
            }
        }
        Ok(())
    }

    pub fn chain(&self, id: u64) -> Option<&Chain> {
        self.chains.iter().find(|c| c.chain_id == id)
    }

    pub fn is_solana(&self, id: u64) -> bool {
        self.solana.as_ref().is_some_and(|s| s.chain_id == id)
    }

    /// Destination token for an EVM route. Native maps to native unless configured otherwise.
    pub fn dst_token(
        &self,
        src_chain: u64,
        src_token: &Address,
        dst_chain: u64,
    ) -> Option<Address> {
        let route = self.tokens.iter().find(|t| {
            t.src_chain_id == src_chain && t.src_token == *src_token && t.dst_chain_id == dst_chain
        });
        match route {
            Some(r) => r.dst_token,
            None if src_token.is_zero() => Some(Address::default()),
            None => None,
        }
    }

    pub fn dst_mint(&self, src_chain: u64, src_token: &Address, dst_chain: u64) -> Option<&str> {
        self.tokens
            .iter()
            .find(|t| {
                t.src_chain_id == src_chain
                    && t.src_token == *src_token
                    && t.dst_chain_id == dst_chain
            })
            .and_then(|t| t.dst_mint.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let text = include_str!("../relayer.example.toml");
        let cfg: Config = toml::from_str(text).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.chains.len(), 2);
        assert!(cfg.is_solana(1_399_811_149));
        assert_eq!(
            cfg.dst_token(1, &Address::default(), 8453),
            Some(Address::default())
        );
    }
}
