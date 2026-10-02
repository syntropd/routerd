//! Pure-Rust Base64 encoding/decoding and dimension validation for gateway images.

/// Encodes binary byte slices into RFC 4648 Base64 strings.
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = ((c[0] as u32) << 16)
            | (if c.len() > 1 { (c[1] as u32) << 8 } else { 0 })
            | (if c.len() > 2 { c[2] as u32 } else { 0 });
        out.push(T[((b >> 18) & 0x3f) as usize] as char);
        out.push(T[((b >> 12) & 0x3f) as usize] as char);
        out.push(if c.len() > 1 {
            T[((b >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(b & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Decodes RFC 4648 Base64 strings into binary byte vectors.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let s = input.trim();
    if s.is_empty() || s.len() % 4 != 0 {
        return None;
    }
    let tbl = |b: u8| match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity((s.len() / 4) * 3);
    for c in s.as_bytes().chunks_exact(4) {
        let (b0, b1) = (tbl(c[0])?, tbl(c[1])?);
        let b2 = if c[2] == b'=' { 0 } else { tbl(c[2])? };
        let b3 = if c[3] == b'=' { 0 } else { tbl(c[3])? };
        let trip = ((b0 as u32) << 18) | ((b1 as u32) << 12) | ((b2 as u32) << 6) | (b3 as u32);
        out.push((trip >> 16) as u8);
        if c[2] != b'=' {
            out.push((trip >> 8) as u8);
        }
        if c[3] != b'=' {
            out.push(trip as u8);
        }
    }
    Some(out)
}

/// Parses and validates WxH dimensions within the 1..=4096 range.
pub fn parse_dimensions(size: Option<&str>) -> Result<(u32, u32), &'static str> {
    match size {
        None => Ok((512, 512)),
        Some(s) => {
            let mut p = s.split('x');
            let w = p.next().and_then(|v| v.trim().parse::<u32>().ok());
            let h = p.next().and_then(|v| v.trim().parse::<u32>().ok());
            match (w, h, p.next()) {
                (Some(w), Some(h), None) if (1..=4096).contains(&w) && (1..=4096).contains(&h) => {
                    Ok((w, h))
                }
                _ => Err("Invalid size: expected WxH with values between 1 and 4096"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_base64_and_dimensions() {
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_decode("Zm9v"), Some(b"foo".to_vec()));
        assert_eq!(base64_decode("bad!"), None);
        assert_eq!(parse_dimensions(Some("1024x1024")), Ok((1024, 1024)));
        assert_eq!(parse_dimensions(None), Ok((512, 512)));
        assert!(parse_dimensions(Some("9999x9999")).is_err());
    }
}
