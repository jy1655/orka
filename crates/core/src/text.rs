pub fn normalize_text(raw: &str, max_chars: usize) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

pub fn normalize_text_with_fallback(raw: &str, max_chars: usize, fallback: &str) -> String {
    normalize_text(raw, max_chars).unwrap_or_else(|| fallback.to_string())
}

pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || max_chars == 0 {
        return vec![];
    }
    if trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = trimmed;
    while !remaining.is_empty() {
        if remaining.chars().count() <= max_chars {
            chunks.push(remaining.to_string());
            break;
        }
        // Find the byte offset corresponding to `max_chars` characters.
        let byte_limit = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(remaining.len());
        let boundary = remaining[..byte_limit]
            .rfind('\n')
            .unwrap_or_else(|| remaining[..byte_limit].rfind(' ').unwrap_or(byte_limit));
        let boundary = if boundary == 0 { byte_limit } else { boundary };
        chunks.push(remaining[..boundary].trim_end().to_string());
        remaining = remaining[boundary..].trim_start();
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::{chunk_text, normalize_text, normalize_text_with_fallback};

    #[test]
    fn normalize_text_rejects_empty_input() {
        assert_eq!(normalize_text("   ", 10), None);
    }

    #[test]
    fn normalize_text_trims_and_truncates() {
        assert_eq!(normalize_text("  hello  ", 10), Some("hello".to_string()));
        assert_eq!(normalize_text("abcdef", 3), Some("abc".to_string()));
    }

    #[test]
    fn normalize_text_with_fallback_uses_fallback_when_empty() {
        assert_eq!(
            normalize_text_with_fallback("  ", 10, "(empty response)"),
            "(empty response)".to_string()
        );
    }

    #[test]
    fn chunk_text_returns_empty_for_blank() {
        assert!(chunk_text("   ", 100).is_empty());
    }

    #[test]
    fn chunk_text_returns_single_chunk_when_short() {
        assert_eq!(chunk_text("hello", 100), vec!["hello"]);
    }

    #[test]
    fn chunk_text_splits_at_newline_boundary() {
        let text = "line1\nline2\nline3";
        let chunks = chunk_text(text, 12);
        assert_eq!(chunks, vec!["line1\nline2", "line3"]);
    }

    #[test]
    fn chunk_text_splits_long_text() {
        let text = "a".repeat(30);
        let chunks = chunk_text(&text, 10);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.len() <= 10));
    }

    #[test]
    fn chunk_text_handles_multibyte_chars() {
        // Each Korean character is 3 bytes in UTF-8.
        let text = "가나다라마바사아자차카타파하";
        // 14 chars, each 3 bytes = 42 bytes total.
        let chunks = chunk_text(text, 5);
        // Should split by character count, not byte count, and not panic.
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 5);
        }
    }

    #[test]
    fn chunk_text_returns_empty_for_zero_max() {
        assert!(chunk_text("hello", 0).is_empty());
    }
}
