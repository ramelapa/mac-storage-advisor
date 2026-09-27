use std::fmt;

/// Parse a logical-size threshold.
///
/// Plain integers are bytes. Units: `K`/`KB` = 1000, `KiB` = 1024, and the
/// same pattern for `M`/`G`/`T`. A single space between the number and the
/// unit is allowed.
pub fn parse_byte_size(raw: &str) -> Result<u64, SizeParseError> {
    let compact: String = raw.split_whitespace().collect();
    if compact.is_empty() {
        return Err(SizeParseError("size is empty".into()));
    }
    let split = compact.find(|ch: char| !ch.is_ascii_digit());
    let (number_text, unit) = match split {
        Some(0) => {
            return Err(SizeParseError(format!(
                "size `{raw}` must start with a non-negative integer"
            )));
        }
        Some(index) => (&compact[..index], &compact[index..]),
        None => (compact.as_str(), ""),
    };
    let number: u64 = number_text
        .parse()
        .map_err(|_| SizeParseError(format!("size `{raw}` has an invalid number")))?;
    let multiplier: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "kib" => 1024,
        "m" | "mb" => 1_000_000,
        "mib" => 1024 * 1024,
        "g" | "gb" => 1_000_000_000,
        "gib" => 1024 * 1024 * 1024,
        "t" | "tb" => 1_000_000_000_000,
        "tib" => 1024u64.saturating_pow(4),
        _ => {
            return Err(SizeParseError(format!(
                "size `{raw}` has an unknown unit (expected B, K, KiB, M, MiB, G, GiB, T, TiB)"
            )));
        }
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| SizeParseError(format!("size `{raw}` overflows the supported range")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeParseError(String);

impl fmt::Display for SizeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SizeParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bytes_and_units() {
        assert_eq!(parse_byte_size("0").unwrap(), 0);
        assert_eq!(parse_byte_size("10").unwrap(), 10);
        assert_eq!(parse_byte_size(" 5 ").unwrap(), 5);
        assert_eq!(parse_byte_size("1K").unwrap(), 1_000);
        assert_eq!(parse_byte_size("1KB").unwrap(), 1_000);
        assert_eq!(parse_byte_size("1 KiB").unwrap(), 1024);
        assert_eq!(parse_byte_size("2MiB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_byte_size("1G").unwrap(), 1_000_000_000);
        assert_eq!(parse_byte_size("1gib").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_junk() {
        assert!(parse_byte_size("").is_err());
        assert!(parse_byte_size("-1").is_err());
        assert!(parse_byte_size("12XB").is_err());
        assert!(parse_byte_size("nope").is_err());
    }
}
