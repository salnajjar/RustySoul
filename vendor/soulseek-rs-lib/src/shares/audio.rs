//! The audio attributes a share advertises for a file, read from the file's
//! own headers: Soulseek codes 0 bitrate (kbps), 1 duration (seconds), 2 VBR,
//! 4 sample rate (Hz) and 5 bit depth. Searchers filter and sort on these, so
//! a file without them ranks last in every other client.

use std::path::Path;

use std::io::{Read, Seek, SeekFrom};

const BITRATE: u32 = 0;
const DURATION: u32 = 1;
const VBR: u32 = 2;
const SAMPLE_RATE: u32 = 4;
const BIT_DEPTH: u32 = 5;

/// How much of a file's head one read takes. A frame header with its Xing
/// block needs under a hundred bytes; the rest is margin so a typical ID3v2
/// tag of a few kilobytes fits in the first read too.
const HEAD: usize = 8 * 1024;

/// The attributes for `path`, or none when the format is not one we read or
/// the file does not parse. Header/trailer reads are bounded; audio is not decoded.
#[must_use]
pub fn probe(path: &Path) -> Vec<(u32, u32)> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp3") => mp3(path),
        Some("flac") => flac(path),
        Some(
            "wav" | "wave" | "aif" | "aiff" | "ape" | "wv" | "m4a" | "m4b" | "mp4" | "aac" | "ogg"
            | "oga" | "opus" | "spx" | "mpc",
        ) => other_audio(path, ext.as_deref().unwrap()),
        _ => None,
    }
    .unwrap_or_default()
}

struct BoundedReader {
    file: std::fs::File,
    left: usize,
}
impl Read for BoundedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            return Err(std::io::Error::other("Audio property read budget exceeded"));
        }
        let limit = buf.len().min(self.left);
        let read = self.file.read(&mut buf[..limit])?;
        self.left -= read;
        Ok(read)
    }
}
impl Seek for BoundedReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}
fn other_audio(path: &Path, extension: &str) -> Option<Vec<(u32, u32)>> {
    use lofty::{
        config::{GlobalOptions, ParseOptions, apply_global_options},
        file::AudioFile,
        mp4::{Mp4Codec, Mp4File},
        probe::Probe,
    };
    // Options are thread-local. Never decode artwork or retain textual tags.
    apply_global_options(
        GlobalOptions::new()
            .allocation_limit(1024 * 1024)
            .use_custom_resolvers(false)
            .preserve_format_specific_items(false),
    );
    let reader = BoundedReader {
        file: std::fs::File::open(path).ok()?,
        left: 8 * 1024 * 1024,
    };
    let options = ParseOptions::new()
        .read_tags(false)
        .read_cover_art(false)
        .read_properties(true);
    let (properties, mp4_lossless): (lofty::properties::FileProperties, bool) =
        if matches!(extension, "m4a" | "m4b" | "mp4") {
            let mut reader = std::io::BufReader::new(reader);
            let audio = Mp4File::read_from(&mut reader, options).ok()?;
            let lossless = matches!(
                audio.properties().codec(),
                Some(Mp4Codec::ALAC | Mp4Codec::FLAC)
            );
            (audio.properties().clone().into(), lossless)
        } else {
            let audio = Probe::new(std::io::BufReader::new(reader))
                .guess_file_type()
                .ok()?
                .options(options)
                .read()
                .ok()?;
            (audio.properties().clone(), false)
        };
    let mut attributes = vec![];
    if !properties.duration().is_zero() {
        attributes.push((
            DURATION,
            properties.duration().as_secs().min(u64::from(u32::MAX)) as u32,
        ));
    }
    let lossless =
        matches!(extension, "wav" | "wave" | "aiff" | "aif" | "ape" | "wv") || mp4_lossless;
    if !lossless || extension == "wv" {
        if let Some(rate) = properties.audio_bitrate().filter(|r| *r > 0) {
            attributes.push((BITRATE, rate));
        }
    }
    if let Some(rate) = properties.sample_rate().filter(|r| *r > 0) {
        attributes.push((SAMPLE_RATE, rate));
    }
    if lossless {
        if let Some(depth) = properties.bit_depth().filter(|d| *d > 0) {
            attributes.push((BIT_DEPTH, u32::from(depth)));
        }
    }
    attributes.sort_unstable_by_key(|a| a.0);
    Some(attributes)
}

