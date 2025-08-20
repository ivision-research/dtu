pub const HEX_BYTES: &[u8; 16] = HEX_BYTES_LOWER;

pub const HEX_BYTES_LOWER: &[u8; 16] = &[
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'a', b'b', b'c', b'd', b'e', b'f',
];

pub const HEX_BYTES_UPPER: &[u8; 16] = &[
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'A', b'B', b'C', b'D', b'E', b'F',
];

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut into = String::with_capacity(bytes.len() * 2);

    for b in bytes {
        let high = (b & 0xF0) >> 4;
        let low = b & 0xF;
        into.push(HEX_BYTES_LOWER[high as usize] as char);
        into.push(HEX_BYTES_LOWER[low as usize] as char);
    }
    into
}

fn decode_nibble(nibble: u8) -> Option<u8> {
    for (i, c) in HEX_BYTES_LOWER.iter().enumerate() {
        if *c == nibble {
            return Some(i as u8);
        }
        if i > 9 {
            if HEX_BYTES_UPPER[i] == nibble {
                return Some(i as u8);
            }
        }
    }
    None
}

fn decode_nibbles(high: u8, low: u8) -> Option<u8> {
    Some((decode_nibble(high)? << 4) | decode_nibble(low)?)
}

pub fn bytes_from_hex(mut ashex: &str) -> Option<Vec<u8>> {
    if ashex.starts_with("0x") || ashex.starts_with("0X") {
        ashex = &ashex[2..];
    }
    let len = ashex.len();
    if len % 2 != 0 {
        return None;
    }
    let mut into = Vec::with_capacity(ashex.len() / 2);
    let mut idx = 0;

    let bytes = ashex.as_bytes();

    while idx < len {
        let high = bytes[idx];
        let low = bytes[idx + 1];
        let byte = decode_nibbles(high, low)?;
        into.push(byte);
        idx += 2;
    }

    Some(into)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bytes_to_hex() {
        let bytes = &[0x00, 0x7F, 0x80, 0xFF];

        let as_hex = bytes_to_hex(bytes);
        assert_eq!(as_hex.as_str(), "007f80ff");
    }

    #[test]
    fn test_bytes_from_hex() {
        assert_eq!(bytes_from_hex("hello world"), None);
        let expected: Vec<u8> = vec![0xCA, 0xFE, 0xC0, 0xDE];
        assert_eq!(bytes_from_hex("0xcafec0de").unwrap(), expected);
        assert_eq!(bytes_from_hex("0XCAFEC0DE").unwrap(), expected);
        assert_eq!(bytes_from_hex("cafec0de").unwrap(), expected);
        assert_eq!(bytes_from_hex("CAFEC0DE").unwrap(), expected);

        assert_eq!(bytes_from_hex("cafEC0de").unwrap(), expected);
        assert_eq!(bytes_from_hex("CafEC0dE").unwrap(), expected);
    }
}
