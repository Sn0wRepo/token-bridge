//! The relay loop: scan finalized source blocks → decode → check `processed` → release / hand off.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::abi::{
    encode_processed, encode_release, event_topic, transfer_id, BridgeRequested, RawLog,
    BRIDGE_REQUESTED_SIG,
};
use crate::config::{Chain, Config};
use crate::rpc::Rpc;
use crate::tx::{Eip1559Tx, Signer};
use crate::types::{quantity, Address, H256, U256};

/// Last fully processed block per source chain id.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub last_block: BTreeMap<u64, u64>,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                serde_json::from_str(&t).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Atomic write (temp file + rename) so a crash never leaves a truncated state file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// What happened to one event.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Released(H256),
    WouldRelease,
    AlreadyProcessed,
    QueuedForSolana,
    WouldQueueForSolana,
    Skipped(String),
}

pub struct Relayer {
    cfg: Config,
    rpcs: BTreeMap<u64, Rpc>,
    signer: Option<Signer>,
    state: State,
}

impl Relayer {
    /// `signer = None` runs in dry-run mode: everything is checked, nothing is sent.
    pub fn new(cfg: Config, signer: Option<Signer>) -> Result<Self> {
        let rpcs = cfg
            .chains
            .iter()
            .map(|c| (c.chain_id, Rpc::new(&c.rpc_url)))
            .collect();
        let state = State::load(&cfg.state_file)?;
        Ok(Relayer {
            cfg,
            rpcs,
            signer,
            state,
        })
    }

    fn rpc(&self, chain_id: u64) -> Result<&Rpc> {
        self.rpcs
            .get(&chain_id)
            .ok_or_else(|| anyhow!("no RPC for chain {chain_id}"))
    }

    /// Confirms every RPC is reachable and reports the chain id the config expects.
    pub fn check(&self) -> Result<()> {
        for c in &self.cfg.chains {
            let rpc = self.rpc(c.chain_id)?;
            let id = rpc
                .chain_id()
                .with_context(|| format!("{}: RPC unreachable", c.name))?;
            anyhow::ensure!(
                id == c.chain_id,
                "{}: RPC reports chain {id}, config says {}",
                c.name,
                c.chain_id
            );
            let head = rpc.block_number()?;
            log(&format!(
                "{} ok: chain {id}, head {head}, vault {}",
                c.name, c.vault
            ));
            if let Some(s) = &self.signer {
                let bal = rpc.call("eth_getBalance", json!([s.address.to_string(), "latest"]))?;
                let wei = quantity(bal.as_str().unwrap_or("0x0"))?;
                log(&format!("  relayer {} balance {} wei", s.address, wei));
            }
        }
        Ok(())
    }

    /// One pass over every source chain. Returns the outcomes in processing order.
    pub fn tick(&mut self) -> Result<Vec<Outcome>> {
        let topic = event_topic(BRIDGE_REQUESTED_SIG);
        let mut outcomes = Vec::new();
        let chains: Vec<(u64, u64, u64, u64, Address, String)> = self
            .cfg
            .chains
            .iter()
            .map(|c| {
                (
                    c.chain_id,
                    c.confirmations,
                    c.start_block,
                    c.max_block_range,
                    c.vault,
                    c.name.clone(),
                )
            })
            .collect();
        for (src, confirmations, start_block, range, vault, name) in chains {
            let head = self.rpc(src)?.block_number()?;
            let safe = head.saturating_sub(confirmations);
            let mut from = self
                .state
                .last_block
                .get(&src)
                .map_or(start_block, |b| b + 1);
            while from <= safe {
                let to = safe.min(from + range - 1);
                let logs = self.rpc(src)?.logs(&vault, &topic, from, to)?;
                for raw in &logs {
                    let outcome = self
                        .handle(src, raw)
                        .with_context(|| format!("{name}: tx {}", raw.transaction_hash))?;
                    outcomes.push(outcome);
                }
                // Only advance once every event in the range is settled; a failure retries the range,
                // and the on-chain `processed` check makes retries idempotent.
                self.state.last_block.insert(src, to);
                self.state.save(&self.cfg.state_file)?;
                from = to + 1;
            }
        }
        Ok(outcomes)
    }

