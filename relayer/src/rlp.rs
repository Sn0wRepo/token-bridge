//! Minimal RLP encoder (Ethereum Yellow Paper, appendix B). Encoding only; the relayer never decodes.

pub enum Item<'a> {
    Bytes(&'a [u8]),
    List(Vec<Item<'a>>),
}

/// Big-endian integer without leading zeroes (0 encodes as the empty string).
pub fn int_bytes(v: u128) -> Vec<u8> {
    let b = v.to_be_bytes();
    let first = b.iter().position(|x| *x != 0).unwrap_or(16);
    b[first..].to_vec()
}

pub fn encode(item: &Item<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(item, &mut out);
    out
}

fn encode_into(item: &Item<'_>, out: &mut Vec<u8>) {
    match item {
        Item::Bytes(b) if b.len() == 1 && b[0] < 0x80 => out.push(b[0]),
        Item::Bytes(b) => {
            header(0x80, b.len(), out);
            out.extend_from_slice(b);
        }
        Item::List(items) => {
            let mut body = Vec::new();
            for it in items {
                encode_into(it, &mut body);
            }
            header(0xc0, body.len(), out);
            out.extend_from_slice(&body);
        }
    }
}

fn header(offset: u8, len: usize, out: &mut Vec<u8>) {
    if len < 56 {
        out.push(offset + len as u8);
    } else {
        let be = int_bytes(len as u128);
        out.push(offset + 55 + be.len() as u8);
        out.extend_from_slice(&be);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vectors from the Ethereum wiki RLP page.
    #[test]
    fn spec_vectors() {
        assert_eq!(encode(&Item::Bytes(b"dog")), [0x83, b'd', b'o', b'g']);
        assert_eq!(
            encode(&Item::List(vec![Item::Bytes(b"cat"), Item::Bytes(b"dog")])),
            [0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']
        );
        assert_eq!(encode(&Item::Bytes(b"")), [0x80]);
        assert_eq!(encode(&Item::List(vec![])), [0xc0]);
        assert_eq!(encode(&Item::Bytes(&int_bytes(0))), [0x80]);
        assert_eq!(encode(&Item::Bytes(&int_bytes(15))), [0x0f]);
        assert_eq!(encode(&Item::Bytes(&int_bytes(1024))), [0x82, 0x04, 0x00]);
        let lorem = b"Lorem ipsum dolor sit amet, consectetur adipisicing elit";
        let enc = encode(&Item::Bytes(lorem));
        assert_eq!(&enc[..2], [0xb8, 0x38]);
        assert_eq!(&enc[2..], lorem);
    }
}
