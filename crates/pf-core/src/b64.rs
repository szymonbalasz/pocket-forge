//! Standard base64 decoding for pasted image data (`project_create_from_clipboard.data_b64`).
//! A few lines in-crate instead of a new dependency (AGENTS §3.5).

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
    use super::decode;

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
