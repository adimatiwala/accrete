//! Seed parsing.
//!
//! A seed is a `u64`. The user may write it as a decimal integer (`42`), a hex
//! integer (`0x2a`), or any other text, which is hashed with FNV-1a 64
//! (`"river valley"` -> a fixed `u64`). Parsing is pure and platform-independent.

use crate::hash::fnv1a64;

pub fn parse_seed(s: &str) -> u64 {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        if let Ok(v) = u64::from_str_radix(hex, 16) {
            return v;
        }
    }
    if let Ok(v) = t.parse::<u64>() {
        return v;
    }
    fnv1a64(t.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(parse_seed("42"), 42);
        assert_eq!(parse_seed(" 0x2A "), 42);
        assert_eq!(parse_seed("hello"), fnv1a64(b"hello"));
        assert_ne!(parse_seed("hello"), parse_seed("hellp"));
    }
}
