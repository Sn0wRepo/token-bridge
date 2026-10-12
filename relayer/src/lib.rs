//! Reference relayer for `BridgeVault.sol`.
//!
//! Modules are small and dependency-light on purpose so each piece can be audited on its own:
//!
//! * [`abi`]    – event decoding and calldata encoding for the vault
//! * [`rlp`]    – minimal RLP encoder
//! * [`tx`]     – EIP-1559 transaction building and secp256k1 signing
//! * [`rpc`]    – blocking JSON-RPC client
//! * [`config`] – TOML configuration
//! * [`relay`]  – the scan → verify → release loop

pub mod abi;
pub mod config;
pub mod relay;
pub mod rlp;
pub mod rpc;
pub mod tx;
pub mod types;

pub use types::{keccak256, Address, H256, U256};
