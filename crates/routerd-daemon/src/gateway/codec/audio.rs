//! Audio codecs, WAV container encoding, and robust multipart/form-data extraction.

/// Assembles a standard 44-byte RIFF/WAVE header followed by raw PCM samples.
pub fn encode_wav_header(sample_rate: u32, channels: u16, pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let riff_size = 36 + data_len;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;

    let mut buf = Vec::with_capacity(44 + pcm.len());
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.extend_from_slice(pcm);
    buf
}

/// Parses the multipart boundary string from a Content-Type header value.
pub fn parse_multipart_boundary(content_type: &str) -> Option<&str> {
    for part in content_type.split(';') {
        let trimmed = part.trim();
        if let Some(rest) = trimmed.strip_prefix("boundary=") {
            let val = rest.trim_matches('"').trim();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Extracts a named field payload from a multipart/form-data body.
///
/// Bounded by the multipart boundary marker rather than premature CRLF sequences
/// inside binary audio samples.
pub fn extract_multipart_field<'a>(
    data: &'a [u8],
    name: &str,
    boundary: Option<&str>,
) -> Option<&'a [u8]> {
    let needle = format!("name=\"{name}\"");
    let pos = data.windows(needle.len()).position(|w| w == needle.as_bytes())?;
    let after_header = &data[pos + needle.len()..];
    let delim = b"\r\n\r\n";
    let body_start = after_header.windows(4).position(|w| w == delim)? + 4;
    let payload = &after_header[body_start..];

    let end = if let Some(b) = boundary {
        let marker = format!("\r\n--{b}");
        payload
            .windows(marker.len())
            .position(|w| w == marker.as_bytes())
            .unwrap_or(payload.len())
    } else {
        let marker = b"\r\n--";
        payload
            .windows(marker.len())
            .position(|w| w == marker)
            .unwrap_or(payload.len())
    };

    Some(&payload[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_wav_header_structure() {
        let dummy = vec![0x12, 0x34, 0x56, 0x78];
        let wav = encode_wav_header(16000, 1, &dummy);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[44..], &dummy[..]);
    }

    #[test]
    fn test_parse_multipart_boundary() {
        assert_eq!(
            parse_multipart_boundary("multipart/form-data; boundary=XYZ123"),
            Some("XYZ123")
        );
        assert_eq!(
            parse_multipart_boundary("multipart/form-data; boundary=\"quoted-bound\""),
            Some("quoted-bound")
        );
        assert_eq!(parse_multipart_boundary("application/json"), None);
    }

    #[test]
    fn test_extract_multipart_binary_with_internal_crlf() {
        let boundary = "----WebKitFormBoundaryX";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"file\"; filename=\"test.pcm\"\r\n");
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        let binary_data = vec![0x01, 0x0D, 0x0A, 0x02, 0x0D, 0x0A, 0x03, 0x04];
        body.extend_from_slice(&binary_data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let extracted = extract_multipart_field(&body, "file", Some(boundary));
        assert_eq!(extracted, Some(&binary_data[..]));
    }
}
