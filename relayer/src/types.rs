//! Fixed-size primitives with hex (de)serialisation.

use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, bail, ensure, Result};
use serde::{de, Deserialize, Deserializer};
use tiny_keccak::{Hasher, Keccak};

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();
    k.update(data);
    let mut out = [0u8; 32];
    k.finalize(&mut out);
    out
}

pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn from_hex(s: &str) -> Result<Vec<u8>> {
    let body = s.strip_prefix("0x").unwrap_or(s);
    ensure!(body.len() % 2 == 0, "odd-length hex: {s}");
    body.as_bytes()
        .chunks(2)
        .map(|p| {
            let hi = (p[0] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow!("bad hex: {s}"))?;
            let lo = (p[1] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow!("bad hex: {s}"))?;
            Ok((hi * 16 + lo) as u8)
        })
        .collect()
}

/// Parses an RPC quantity (`0x1a`) into a u128.
pub fn quantity(s: &str) -> Result<u128> {
    let body = s
        .strip_prefix("0x")
        .ok_or_else(|| anyhow!("quantity without 0x: {s}"))?;
    if body.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(body, 16).map_err(|e| anyhow!("bad quantity {s}: {e}"))
}

macro_rules! fixed_bytes {
    ($name:ident, $n:expr) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub [u8; $n]);

        impl $name {
            pub fn from_slice(b: &[u8]) -> Result<Self> {
                ensure!(b.len() == $n, "expected {} bytes, got {}", $n, b.len());
                let mut out = [0u8; $n];
                out.copy_from_slice(b);
                Ok($name(out))
            }

            pub fn is_zero(&self) -> bool {
                self.0.iter().all(|b| *b == 0)
            }
        }

        impl FromStr for $name {
            type Err = anyhow::Error;
            fn from_str(s: &str) -> Result<Self> {
                Self::from_slice(&from_hex(s)?)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&to_hex(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), to_hex(&self.0))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(de::Error::custom)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(
                &self,
                s: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_string())
            }
        }
    };
}

fixed_bytes!(Address, 20);
fixed_bytes!(H256, 32);

/// Big-endian 256-bit unsigned integer, used only as an opaque ABI word plus small arithmetic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct U256(pub [u8; 32]);

impl U256 {
    pub const ZERO: U256 = U256([0u8; 32]);

    pub fn from_u128(v: u128) -> Self {
        let mut out = [0u8; 32];
        out[16..].copy_from_slice(&v.to_be_bytes());
        U256(out)
    }

    /// Value as u128 when it fits.
    pub fn to_u128(&self) -> Option<u128> {
        if self.0[..16].iter().any(|b| *b != 0) {
            return None;
        }
        Some(u128::from_be_bytes(
            self.0[16..].try_into().expect("16 bytes"),
        ))
    }

    /// Big-endian bytes without leading zeroes (RLP integer form).
    pub fn trimmed(&self) -> &[u8] {
        let first = self.0.iter().position(|b| *b != 0).unwrap_or(32);
        &self.0[first..]
    }
}

impl fmt::Display for U256 {
    /// Decimal rendering (schoolbook division, fine for logging).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(v) = self.to_u128() {
            return write!(f, "{v}");
        }
        let mut digits = Vec::new();
        let mut n = self.0;
        while n.iter().any(|b| *b != 0) {
            let mut rem = 0u32;
            for b in n.iter_mut() {
                let cur = (rem << 8) | u32::from(*b);
                *b = (cur / 10) as u8;
                rem = cur % 10;
            }
            digits.push(b'0' + rem as u8);
        }
        digits.reverse();
        f.write_str(std::str::from_utf8(&digits).expect("ascii"))
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "U256({self})")
    }
}

impl FromStr for U256 {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        if let Some(hex) = s.strip_prefix("0x") {
            let bytes = from_hex(
                if hex.len() % 2 == 1 {
                    format!("0{hex}")
                } else {
                    hex.to_string()
                }
                .as_str(),
            )?;
            ensure!(bytes.len() <= 32, "U256 overflow: {s}");
            let mut out = [0u8; 32];
            out[32 - bytes.len()..].copy_from_slice(&bytes);
            return Ok(U256(out));
        }
        let mut out = [0u8; 32];
        for c in s.chars() {
            let d = c.to_digit(10).ok_or_else(|| anyhow!("bad decimal: {s}"))?;
            let mut carry = d;
            for b in out.iter_mut().rev() {
                let v = u32::from(*b) * 10 + carry;
                *b = (v & 0xff) as u8;
                carry = v >> 8;
            }
            if carry != 0 {
                bail!("U256 overflow: {s}");
            }
        }
        Ok(U256(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u256_decimal_round_trip() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(max.parse::<U256>().unwrap().to_string(), max);
        assert_eq!("0x0".parse::<U256>().unwrap(), U256::ZERO);
        assert_eq!("1000".parse::<U256>().unwrap().to_u128(), Some(1000));
        assert!(
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
                .parse::<U256>()
                .is_err()
        );
    }

    #[test]
    fn quantities() {
        assert_eq!(quantity("0x0").unwrap(), 0);
        assert_eq!(quantity("0x1a").unwrap(), 26);
        assert!(quantity("1a").is_err());
    }
}
