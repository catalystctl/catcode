// Hashline: stable 4-char line tags for anchored editing.
// read_file emits "HASH│content" per line; edit targets those hashes instead of
// line numbers, so edits don't drift when the file changes between read and edit.
// If a hash isn't found at edit time, the anchor is stale → caller re-reads.

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn fnv1a(s: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 4 base64url chars from the low 24 bits of an FNV-1a hash of the line.
/// ponytail: 24 bits = 16M values; duplicates on large files collide. The
/// stale-anchor re-read loop handles real drift; for true duplicate-line
/// ambiguity, find_hash matches the first occurrence (upgrade to index+hash
/// pairs if a corpus needs it).
pub fn line_hash(line: &str) -> String {
    let h = fnv1a(line.as_bytes());
    let b = [(h & 0xff) as u8, ((h >> 8) & 0xff) as u8, ((h >> 16) & 0xff) as u8];
    let mut s = String::with_capacity(4);
    s.push(B64[(b[0] >> 2) as usize] as char);
    s.push(B64[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
    s.push(B64[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char);
    s.push(B64[(b[2] & 0x3f) as usize] as char);
    s
}

/// Format one line as `HASH│content`.
pub fn tag_line(line: &str) -> String {
    format!("{}│{}", line_hash(line), line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_4_chars_and_stable() {
        let a = line_hash("const x = 1;");
        let b = line_hash("const x = 1;");
        assert_eq!(a, b);
        assert_eq!(a.len(), 4);
        assert_ne!(line_hash("const x = 2;"), a);
        for c in a.chars() {
            assert!(c.is_ascii_alphanumeric() || c == '-' || c == '_');
        }
    }

    #[test]
    fn tag_format() {
        assert_eq!(tag_line("hi"), format!("{}│hi", line_hash("hi")));
    }
}