fn flac(path: &Path) -> Option<Vec<(u32, u32)>> {
    let mut head = [0u8; 42];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[0..4] != b"fLaC" || head[4] & 0x7F != 0 {
        return None;
    }
    let packed = u64::from_be_bytes(head[18..26].try_into().ok()?);
    let sample_rate = (packed >> 44) as u32;
    let bits = ((packed >> 36) & 0x1F) as u32 + 1;
    let samples = packed & 0xF_FFFF_FFFF;
    if sample_rate == 0 {
        return None;
    }
    let mut attributes = Vec::new();
    if samples > 0 {
        let seconds = samples / u64::from(sample_rate);
        attributes.push((DURATION, u32::try_from(seconds).unwrap_or(u32::MAX)));
    }
    attributes.push((SAMPLE_RATE, sample_rate));
    attributes.push((BIT_DEPTH, bits));
    Some(attributes)
}

/// Layer III bitrates in kbps by the header's four-bit index, for MPEG-1 and
/// for MPEG-2 and 2.5, which halve the table.
const MPEG1_BITRATES: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_BITRATES: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
/// Sample rates by the header's two-bit index, one row per version: MPEG-2.5,
/// MPEG-2, MPEG-1.
const SAMPLE_RATES: [[u32; 3]; 3] = [
    [11_025, 12_000, 8_000],
    [22_050, 24_000, 16_000],
    [44_100, 48_000, 32_000],
];

/// One Layer III frame header, decoded.
struct Frame {
    bitrate_kbps: u32,
    sample_rate: u32,
    samples: u32,
    /// Offset of the Xing/Info block relative to the header.
    xing_at: usize,
}

fn frame(h: &[u8]) -> Option<Frame> {
    if h.len() < 4 || h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (h[1] >> 3) & 3; // 0 = 2.5, 2 = 2, 3 = 1
    let layer = (h[1] >> 1) & 3; // 1 = Layer III
    let bitrate_index = usize::from(h[2] >> 4);
    let rate_index = usize::from((h[2] >> 2) & 3);
    if version == 1 || layer != 1 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let mpeg1 = version == 3;
    let mono = (h[3] >> 6) & 3 == 3;
    // Protection bit clear means a two-byte CRC precedes the side info.
    let crc = usize::from(h[1] & 1 == 0) * 2;
    let rates = SAMPLE_RATES[if version == 0 {
        0
    } else if mpeg1 {
        2
    } else {
        1
    }];
    Some(Frame {
        bitrate_kbps: if mpeg1 {
            MPEG1_BITRATES
        } else {
            MPEG2_BITRATES
        }[bitrate_index],
        sample_rate: rates[rate_index],
        samples: if mpeg1 { 1152 } else { 576 },
        // Side info is 32 bytes for MPEG-1 stereo, 17 for MPEG-1 mono and
        // MPEG-2 stereo, 9 for MPEG-2 mono.
        xing_at: 4
            + crc
            + match (mpeg1, mono) {
                (true, false) => 32,
                (true, true) | (false, false) => 17,
                (false, true) => 9,
            },
    })
}

