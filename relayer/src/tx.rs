//! EIP-1559 (type 2) transactions and secp256k1 signing.

use anyhow::{anyhow, Result};
use k256::ecdsa::SigningKey;

use crate::rlp::{encode, int_bytes, Item};
use crate::types::{from_hex, keccak256, Address, H256, U256};

#[derive(Debug, Clone)]
pub struct Eip1559Tx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub to: Address,
    pub value: U256,
    pub data: Vec<u8>,
}

impl Eip1559Tx {
    fn fields(&self) -> (Vec<Vec<u8>>, &[u8], &[u8]) {
        let ints = vec![
            int_bytes(u128::from(self.chain_id)),
            int_bytes(u128::from(self.nonce)),
            int_bytes(self.max_priority_fee_per_gas),
            int_bytes(self.max_fee_per_gas),
            int_bytes(u128::from(self.gas_limit)),
        ];
        (ints, &self.to.0, self.value.trimmed())
    }

    fn rlp(&self, sig: Option<(u8, &[u8], &[u8])>) -> Vec<u8> {
        let (ints, to, value) = self.fields();
        let mut items: Vec<Item<'_>> = ints.iter().map(|b| Item::Bytes(b)).collect();
        items.push(Item::Bytes(to));
        items.push(Item::Bytes(value));
        items.push(Item::Bytes(&self.data));
        items.push(Item::List(vec![])); // access list
        let parity;
        if let Some((y, r, s)) = sig {
            parity = int_bytes(u128::from(y));
            items.push(Item::Bytes(&parity));
            items.push(Item::Bytes(r));
            items.push(Item::Bytes(s));
        }
        let mut out = vec![0x02];
        out.extend(encode(&Item::List(items)));
        out
    }

    /// Hash that gets signed: `keccak256(0x02 || rlp(fields))`.
    pub fn sighash(&self) -> H256 {
        H256(keccak256(&self.rlp(None)))
    }

    /// Signed raw transaction, ready for `eth_sendRawTransaction`.
    pub fn sign(&self, key: &Signer) -> Result<Vec<u8>> {
        let (sig, recid) = key
            .key
            .sign_prehash_recoverable(&self.sighash().0)
            .map_err(|e| anyhow!("signing failed: {e}"))?;
        let bytes = sig.to_bytes();
        let strip = |b: &[u8]| -> Vec<u8> {
            let first = b.iter().position(|x| *x != 0).unwrap_or(b.len());
            b[first..].to_vec()
        };
        let (r, s) = (strip(&bytes[..32]), strip(&bytes[32..]));
        Ok(self.rlp(Some((recid.to_byte(), &r, &s))))
    }
}

/// A secp256k1 key plus its Ethereum address.
pub struct Signer {
    key: SigningKey,
    pub address: Address,
}

impl Signer {
    pub fn from_hex(hex: &str) -> Result<Self> {
        let bytes = from_hex(hex.trim())?;
        let key =
            SigningKey::from_slice(&bytes).map_err(|_| anyhow!("invalid secp256k1 private key"))?;
        let point = key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&point.as_bytes()[1..]);
        let address = Address::from_slice(&hash[12..])?;
        Ok(Signer { key, address })
    }
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signer({})", self.address) // never print the key
    }
}
