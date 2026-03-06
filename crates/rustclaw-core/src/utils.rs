/// Truncate a string to `max` bytes at a valid UTF-8 boundary, appending "..." if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

/// Truncate a string, also collapsing newlines to spaces.
pub fn truncate_oneline(s: &str, max: usize) -> String {
    let replaced = s.replace('\n', " ");
    if replaced.len() <= max {
        replaced
    } else {
        let end = floor_char_boundary(&replaced, max);
        format!("{}...", &replaced[..end])
    }
}

/// Find the largest byte index <= `pos` that is a valid UTF-8 char boundary.
fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Split a string into chunks no larger than `max_len` bytes, breaking at
/// valid UTF-8 boundaries and preferring newline breaks for cleaner output.
pub fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if max_len == 0 {
        return vec![text];
    }
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let mut end = (start + max_len).min(text.len());

        // Find valid UTF-8 boundary
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }

        // Safety: if end == start, advance past the current char to avoid infinite loop
        if end == start {
            end = start + 1;
            while end < text.len() && !text.is_char_boundary(end) {
                end += 1;
            }
        }

        // Try to break at a newline for cleaner splits
        if end < text.len() {
            if let Some(nl) = text[start..end].rfind('\n') {
                end = start + nl + 1;
            }
        }

        chunks.push(&text[start..end]);
        start = end;
    }

    chunks
}
