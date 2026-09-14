use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Commit;

/// A path in a repository at some ref: a file, a directory, a symlink or a
/// submodule.
///
/// `content` keeps GitFox's per-type body as JSON — a directory's `entries`,
/// a file's base64 `data` — and the accessors below read it. Its shape depends
/// on `kind`, which is what a single typed field could not express.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Content {
    /// `file`, `dir`, `symlink` or `submodule`.
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default)]
    pub latest_commit: Option<Commit>,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentEntry {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
}

impl Content {
    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }

    /// A directory's entries; empty for anything else.
    pub fn entries(&self) -> Vec<ContentEntry> {
        self.content
            .get("entries")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// The file's size in bytes, when GitFox reported it.
    pub fn size(&self) -> Option<i64> {
        self.content.get("size").and_then(Value::as_i64)
    }

    /// The file's bytes, decoded from the transfer encoding.
    ///
    /// `None` when this is not a file or the encoding is not one GitFox is
    /// known to use.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        let data = self.content.get("data")?.as_str()?;
        match self.content.get("encoding").and_then(Value::as_str) {
            Some("base64") | None => decode_base64(data),
            Some("utf8" | "utf-8" | "text") => Some(data.as_bytes().to_vec()),
            Some(_) => None,
        }
    }
}

/// Standard base64, tolerant of line breaks. Kept here rather than pulling in
/// a crate for the one place GitFox sends it.
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    }

    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            acc |= value(*byte)? << (18 - 6 * i as u32);
        }
        let bytes = acc.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

/// Standard base64 with padding.
pub fn encode_base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = match chunk.len() {
            3 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | chunk[2] as u32,
            2 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8,
            _ => (chunk[0] as u32) << 16,
        };
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base64_round_trips_every_padding_length() {
        for text in ["", "a", "ab", "abc", "abcd", "# AgentNexus\n\n> 中文"] {
            let encoded = encode_base64(text.as_bytes());
            assert_eq!(decode_base64(&encoded).unwrap(), text.as_bytes(), "{text}");
        }
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
        // GitFox wraps long payloads; the line breaks are not data.
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");
        assert!(decode_base64("not*base64").is_none());
    }

    #[test]
    fn a_file_decodes_and_a_directory_lists() {
        let file: Content = serde_json::from_value(json!({
            "type": "file", "name": "README.md", "path": "README.md", "sha": "b72a",
            "content": { "data": "IyBBZ2VudE5leHVz", "encoding": "base64", "size": 12 }
        }))
        .unwrap();
        assert_eq!(file.bytes().unwrap(), b"# AgentNexus");
        assert_eq!(file.size(), Some(12));
        assert!(file.entries().is_empty());

        let dir: Content = serde_json::from_value(json!({
            "type": "dir", "path": "",
            "content": { "entries": [
                { "type": "file", "name": "Cargo.toml", "path": "Cargo.toml", "sha": "1" },
                { "type": "dir", "name": "src", "path": "src", "sha": "2" }
            ] }
        }))
        .unwrap();
        assert!(dir.is_dir());
        assert_eq!(dir.entries().len(), 2);
        assert_eq!(dir.entries()[1].kind, "dir");
        assert!(dir.bytes().is_none());
    }
}
