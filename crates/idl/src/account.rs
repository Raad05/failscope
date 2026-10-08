//! Decodes the two on-chain IDL account layouts into IDL JSON bytes.

use std::io::Read;

use base64::Engine;
use flate2::read::{GzDecoder, ZlibDecoder};

use crate::IdlError;

/// IDL JSON larger than this is refused (guards against decompression bombs).
pub const MAX_IDL_BYTES: u64 = 16 * 1024 * 1024;

/// `sha256("account:IdlAccount")[..8]`, Anchor's legacy IDL account discriminator.
pub const LEGACY_DISCRIMINATOR: [u8; 8] = [0x18, 0x46, 0x62, 0xbf, 0x3a, 0x90, 0x7b, 0x9e];

/// Legacy layout: discriminator (8) | authority (32) | data_len u32 LE (4) |
/// zlib-compressed JSON (data_len).
pub fn decode_legacy_account(data: &[u8]) -> Result<Vec<u8>, IdlError> {
    let disc = data
        .get(..8)
        .ok_or(IdlError::Malformed("legacy: too short"))?;
    if disc != LEGACY_DISCRIMINATOR {
        return Err(IdlError::Malformed("legacy: wrong discriminator"));
    }
    let len = read_u32(data, 40).ok_or(IdlError::Malformed("legacy: no length"))?;
    let body = slice(data, 44, len).ok_or(IdlError::Malformed("legacy: length past end"))?;
    inflate(ZlibDecoder::new(body))
}

/// Program Metadata account header (96 bytes), then data:
///
/// | offset | field |
/// |---|---|
/// | 0 | discriminator (2 = metadata) |
/// | 1..33 | program |
/// | 33..65 | authority (zeros = canonical) |
/// | 65 | mutable |
/// | 66 | canonical |
/// | 67..83 | seed |
/// | 83 | encoding: 0 none, 1 utf8, 2 base58, 3 base64 |
/// | 84 | compression: 0 none, 1 gzip, 2 zlib |
/// | 85 | format: 0 none, 1 json, 2 yaml, 3 toml |
/// | 86 | data source: 0 direct, 1 url, 2 external |
/// | 87..91 | data length, u32 LE |
/// | 91..96 | padding |
pub fn decode_metadata_account(data: &[u8]) -> Result<Vec<u8>, IdlError> {
    const HEADER: usize = 96;
    if data.len() < HEADER {
        return Err(IdlError::Malformed("metadata: too short"));
    }
    let byte = |i: usize| data.get(i).copied().unwrap_or_default();
    if byte(0) != 2 {
        return Err(IdlError::Malformed("metadata: not a metadata account"));
    }
    let (encoding, compression, format, source) = (byte(83), byte(84), byte(85), byte(86));
    if source != 0 {
        return Err(IdlError::Unsupported(format!(
            "metadata data source {source} (only direct data is supported)"
        )));
    }
    if !matches!(format, 0 | 1) {
        return Err(IdlError::Unsupported(format!(
            "metadata format {format} (not JSON)"
        )));
    }
    let len = read_u32(data, 87).ok_or(IdlError::Malformed("metadata: no length"))?;
    let body = slice(data, HEADER, len).ok_or(IdlError::Malformed("metadata: length past end"))?;

    // Data is compressed, then encoded; undo in reverse order.
    let decoded = match encoding {
        0 | 1 => body.to_vec(),
        2 => bs58::decode(body)
            .into_vec()
            .map_err(|_| IdlError::Malformed("metadata: bad base58"))?,
        3 => base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|_| IdlError::Malformed("metadata: bad base64"))?,
        other => return Err(IdlError::Unsupported(format!("metadata encoding {other}"))),
    };
    match compression {
        0 => {
            if decoded.len() as u64 > MAX_IDL_BYTES {
                return Err(IdlError::Malformed("metadata: too large"));
            }
            Ok(decoded)
        }
        1 => inflate(GzDecoder::new(decoded.as_slice())),
        2 => inflate(ZlibDecoder::new(decoded.as_slice())),
        other => Err(IdlError::Unsupported(format!(
            "metadata compression {other}"
        ))),
    }
}

fn inflate(reader: impl Read) -> Result<Vec<u8>, IdlError> {
    let mut out = Vec::new();
    reader
        .take(MAX_IDL_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|_| IdlError::Malformed("decompression failed"))?;
    if out.len() as u64 > MAX_IDL_BYTES {
        return Err(IdlError::Malformed("decompressed IDL too large"));
    }
    Ok(out)
}

fn read_u32(data: &[u8], at: usize) -> Option<usize> {
    let bytes: [u8; 4] = data.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(bytes)).ok()
}

fn slice(data: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    data.get(start..start.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    fn metadata(encoding: u8, compression: u8, format: u8, source: u8, body: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 96];
        d[0] = 2;
        d[83] = encoding;
        d[84] = compression;
        d[85] = format;
        d[86] = source;
        d[87..91].copy_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        d.extend_from_slice(body);
        d
    }

    #[test]
    fn metadata_plain_and_encoded() {
        let json = br#"{"errors":[]}"#;
        assert_eq!(
            decode_metadata_account(&metadata(1, 0, 1, 0, json)).unwrap(),
            json
        );
        assert_eq!(
            decode_metadata_account(&metadata(1, 2, 1, 0, &zlib(json))).unwrap(),
            json
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode(zlib(json));
        assert_eq!(
            decode_metadata_account(&metadata(3, 2, 1, 0, b64.as_bytes())).unwrap(),
            json
        );
    }

    #[test]
    fn metadata_rejects_unsupported_and_malformed() {
        assert!(matches!(
            decode_metadata_account(&metadata(1, 0, 1, 1, b"https://x")),
            Err(IdlError::Unsupported(_))
        ));
        assert!(matches!(
            decode_metadata_account(&metadata(1, 0, 2, 0, b"a: b")),
            Err(IdlError::Unsupported(_))
        ));
        let mut short = metadata(1, 0, 1, 0, b"{}");
        short.truncate(97);
        assert!(decode_metadata_account(&short).is_err());
        assert!(decode_metadata_account(&[2; 10]).is_err());
    }

    #[test]
    fn legacy_rejects_wrong_discriminator_and_bad_length() {
        let mut d = LEGACY_DISCRIMINATOR.to_vec();
        d.extend([0u8; 32]);
        d.extend(1000u32.to_le_bytes());
        d.extend([1, 2, 3]);
        assert!(decode_legacy_account(&d).is_err());
        d[0] ^= 1;
        assert!(matches!(
            decode_legacy_account(&d),
            Err(IdlError::Malformed("legacy: wrong discriminator"))
        ));
    }

    #[test]
    fn decompression_bomb_is_refused() {
        let big = vec![b' '; usize::try_from(MAX_IDL_BYTES).unwrap() + 10];
        let d = metadata(1, 2, 1, 0, &zlib(&big));
        assert!(decode_metadata_account(&d).is_err());
    }
}
