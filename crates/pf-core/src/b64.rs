//! Standard base64 for binary bodies over IPC (`project_create_from_clipboard.data_b64`, EDT
//! mask bytes). A few lines in-crate instead of a new dependency (AGENTS §3.5).

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes standard base64 with padding.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= n {
                ALPHABET[(v >> (18 - 6 * i) & 63) as usize]
            } else {
                b'='
            });
        }
    }
    // Only ASCII from the alphabet and '=' was pushed.
    String::from_utf8(out).unwrap_or_default()
}

fn value(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    } as u32)
}

/// Decodes standard (or URL-safe) base64. Whitespace is ignored, padding is optional, and an
/// optional `data:…;base64,` prefix is skipped. `None` on any other character.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let body = match text.find(";base64,") {
        Some(i) if text.starts_with("data:") => &text[i + 8..],
        _ => text,
    };
    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut padding = false;
    for c in body.bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding = true;
            continue;
        }
        if padding {
            return None;
        }
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (bits < 6).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn fr_edt_18_encode_round_trips() {
        assert_eq!(encode(b"hello"), "aGVsbG8=");
        assert_eq!(encode(b"hello!"), "aGVsbG8h");
        assert_eq!(encode(b""), "");
        let all: Vec<u8> = (0..=255).cycle().take(1000).collect();
        for n in [0, 1, 2, 3, 4, 999, 1000] {
            assert_eq!(decode(&encode(&all[..n])).unwrap(), &all[..n]);
        }
    }

    #[test]
    fn fr_prj_10_decodes_base64_variants() {
        assert_eq!(decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode("aGVs\nbG8h").unwrap(), b"hello!");
        assert_eq!(decode("data:image/png;base64,AAEC").unwrap(), [0, 1, 2]);
        assert_eq!(decode("").unwrap(), b"");
        assert!(decode("a$b=").is_none());
        assert!(decode("aG=Vs").is_none());
        assert!(decode("a").is_none());
    }
}
