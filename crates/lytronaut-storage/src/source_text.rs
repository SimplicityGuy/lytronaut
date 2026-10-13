//! Lossless source-text projection helpers for PostgreSQL's NUL-free TEXT type.
//! Original records remain opaque bytes; these helpers never alter original files.

const HEX: &[u8; 16] = b"0123456789abcdef";

pub(crate) fn encode_key(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 15)] as char);
    }
    encoded
}

pub(crate) fn decode_key(value: &str) -> Result<String, &'static str> {
    if !value.len().is_multiple_of(2) {
        return Err("invalid application source-key encoding");
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        fn nibble(byte: u8) -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            }
        }
        let high = nibble(pair[0]).ok_or("invalid application source-key encoding")?;
        let low = nibble(pair[1]).ok_or("invalid application source-key encoding")?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| "invalid application source-key encoding")
}

/// SQLite substr and LIKE stop at the first NUL. Its NOCASE collation compares
/// ASCII-folded prefixes, then the complete original byte lengths when equal.
/// PostgreSQL uses these explicit derived values rather than Unicode lower/ILIKE.
pub(crate) struct TextProjection {
    pub(crate) original: Vec<u8>,
    pub(crate) display: String,
    pub(crate) folded_prefix: Vec<u8>,
    pub(crate) original_bytes: i64,
}
impl TextProjection {
    pub(crate) fn new(value: &str, display_characters: usize) -> Self {
        let prefix = value.split('\0').next().unwrap_or("");
        Self {
            original: value.as_bytes().to_vec(),
            display: prefix.chars().take(display_characters).collect(),
            folded_prefix: prefix.as_bytes().to_ascii_lowercase(),
            // The Desktop decoder bounds source cells at one MiB before projection.
            original_bytes: value.len() as i64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_round_trip_nul_unicode_controls_and_all_ascii_bytes() {
        let ascii: String = (0_u8..=127).map(char::from).collect();
        for value in ["", "a\0b", "雪 🌌 café", "\u{e000}\u{1}0", ascii.as_str()] {
            let encoded = encode_key(value);
            assert_eq!(decode_key(&encoded).unwrap(), value);
            assert!(!encoded.contains('\0'));
        }
        for invalid in ["0", "GG", "FF", "ff", "c0af", "ed a0", "\0\0"] {
            assert!(decode_key(invalid).is_err());
        }
    }

    #[test]
    fn key_encoding_retains_binary_identity_order() {
        let values = ["", "a", "a\0", "a\u{1}", "A", "雪", "é", "🌌"];
        for a in values {
            for b in values {
                assert_eq!(
                    a.as_bytes().cmp(b.as_bytes()),
                    encode_key(a).cmp(&encode_key(b))
                );
            }
        }
    }

    #[test]
    fn projections_keep_originals_and_sqlite_display_search_semantics() {
        let value = "AbC café 雪\0After NUL";
        let projection = TextProjection::new(value, 5);
        assert_eq!(projection.original, value.as_bytes());
        assert_eq!(projection.display, "AbC c");
        assert_eq!(projection.folded_prefix, "abc café 雪".as_bytes());
        assert_eq!(projection.original_bytes, value.len() as i64);
        let unicode = TextProjection::new("雪🌌é", 2);
        assert_eq!(unicode.display, "雪🌌");
        assert_eq!(unicode.folded_prefix, "雪🌌é".as_bytes());
        assert_eq!(TextProjection::new("\0hidden", 256).display, "");
    }
}
