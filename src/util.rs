pub(crate) fn truncate_with_ellipsis(s: &str, max_len: usize, ellipsis: &str) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let cut = max_len.saturating_sub(ellipsis.len());
    let boundary = s
        .char_indices()
        .take_while(|&(i, _)| i <= cut)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}{}", &s[..boundary], ellipsis)
}

/// Split YAML-style frontmatter (`---\n...\n---`) from the body.
/// Returns (frontmatter_text, body_text) where body_text is the raw remainder
/// after the closing delimiter; callers trim/parse as needed.
/// Returns None if the document has no frontmatter.
pub fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))?;
    let close = rest.find("\n---")?;
    let meta = &rest[..close];
    let body = &rest[close + 4..];
    Some((meta, body))
}
