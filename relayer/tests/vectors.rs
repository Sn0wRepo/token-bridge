//! Cross-checks against ethers.js v6 output (vectors generated once and pinned here).
//! Key = Anvil/Hardhat test account #0. Never use it on a real network.

use bridge_relayer::abi::encode_release;
use bridge_relayer::tx::{Eip1559Tx, Signer};
use bridge_relayer::types::{keccak256, to_hex};
use bridge_relayer::{Address, U256};

const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const VAULT: &str = "0x5FbDB2315678afecb367f032d93F642f64180aa3";

fn release_data() -> Vec<u8> {
    let to: Address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        .parse()
        .unwrap();
    encode_release(
        1,
        &U256::from_u128(42),
        &Address::default(),
        &to,
        &U256::from_u128(999_000_000_000_000_000),
    )
}

#[test]
fn address_from_key() {
    let s = Signer::from_hex(TEST_KEY).unwrap();
    assert_eq!(
        s.address.to_string(),
        "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
    );
}

#[test]
fn release_calldata_matches_ethers() {
    assert_eq!(
        to_hex(&release_data()),
        "0x0258c04d0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c80000000000000000000000000000000000000000000000000ddd2935029d8000"
    );
}

#[test]
fn signed_release_tx_matches_ethers() {
    let tx = Eip1559Tx {
        chain_id: 8453,
        nonce: 7,
        max_priority_fee_per_gas: 1_500_000_000,
        max_fee_per_gas: 30_000_000_000,
        gas_limit: 120_000,
        to: VAULT.parse().unwrap(),
        value: U256::ZERO,
        data: release_data(),
    };
    let raw = tx.sign(&Signer::from_hex(TEST_KEY).unwrap()).unwrap();
    assert_eq!(
        to_hex(&raw),
        "0x02f90113822105078459682f008506fc23ac008301d4c0945fbdb2315678afecb367f032d93f642f64180aa380b8a40258c04d0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c80000000000000000000000000000000000000000000000000ddd2935029d8000c080a09314568340907fb7a54dba29b41dc10a68e096da1837d98ce1c8697a311c99f1a01afe027b363e390fbe461d3547e6a8a7f01f3eb18431c6101afd64d79a4a7aa0"
    );
    assert_eq!(
        to_hex(&keccak256(&raw)),
        "0x2613ea041be3944c8826e8276f73d88b797d8ffbf2f4c9c9c754ca8489f72bdc"
    );
}

#[test]
fn signed_value_transfer_matches_ethers() {
    let tx = Eip1559Tx {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1_500_000_000,
        max_fee_per_gas: 30_000_000_000,
        gas_limit: 120_000,
        to: VAULT.parse().unwrap(),
        value: U256::from_u128(1),
        data: vec![],
    };
    let raw = tx.sign(&Signer::from_hex(TEST_KEY).unwrap()).unwrap();
    assert_eq!(
        to_hex(&raw),
        "0x02f86c01808459682f008506fc23ac008301d4c0945fbdb2315678afecb367f032d93f642f64180aa30180c080a0dbea72f9a90249205e80e91fb46bf627d0211e74185bb640c6480aad67cddb7da0616c98a62d5dd30fa4c578ad5e1c2f11bb3a8f0538c0cce8bf709df4ba0158c6"
    );
}