fn mp3(path: &Path) -> Option<Vec<(u32, u32)>> {
    let mut file = std::fs::File::open(path).ok()?;
    let total = file.metadata().ok()?.len();
    let mut head = [0u8; HEAD];
    let mut len = file.read(&mut head).ok()?;
    // An ID3v2 tag in front: its size is four syncsafe bytes, plus a footer.
    // `base` is where `head` sits in the file once a big tag forces a second
    // read, so the audio length below counts from the first frame.
    let mut base = 0usize;
    let mut audio_at = 0usize;
    if len >= 10 && &head[0..3] == b"ID3" {
        let size = head[6..10]
            .iter()
            .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7F));
        audio_at = 10 + size + if head[5] & 0x10 != 0 { 10 } else { 0 };
        if audio_at + 4 > len {
            file.seek(SeekFrom::Start(audio_at as u64)).ok()?;
            len = file.read(&mut head).ok()?;
            base = audio_at;
            audio_at = 0;
        }
    }
    let head = &head[..len];
    let start = (audio_at..len.saturating_sub(4)).find(|&i| frame(&head[i..]).is_some())?;
    let frame = frame(&head[start..])?;
    let audio_bytes = total.saturating_sub((base + start) as u64);
    let at_frame_rate = |bytes: u64| {
        (bytes.saturating_mul(8) / (u64::from(frame.bitrate_kbps) * 1000)).min(u64::from(u32::MAX))
            as u32
    };

    // VBRI uses a fixed offset 32 bytes after the MPEG header.
    let vbri = start + 36;
    if head.get(vbri..vbri + 4) == Some(b"VBRI") {
        let bytes = u32::from_be_bytes(head.get(vbri + 10..vbri + 14)?.try_into().ok()?);
        let frames = u32::from_be_bytes(head.get(vbri + 14..vbri + 18)?.try_into().ok()?);
        let seconds = f64::from(frames) * f64::from(frame.samples) / f64::from(frame.sample_rate);
        if seconds > 0.0 {
            return Some(vec![
                (BITRATE, (f64::from(bytes) * 8.0 / seconds / 1000.0) as u32),
                (DURATION, seconds as u32),
                (VBR, 1),
                (SAMPLE_RATE, frame.sample_rate),
            ]);
        }
    }

    let xing = start + frame.xing_at;
    let tag = head.get(xing..xing + 4);
    let vbr = u32::from(tag == Some(b"Xing"));
    if tag != Some(b"Xing") && tag != Some(b"Info") {
        return Some(vec![
            (BITRATE, frame.bitrate_kbps),
            (DURATION, at_frame_rate(audio_bytes)),
            (VBR, 0),
            (SAMPLE_RATE, frame.sample_rate),
        ]);
    }
    let flags = u32::from_be_bytes(head.get(xing + 4..xing + 8)?.try_into().ok()?);
    let mut at = xing + 8;
    let mut frames = None;
    if flags & 1 != 0 {
        frames = Some(u32::from_be_bytes(head.get(at..at + 4)?.try_into().ok()?));
        at += 4;
    }
    let bytes = if flags & 2 != 0 {
        u64::from(u32::from_be_bytes(head.get(at..at + 4)?.try_into().ok()?))
    } else {
        audio_bytes
    };
    // Without a frame count the block adds nothing the first frame did not say.
    let Some(frames) = frames else {
        return Some(vec![
            (BITRATE, frame.bitrate_kbps),
            (DURATION, at_frame_rate(bytes)),
            (VBR, vbr),
            (SAMPLE_RATE, frame.sample_rate),
        ]);
    };
    let seconds = f64::from(frames) * f64::from(frame.samples) / f64::from(frame.sample_rate);
    // Info identifies CBR. Its byte count includes metadata/padding frames and
    // must not replace the MPEG frame's nominal rate (e.g. 192 with 195 kbps).
    let kbps = if vbr == 0 {
        frame.bitrate_kbps
    } else if seconds > 0.0 {
        (bytes as f64 * 8.0 / seconds / 1000.0) as u32
    } else {
        frame.bitrate_kbps
    };
    Some(vec![
        (BITRATE, kbps),
        (DURATION, seconds as u32),
        (VBR, vbr),
        (SAMPLE_RATE, frame.sample_rate),
    ])
}

