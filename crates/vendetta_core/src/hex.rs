const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_DIGITS[(b >> 4) as usize] as char);
        s.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, HexError> {
    if !s.len().is_multiple_of(2) {
        return Err(HexError::InvalidLength);
    }

    let mut bytes = Vec::with_capacity(s.len() / 2);
    let (chunks, _) = s.as_bytes().as_chunks::<2>();
    for chunk in chunks {
        let high = hex_val(chunk[0]).ok_or(HexError::InvalidChar)?;
        let low = hex_val(chunk[1]).ok_or(HexError::InvalidChar)?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    #[error("odd hex string length")]
    InvalidLength,
    #[error("invalid hexadecimal character")]
    InvalidChar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encode_and_decode_roundtrip() {
        let raw = b"Vendetta Telegram Archive";
        let encoded = encode_hex(raw);
        assert_eq!(
            encoded,
            "56656e64657474612054656c656772616d2041726368697665"
        );
        let decoded = decode_hex(&encoded).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn hex_decode_rejects_invalid_inputs() {
        assert_eq!(decode_hex("abc"), Err(HexError::InvalidLength));
        assert_eq!(decode_hex("zz"), Err(HexError::InvalidChar));
        assert_eq!(decode_hex(""), Ok(Vec::new()));
    }
}
