//! Numeric list indices (`1`, `#2`) and inclusive ranges (`1-15`, `1..15`).

use std::fmt;

pub const DEFAULT_PARALLEL_CONCURRENCY: usize = 15;
pub const MAX_LIST_INDEX_RANGE_WARN_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListIndexRangeError {
    pub message: String,
}

impl fmt::Display for ListIndexRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ListIndexRangeError {}

/// Parse a 1-based list index from `1`, `#2`, etc. Returns `None` if not a list-index token.
pub fn parse_list_index(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !digits.chars().all(|c| c.is_ascii_digit()) || digits.is_empty() {
        return None;
    }
    let index: usize = digits.parse().ok()?;
    (index >= 1).then_some(index)
}

/// Parse a single index or inclusive range: `1`, `#2`, `1-15`, `1..15`, `1:15`, `#1-#15`.
pub fn parse_list_index_range(value: &str) -> Result<Option<Vec<usize>>, ListIndexRangeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if let Some(single) = parse_list_index(trimmed) {
        return Ok(Some(vec![single]));
    }

    let re = regex_lite_range(trimmed);
    let Some((start, end)) = re else {
        return Ok(None);
    };

    if start < 1 {
        return Err(ListIndexRangeError {
            message: format!("Range start must be >= 1. Got {start}."),
        });
    }
    if start > end {
        return Err(ListIndexRangeError {
            message: format!("Invalid range: start ({start}) must be <= end ({end})."),
        });
    }

    Ok(Some((start..=end).collect()))
}

fn regex_lite_range(trimmed: &str) -> Option<(usize, usize)> {
    // #?digits ( - | .. | : ) #?digits
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'#') {
        i = 1;
    }
    let start_begin = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start_begin {
        return None;
    }
    let start: usize = trimmed[start_begin..i].parse().ok()?;

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let sep = if bytes.get(i..).is_some_and(|s| s.starts_with(b"..")) {
        i += 2;
        true
    } else if bytes.get(i) == Some(&b'-') || bytes.get(i) == Some(&b':') {
        i += 1;
        true
    } else {
        false
    };
    if !sep {
        return None;
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let end_begin = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == end_begin || i != bytes.len() {
        return None;
    }
    let end: usize = trimmed[end_begin..i].parse().ok()?;
    Some((start, end))
}

pub fn item_at_list_index<T>(items: &[T], index: usize) -> Option<&T> {
    if index < 1 || index > items.len() {
        None
    } else {
        items.get(index - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_hash() {
        assert_eq!(parse_list_index("1"), Some(1));
        assert_eq!(parse_list_index("#2"), Some(2));
        assert_eq!(parse_list_index("conn"), None);
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_list_index_range("1-3").unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(parse_list_index_range("1..2").unwrap(), Some(vec![1, 2]));
        assert_eq!(parse_list_index_range("#1:#2").unwrap(), Some(vec![1, 2]));
    }
}
