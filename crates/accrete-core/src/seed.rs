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

/// Parse a list of seed arguments, expanding integer ranges: `"1-10"` (or
/// `"1 - 10"`) is seeds 1 to 10 inclusive. Anything without a dash between
/// two integers is a single seed (`parse_seed`), so text seeds such as
/// `"river-valley"` still work. Note: an argument like `"2-3"` is a range,
/// not a text seed. Half-written ranges (`"1-"`, `"-3"`) and hex ranges
/// (`"0x1-0x3"`) are rejected rather than silently hashed. Duplicates are
/// dropped, keeping the first occurrence; an empty list is an error.
pub fn parse_seed_list(items: &[String]) -> Result<Vec<u64>, String> {
    const MAX_RANGE: u64 = 10_000;
    let mut out: Vec<u64> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
    let hex = |x: &str| {
        let x = x.trim();
        (x.starts_with("0x") || x.starts_with("0X"))
            && x.len() > 2
            && x[2..].bytes().all(|c| c.is_ascii_hexdigit())
    };
    for item in items {
        let t = item.trim();
        let seeds: Vec<u64> = match t.split_once('-') {
            Some((a, b)) if digits(a.trim()) && digits(b.trim()) => {
                let (lo, hi) = match (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
                    (Ok(lo), Ok(hi)) => (lo, hi),
                    _ => return Err(format!("seed range {t} is out of range")),
                };
                if lo > hi {
                    return Err(format!("seed range {t} runs backwards"));
                }
                if hi - lo >= MAX_RANGE {
                    return Err(format!("seed range {t} has more than {MAX_RANGE} seeds"));
                }
                (lo..=hi).collect()
            }
            Some((a, b))
                if (digits(a.trim()) && b.trim().is_empty())
                    || (a.trim().is_empty() && digits(b.trim())) =>
            {
                return Err(format!("seed range {t} is incomplete (use LO-HI)"));
            }
            Some((a, b)) if hex(a) && hex(b) => {
                return Err(format!("seed range {t}: ranges must be decimal"));
            }
            _ => vec![parse_seed(t)],
        };
        for s in seeds {
            if seen.insert(s) {
                out.push(s);
            }
        }
    }
    if out.is_empty() {
        return Err("provide at least one seed".into());
    }
    Ok(out)
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

    #[test]
    fn seed_lists_expand_ranges_and_keep_text_seeds() {
        let v = |items: &[&str]| {
            parse_seed_list(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(v(&["1-3", "7"]).unwrap(), vec![1, 2, 3, 7]);
        assert_eq!(
            v(&["3", "1-4"]).unwrap(),
            vec![3, 1, 2, 4],
            "deduplicated, first wins"
        );
        assert_eq!(
            v(&["river-valley"]).unwrap(),
            vec![parse_seed("river-valley")]
        );
        assert_eq!(v(&["5-5"]).unwrap(), vec![5]);
        assert!(v(&["4-2"]).is_err());
        assert!(v(&["1-99999"]).is_err());
        assert!(v(&[]).is_err());
        assert_eq!(
            v(&[" 1 - 3 "]).unwrap(),
            vec![1, 2, 3],
            "spaces around the dash"
        );
        assert_eq!(v(&["007-009"]).unwrap(), vec![7, 8, 9]);
        assert_eq!(v(&["0-0"]).unwrap(), vec![0]);
        for bad in ["1-", "-3", "0x1-0x3"] {
            assert!(v(&[bad]).is_err(), "{bad}");
        }
        let max = u64::MAX.to_string();
        assert_eq!(v(&[&format!("{max}-{max}")]).unwrap(), vec![u64::MAX]);
        assert!(v(&[&format!("{max}-18446744073709551616")]).is_err());
    }
}
