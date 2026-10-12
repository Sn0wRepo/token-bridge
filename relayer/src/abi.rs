//! ABI glue for `BridgeVault.sol`: the `BridgeRequested` event and the calls the relayer makes.

use anyhow::{ensure, Result};
use serde::Deserialize;

use crate::types::{from_hex, keccak256, Address, H256, U256};

pub const BRIDGE_REQUESTED_SIG: &str =
    "BridgeRequested(uint256,address,address,uint256,uint256,uint256,bytes32)";

pub fn event_topic(sig: &str) -> H256 {
    H256(keccak256(sig.as_bytes()))
}

pub fn selector(sig: &str) -> [u8; 4] {
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

/// Raw log as returned by `eth_getLogs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawLog {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: String,
    pub block_number: String,
    pub transaction_hash: H256,
    pub log_index: String,
}

/// Decoded `BridgeRequested` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequested {
    pub nonce: U256,
    pub sender: Address,
    pub token: Address,
    pub amount: U256,
    pub fee: U256,
    pub dst_chain_id: U256,
    /// EVM: left-padded address. Solana: raw 32-byte public key.
    pub recipient: H256,
}

impl BridgeRequested {
    pub fn decode(log: &RawLog) -> Result<Self> {
        ensure!(
            log.topics.len() == 4,
            "expected 4 topics, got {}",
            log.topics.len()
        );
        ensure!(
            log.topics[0] == event_topic(BRIDGE_REQUESTED_SIG),
            "not a BridgeRequested log"
        );
        let data = from_hex(&log.data)?;
        ensure!(
            data.len() == 128,
            "expected 128 data bytes, got {}",
            data.len()
        );
        let word =
            |i: usize| -> [u8; 32] { data[i * 32..(i + 1) * 32].try_into().expect("32 bytes") };
        Ok(BridgeRequested {
            nonce: U256(log.topics[1].0),
            sender: topic_address(&log.topics[2])?,
            token: topic_address(&log.topics[3])?,
            amount: U256(word(0)),
            fee: U256(word(1)),
            dst_chain_id: U256(word(2)),
            recipient: H256(word(3)),
        })
    }

    /// Recipient as an EVM address, if the upper 12 bytes are zero.
    pub fn evm_recipient(&self) -> Option<Address> {
        if self.recipient.0[..12].iter().all(|b| *b == 0) {
            Address::from_slice(&self.recipient.0[12..]).ok()
        } else {
            None
        }
    }
}

fn topic_address(t: &H256) -> Result<Address> {
    ensure!(
        t.0[..12].iter().all(|b| *b == 0),
        "topic is not an address: {t}"
    );
    Address::from_slice(&t.0[12..])
}

fn word_u256(v: &U256) -> [u8; 32] {
    v.0
}

fn word_addr(a: &Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&a.0);
    w
}

/// `keccak256(abi.encode(srcChainId, srcNonce))`, the vault's replay-protection key.
pub fn transfer_id(src_chain_id: u64, src_nonce: &U256) -> H256 {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&U256::from_u128(u128::from(src_chain_id)).0);
    buf[32..].copy_from_slice(&src_nonce.0);
    H256(keccak256(&buf))
}

/// Calldata for `release(uint256,uint256,address,address,uint256)`.
pub fn encode_release(
    src_chain_id: u64,
    src_nonce: &U256,
    token: &Address,
    to: &Address,
    amount: &U256,
) -> Vec<u8> {
    let mut out = selector("release(uint256,uint256,address,address,uint256)").to_vec();
    out.extend_from_slice(&word_u256(&U256::from_u128(u128::from(src_chain_id))));
    out.extend_from_slice(&word_u256(src_nonce));
    out.extend_from_slice(&word_addr(token));
    out.extend_from_slice(&word_addr(to));
    out.extend_from_slice(&word_u256(amount));
    out
}

/// Calldata for the public getter `processed(bytes32)`.
pub fn encode_processed(id: &H256) -> Vec<u8> {
    let mut out = selector("processed(bytes32)").to_vec();
    out.extend_from_slice(&id.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::to_hex;

    /// Log shaped like a `bridgeNative` of 1 ETH to a Solana key with a 0.1% fee.
    fn sample_log() -> RawLog {
        RawLog {
            address: "0x5fbdb2315678afecb367f032d93f642f64180aa3"
                .parse()
                .unwrap(),
            topics: vec![
                event_topic(BRIDGE_REQUESTED_SIG),
                "0x0000000000000000000000000000000000000000000000000000000000000001"
                    .parse()
                    .unwrap(),
                "0x0000000000000000000000003000000000000000000000000000000000000003"
                    .parse()
                    .unwrap(),
                H256::default(),
            ],
            data: format!(
                "0x{}{}{}{}",
                "0000000000000000000000000000000000000000000000000ddd2935029d8000",
                "00000000000000000000000000000000000000000000000000038d7ea4c68000",
                "00000000000000000000000000000000000000000000000000000000536f6c4d",
                "7f2f8f1f5a7b1d0e3bf3c3f8f1d9e5b2a0c4d6e8f0a1b2c3d4e5f60718293a4b"
            ),
            block_number: "0x10".into(),
            transaction_hash: H256::default(),
            log_index: "0x0".into(),
        }
    }

    #[test]
    fn decodes_bridge_requested() {
        let ev = BridgeRequested::decode(&sample_log()).unwrap();
        assert_eq!(ev.nonce.to_u128(), Some(1));
        assert_eq!(
            ev.sender.to_string(),
            "0x3000000000000000000000000000000000000003"
        );
        assert!(ev.token.is_zero());
        assert_eq!(ev.amount.to_u128(), Some(999_000_000_000_000_000));
        assert_eq!(ev.fee.to_u128(), Some(1_000_000_000_000_000));
        assert_eq!(ev.dst_chain_id.to_u128(), Some(1_399_811_149));
        assert!(
            ev.evm_recipient().is_none(),
            "solana key is not an EVM address"
        );
    }

    #[test]
    fn rejects_wrong_topic() {
        let mut log = sample_log();
        log.topics[0] = H256::default();
        assert!(BridgeRequested::decode(&log).is_err());
    }

    #[test]
    fn selectors() {
        assert_eq!(to_hex(&selector("transfer(address,uint256)")), "0xa9059cbb");
    }
}
