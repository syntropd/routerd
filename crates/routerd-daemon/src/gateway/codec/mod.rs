//! Media codecs for gateway requests and responses.

pub mod audio;
pub mod images;

pub use audio::{encode_wav_header, extract_multipart_field, parse_multipart_boundary};
pub use images::{base64_decode, base64_encode, parse_dimensions};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_reexports() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        let wav = encode_wav_header(8000, 1, b"raw");
        assert!(wav.starts_with(b"RIFF"));
    }
}