#[cfg(test)]
pub(in crate::shares) mod tests {
    use super::*;
    #[test]
    fn real_encoded_audio_reports_compatible_properties_without_decoding_payloads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
        for name in [
            "tone.wav",
            "tone.aiff",
            "tone.flac",
            "tone.mp3",
            "tone.ogg",
            "tone.opus",
            "tone-aac.m4a",
            "tone-alac.m4a",
            "tone.wv",
            "tone.aac",
        ] {
            let attributes = probe(&root.join(name));
            assert_eq!(
                attributes.iter().find(|a| a.0 == DURATION).map(|a| a.1),
                Some(1),
                "{name}: {attributes:?}"
            );
            assert!(
                attributes.contains(&(SAMPLE_RATE, 48000)),
                "{name}: {attributes:?}"
            );
            if matches!(
                name,
                "tone.wav" | "tone.aiff" | "tone.flac" | "tone-alac.m4a" | "tone.wv"
            ) {
                assert!(
                    attributes.contains(&(SAMPLE_RATE, 48000)),
                    "{name}: {attributes:?}"
                );
                assert!(
                    attributes.iter().any(|a| a.0 == BIT_DEPTH && a.1 >= 16),
                    "{name}: {attributes:?}"
                );
            } else {
                assert!(
                    attributes.iter().any(|a| a.0 == BITRATE && a.1 > 0),
                    "{name}: {attributes:?}"
                );
                assert!(
                    !attributes.iter().any(|a| a.0 == BIT_DEPTH),
                    "Lossy output must not invent PCM bit depth: {name}"
                );
            }
        }
        assert!(probe(&root.join("tone.wav")).contains(&(BIT_DEPTH, 24)));
        assert!(probe(&root.join("tone.mp3")).contains(&(VBR, 0)));
        assert!(probe(&root.join("tone.mp3")).contains(&(BITRATE, 192)));
    }

    /// One MPEG-1 Layer III frame header: 128 kbps, 44.1 kHz, stereo.
    const CBR_128: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
    const CBR_128_FRAME_LEN: usize = 417;

    pub(in crate::shares) fn cbr_mp3(frames: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..frames {
            out.extend_from_slice(&CBR_128);
            out.resize(out.len() + CBR_128_FRAME_LEN - 4, 0);
        }
        out
    }

    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("soulseek-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_constant_bitrate_mp3_reports_bitrate_duration_and_no_vbr() {
        // 100 frames of 1152 samples at 44.1 kHz is 2.61 s.
        let path = write("cbr.mp3", &cbr_mp3(100));
        assert_eq!(probe(&path), [(0, 128), (1, 2), (2, 0), (4, 44_100)]);
    }

    /// An ID3v2 tag of `size` bytes (syncsafe in the header) with no footer.
    fn id3v2(size: usize) -> Vec<u8> {
        let mut tag = b"ID3\x04\x00\x00".to_vec();
        tag.push(((size >> 21) & 0x7F) as u8);
        tag.push(((size >> 14) & 0x7F) as u8);
        tag.push(((size >> 7) & 0x7F) as u8);
        tag.push((size & 0x7F) as u8);
        tag.resize(10 + size, 0);
        tag
    }

    #[test]
    fn an_id3v2_tag_in_front_of_the_frames_is_skipped() {
        // 50 frames is 1.31 s; counting the 8000-byte tag as audio, or the
        // tag twice, would put the duration at 0.
        let mut bytes = id3v2(8000);
        bytes.extend_from_slice(&cbr_mp3(50));
        let path = write("tagged.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 128), (1, 1), (2, 0), (4, 44_100)]);
    }

    #[test]
    fn a_tag_larger_than_the_first_read_is_skipped_too() {
        // 20 KiB of tag pushes the frames past the first read; 100 frames is
        // 2.61 s, and counting the tag as audio would say 3.
        let mut bytes = id3v2(20_480);
        bytes.extend_from_slice(&cbr_mp3(100));
        let path = write("big-tag.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 128), (1, 2), (2, 0), (4, 44_100)]);
    }

    #[test]
    fn a_crc_protected_frame_still_finds_its_xing_header() {
        // Protection bit clear: a two-byte CRC sits before the side info.
        let mut bytes = cbr_mp3(1);
        bytes[1] = 0xFA;
        let xing_at = 4 + 2 + 32;
        bytes[xing_at..xing_at + 4].copy_from_slice(b"Xing");
        bytes[xing_at + 4..xing_at + 8].copy_from_slice(&3u32.to_be_bytes());
        bytes[xing_at + 8..xing_at + 12].copy_from_slice(&200u32.to_be_bytes());
        bytes[xing_at + 12..xing_at + 16].copy_from_slice(&104_250u32.to_be_bytes());
        let path = write("crc.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 159), (1, 5), (2, 1), (4, 44_100)]);
    }

    #[test]
    fn a_xing_header_without_a_frame_count_falls_back_to_the_frame_header() {
        let mut bytes = cbr_mp3(100);
        let xing_at = 4 + 32;
        bytes[xing_at..xing_at + 4].copy_from_slice(b"Xing");
        bytes[xing_at + 4..xing_at + 8].copy_from_slice(&2u32.to_be_bytes());
        bytes[xing_at + 8..xing_at + 12].copy_from_slice(&41_700u32.to_be_bytes());
        let path = write("no-frames.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 128), (1, 2), (2, 1), (4, 44_100)]);
    }

    #[test]
    fn a_variable_bitrate_mp3_uses_its_xing_header() {
        // A Xing header in the first frame's data: 200 frames, 104_250 bytes
        // in the stream, so 5.22 s at an average of 160 kbps.
        let mut bytes = cbr_mp3(1);
        let xing_at = 4 + 32; // MPEG-1 stereo side info
        bytes[xing_at..xing_at + 4].copy_from_slice(b"Xing");
        bytes[xing_at + 4..xing_at + 8].copy_from_slice(&3u32.to_be_bytes());
        bytes[xing_at + 8..xing_at + 12].copy_from_slice(&200u32.to_be_bytes());
        bytes[xing_at + 12..xing_at + 16].copy_from_slice(&104_250u32.to_be_bytes());
        let path = write("vbr.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 159), (1, 5), (2, 1), (4, 44_100)]);
    }

    #[test]
    fn a_flac_reports_duration_sample_rate_and_bit_depth() {
        // STREAMINFO: 44.1 kHz, 2 channels, 16 bits, 441_000 samples (10 s).
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]);
        bytes.extend_from_slice(&[0x10, 0x00, 0x10, 0x00]); // block sizes
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // frame sizes
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 0x0006_BAA8;
        bytes.extend_from_slice(&packed.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 16]); // md5
        let path = write("lossless.flac", &bytes);
        assert_eq!(probe(&path), [(1, 10), (4, 44_100), (5, 16)]);
    }

    #[test]
    fn vbri_reports_the_average_and_malformed_new_formats_do_not_invent_attributes() {
        let mut bytes = cbr_mp3(1);
        bytes[36..40].copy_from_slice(b"VBRI");
        bytes[46..50].copy_from_slice(&104250_u32.to_be_bytes());
        bytes[50..54].copy_from_slice(&200_u32.to_be_bytes());
        assert_eq!(
            probe(&write("vbri.mp3", &bytes)),
            [(0, 159), (1, 5), (2, 1), (4, 44_100)]
        );
        for extension in ["wav", "aiff", "m4a", "ogg", "opus", "wv", "ape", "aac"] {
            assert!(
                probe(&write(
                    &format!("malformed.{extension}"),
                    b"not a valid audio file"
                ))
                .is_empty()
            );
        }
    }

    #[test]
    fn info_headers_keep_the_nominal_cbr_rate_while_using_the_frame_count_for_duration() {
        let mut bytes = cbr_mp3(1);
        let info = 36;
        bytes[info..info + 4].copy_from_slice(b"Info");
        bytes[info + 4..info + 8].copy_from_slice(&3_u32.to_be_bytes());
        bytes[info + 8..info + 12].copy_from_slice(&200_u32.to_be_bytes());
        bytes[info + 12..info + 16].copy_from_slice(&104250_u32.to_be_bytes());
        assert_eq!(
            probe(&write("cbr-info.mp3", &bytes)),
            [(0, 128), (1, 5), (2, 0), (4, 44100)]
        );
    }
    #[test]
    fn anything_else_or_a_truncated_file_has_no_attributes() {
        assert!(probe(&write("notes.txt", b"hello")).is_empty());
        assert!(probe(&write("short.mp3", b"ID3")).is_empty());
        assert!(probe(&write("short.flac", b"fLaC\x80\x00")).is_empty());
        assert!(probe(Path::new("/nonexistent/x.mp3")).is_empty());
    }
}
