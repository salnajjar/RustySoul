pub fn text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(format!(
            "Text must be nonempty, at most {max} bytes, and contain no control characters"
        ));
    }
    Ok(())
}
pub fn username(value: &str) -> Result<(), String> {
    text(value, 30)?;
    if !value.is_ascii() || value.trim() != value {
        return Err("Username must be printable ASCII without surrounding spaces".into());
    }
    Ok(())
}
pub fn room(value: &str) -> Result<(), String> {
    text(value, 24)?;
    if !value.is_ascii() || value.trim() != value || value.contains("  ") {
        return Err(
            "Room names must be ASCII, at most 24 bytes, with no surrounding or repeated spaces"
                .into(),
        );
    }
    Ok(())
}
pub fn remote_path(value: &str) -> Result<(), String> {
    text(value, 4096)?;
    if value
        .split(['/', '\\'])
        .any(|s| s == ".." || s == "." || s.contains(':'))
        || value.starts_with(['/', '\\'])
        || value.ends_with(['/', '\\'])
    {
        return Err("Unsafe remote file path".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_traversal_for_both_path_styles() {
        for s in [
            "../secret",
            "Music\\..\\secret",
            "/etc/passwd",
            "C:\\secret",
            "\\\\server\\file",
            "a\0b",
            "a/",
            "a/./b",
        ] {
            assert!(remote_path(s).is_err(), "{s:?}");
        }
        assert!(remote_path("Music\\Album\\Track.flac").is_ok());
        assert!(remote_path("Music/Album/Track.flac").is_ok());
    }
    #[test]
    fn validates_names_and_text_boundaries() {
        for s in ["", " ", " leading", "trailing ", "na\nme", "é"] {
            assert!(username(s).is_err());
        }
        assert!(username(&"a".repeat(30)).is_ok());
        assert!(username(&"a".repeat(31)).is_err());
        assert!(text("music", 5).is_ok());
        assert!(text("music!", 5).is_err());
    }
}
