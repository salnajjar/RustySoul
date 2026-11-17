use anyhow::{Result, ensure};
pub fn range(value: Option<&str>, size: u64) -> Result<(u64, u64)> {
    ensure!(size > 0, "Empty file");
    let Some(value) = value else {
        return Ok((0, size - 1));
    };
    let value = value
        .strip_prefix("bytes=")
        .ok_or_else(|| anyhow::anyhow!("Invalid range"))?;
    ensure!(!value.contains(','), "Only one byte range is supported");
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("Invalid range"))?;
    if start.is_empty() {
        let suffix = end.parse::<u64>()?;
        ensure!(suffix > 0, "Empty range");
        return Ok((size.saturating_sub(suffix), size - 1));
    }
    let start = start.parse::<u64>()?;
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>()?.min(size - 1)
    };
    ensure!(start < size && end >= start, "Range is outside the file");
    Ok((start, end))
}
pub fn mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "aiff" | "aif" => "audio/aiff",
        _ => "application/octet-stream",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_ranges_cover_seeking_and_reject_invalid_or_multiple_ranges() {
        assert_eq!(range(None, 100).unwrap(), (0, 99));
        assert_eq!(range(Some("bytes=5-10"), 100).unwrap(), (5, 10));
        assert_eq!(range(Some("bytes=90-"), 100).unwrap(), (90, 99));
        assert_eq!(range(Some("bytes=-20"), 100).unwrap(), (80, 99));
        for r in [
            "bytes=100-",
            "bytes=3-1",
            "bytes=-0",
            "bytes=0-1,4-5",
            "bad",
        ] {
            assert!(range(Some(r), 100).is_err());
        }
    }
}