    fn handle(&self, src: u64, raw: &RawLog) -> Result<Outcome> {
        let ev = BridgeRequested::decode(raw)?;
        let Some(dst) = ev
            .dst_chain_id
            .to_u128()
            .and_then(|v| u64::try_from(v).ok())
        else {
            return Ok(Outcome::Skipped("destination id out of range".into()));
        };
        let id = transfer_id(src, &ev.nonce);
        let tag = format!("[{src}#{}]", ev.nonce);

        if self.cfg.is_solana(dst) {
            return self.queue_solana(src, dst, &ev, &id, raw, &tag);
        }
        let Some(dst_chain) = self.cfg.chain(dst) else {
            log(&format!("{tag} skip: unknown destination {dst}"));
            return Ok(Outcome::Skipped(format!("unknown destination {dst}")));
        };
        let Some(to) = ev.evm_recipient() else {
            log(&format!(
                "{tag} skip: recipient {} is not an EVM address",
                ev.recipient
            ));
            return Ok(Outcome::Skipped("non-EVM recipient".into()));
        };
        let Some(token) = self.cfg.dst_token(src, &ev.token, dst) else {
            log(&format!(
                "{tag} skip: no token route for {} to chain {dst}",
                ev.token
            ));
            return Ok(Outcome::Skipped("no token route".into()));
        };

        let rpc = self.rpc(dst)?;
        let done = rpc.eth_call(&dst_chain.vault, &encode_processed(&id))?;
        if done.last() == Some(&1) {
            log(&format!("{tag} already processed on {}", dst_chain.name));
            return Ok(Outcome::AlreadyProcessed);
        }

        let data = encode_release(src, &ev.nonce, &token, &to, &ev.amount);
        let Some(signer) = &self.signer else {
            log(&format!(
                "{tag} dry-run: release {} of {token} to {to} on {}",
                ev.amount, dst_chain.name
            ));
            return Ok(Outcome::WouldRelease);
        };
        let hash = send(rpc, dst_chain, signer, data)?;
        log(&format!(
            "{tag} released {} to {to} on {}: {hash}",
            ev.amount, dst_chain.name
        ));
        Ok(Outcome::Released(hash))
    }

    fn queue_solana(
        &self,
        src: u64,
        dst: u64,
        ev: &BridgeRequested,
        id: &H256,
        raw: &RawLog,
        tag: &str,
    ) -> Result<Outcome> {
        let sol = self.cfg.solana.as_ref().expect("checked by is_solana");
        let mint = self.cfg.dst_mint(src, &ev.token, dst);
        if !ev.token.is_zero() && mint.is_none() {
            log(&format!(
                "{tag} skip: no SPL mint configured for {}",
                ev.token
            ));
            return Ok(Outcome::Skipped("no SPL mint".into()));
        }
        if self.signer.is_none() {
            log(&format!(
                "{tag} dry-run: would queue {} for Solana recipient {}",
                ev.amount,
                base58(&ev.recipient.0)
            ));
            return Ok(Outcome::WouldQueueForSolana);
        }
        // Never queue the same transfer twice, even after a full rescan.
        if let Ok(existing) = std::fs::read_to_string(&sol.outbox) {
            let needle = format!("\"transferId\":\"{id}\"");
            if existing.lines().any(|l| l.contains(&needle)) {
                log(&format!("{tag} already queued for Solana"));
                return Ok(Outcome::AlreadyProcessed);
            }
        }
        let job = json!({
            "transferId": id.to_string(),
            "srcChainId": src,
            "srcNonce": ev.nonce.to_string(),
            "srcTx": raw.transaction_hash.to_string(),
            "srcToken": ev.token.to_string(),
            "mint": mint,
            "amount": ev.amount.to_string(),
            "recipient": base58(&ev.recipient.0),
        });
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&sol.outbox)
            .with_context(|| format!("opening {}", sol.outbox.display()))?;
        writeln!(f, "{job}")?;
        log(&format!(
            "{tag} queued for Solana executor -> {}",
            sol.outbox.display()
        ));
        Ok(Outcome::QueuedForSolana)
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.cfg.poll_interval_secs)
    }
}

fn send(rpc: &Rpc, chain: &Chain, signer: &Signer, data: Vec<u8>) -> Result<H256> {
    let gas = rpc.estimate_gas(&signer.address, &chain.vault, &data)?;
    let priority = rpc.max_priority_fee().unwrap_or(1_000_000_000);
    let max_fee = rpc.gas_price()?.saturating_mul(2).max(priority);
    let tx = Eip1559Tx {
        chain_id: chain.chain_id,
        nonce: rpc.nonce(&signer.address)?,
        max_priority_fee_per_gas: priority,
        max_fee_per_gas: max_fee,
        gas_limit: gas + gas / 5,
        to: chain.vault,
        value: U256::ZERO,
        data,
    };
    let hash = rpc.send_raw(&tx.sign(signer)?)?;
    anyhow::ensure!(
        rpc.wait_receipt(&hash, Duration::from_secs(180))?,
        "release reverted: {hash}"
    );
    Ok(hash)
}

/// Bitcoin-alphabet base58, used for Solana public keys.
pub fn base58(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &b in bytes {
        let mut carry = u32::from(b);
        for d in digits.iter_mut() {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat('1').take(zeros));
    out.extend(digits.iter().rev().map(|d| ALPHABET[*d as usize] as char));
    out
}

fn log(msg: &str) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    eprintln!("{secs} {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base58_known_values() {
        assert_eq!(base58(&[0, 0, 1]), "112");
        assert_eq!(base58(b"hello world"), "StV1DL6CwTryKyV");
        assert_eq!(base58(&[0u8; 32]), "11111111111111111111111111111111");
    }
}
