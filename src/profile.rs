use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageEncoder};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MAX_INPUT_PICTURE: usize = 10 * 1024 * 1024;
pub const MAX_PICTURE: usize = 3 * 1024 * 1024;

/// Decode untrusted input within fixed limits, preserve pixels/alpha, and write
/// a fresh PNG so no source metadata reaches peers. Only local uploads use the
/// expensive optimizer; remote avatars must not stall the control loop.
pub fn normalize_picture(bytes: &[u8], optimize: bool) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() <= MAX_INPUT_PICTURE,
        "Profile picture exceeds 10 MB"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Jpeg | image::ImageFormat::Png)
        ),
        "Choose a JPEG or PNG image"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= 24_000_000,
        "Choose an image with at most 24 million pixels"
    );
    let orientation = decoder.orientation()?;
    let mut picture = image::DynamicImage::from_decoder(decoder)?;
    if picture.width() > 512 || picture.height() > 512 {
        picture = picture.thumbnail(512, 512);
    }
    // The square bounding box is orientation invariant. Rotate after shrinking
    // so a phone photo never needs a second full-size pixel buffer.
    picture.apply_orientation(orientation);
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut encoded,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        picture.as_bytes(),
        picture.width(),
        picture.height(),
        picture.color().into(),
    )?;
    if optimize {
        let mut options = oxipng::Options::max_compression();
        options.deflater = oxipng::Deflater::Zopfli(Default::default());
        options.strip = oxipng::StripChunks::All;
        options.optimize_alpha = false;
        options.scale_16 = false;
        options.max_decompressed_size = Some(MAX_PICTURE);
        encoded = oxipng::optimize_from_memory(&encoded, &options)?;
    }
    ensure!(
        encoded.len() <= MAX_PICTURE,
        "Converted picture is too large"
    );
    Ok(encoded)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub description: String,
    pub picture: Option<String>,
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
    pub away_on_startup: bool,
    pub auto_away_minutes: u32,
    pub away_message: String,
    pub report_upload_speed: bool,
    pub reported_upload_speed_kib: Option<u32>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            description: String::new(),
            picture: None,
            likes: vec![],
            dislikes: vec![],
            away_on_startup: false,
            auto_away_minutes: 0,
            away_message: String::new(),
            report_upload_speed: true,
            reported_upload_speed_kib: None,
        }
    }
}
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub description: String,
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
    pub away_on_startup: bool,
    pub auto_away_minutes: u32,
    pub away_message: String,
    pub has_picture: bool,
    pub revision: u64,
    pub report_upload_speed: bool,
    pub reported_upload_speed_kib: Option<u32>,
    pub measured_upload_speed: u32,
    pub upload_speed_measured: bool,
    pub upload_speed_recipients: usize,
    pub upload_speed_selected_recipients: usize,
}
impl Profile {
    pub fn view(&self, revision: u64) -> View {
        View {
            description: self.description.clone(),
            likes: self.likes.clone(),
            dislikes: self.dislikes.clone(),
            away_on_startup: self.away_on_startup,
            auto_away_minutes: self.auto_away_minutes,
            away_message: self.away_message.clone(),
            has_picture: self.picture.is_some(),
            revision,
            report_upload_speed: self.report_upload_speed,
            reported_upload_speed_kib: self.reported_upload_speed_kib,
            measured_upload_speed: 0,
            upload_speed_measured: false,
            upload_speed_recipients: 0,
            upload_speed_selected_recipients: 0,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.reported_upload_speed_kib
                .is_none_or(|v| (1..=1_000_000).contains(&v)),
            "Choose a reported speed from 1 to 1000000 KiB/s, or leave it blank for automatic"
        );
        ensure!(
            self.auto_away_minutes <= 1440,
            "Choose an idle timeout from 1 to 1440 minutes, or 0 to disable"
        );
        ensure!(
            self.description.len() <= 8192 && self.away_message.len() <= 1024,
            "Use at most 8192 bytes for your description and 1024 for an away reply"
        );
        ensure!(
            !self
                .description
                .chars()
                .chain(self.away_message.chars())
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
            "Profile text contains unsupported control characters"
        );
        ensure!(
            self.likes.len() <= 100 && self.dislikes.len() <= 100,
            "Use at most 100 likes and 100 dislikes"
        );
        for item in self.likes.iter().chain(&self.dislikes) {
            crate::validation::text(item, 80).map_err(anyhow::Error::msg)?;
        }
        ensure!(
            self.picture
                .as_ref()
                .is_none_or(|p| p.len() <= MAX_INPUT_PICTURE.div_ceil(3) * 4),
            "Profile picture must be at most 10 MB before encoding"
        );
        Ok(())
    }
    pub fn normalize(mut self) -> Result<Self> {
        self.validate()?;
        for items in [&mut self.likes, &mut self.dislikes] {
            *items = items.iter().map(|s| s.trim().to_lowercase()).collect();
            items.sort();
            items.dedup();
        }
        ensure!(
            !self.likes.iter().any(|s| self.dislikes.contains(s)),
            "A topic cannot be both liked and disliked"
        );
        if let Some(p) = &self.picture {
            let bytes = STANDARD.decode(p)?;
            self.picture = Some(STANDARD.encode(normalize_picture(&bytes, true)?));
        }
        Ok(self)
    }
    pub fn normalize_against(mut self, previous: &Self) -> Result<Self> {
        if self.picture == previous.picture {
            let picture = self.picture.take();
            let mut normalized = self.normalize()?;
            normalized.picture = picture;
            Ok(normalized)
        } else {
            self.normalize()
        }
    }
    pub fn picture_bytes(&self) -> Vec<u8> {
        self.picture
            .as_ref()
            .and_then(|s| STANDARD.decode(s).ok())
            .unwrap_or_default()
    }
}
pub fn load(db: &Connection) -> Result<Profile> {
    db.query_row("SELECT json FROM own_profile WHERE id=1", [], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .map(|s| serde_json::from_str(&s).map_err(Into::into))
    .unwrap_or_else(|| Ok(Profile::default()))
}
pub fn save(db: &Connection, profile: &Profile) -> Result<()> {
    db.execute(
        "INSERT INTO own_profile VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        [serde_json::to_string(profile)?],
    )?;
    Ok(())
}

pub fn cache_picture(
    db: &Connection,
    username: &str,
    bytes: Option<&[u8]>,
) -> Result<Option<String>> {
    use sha2::{Digest, Sha256};
    let Some(bytes) = bytes else {
        db.execute("DELETE FROM user_pictures WHERE username=?1", [username])?;
        return Ok(None);
    };
    ensure!(bytes.len() <= MAX_PICTURE, "Peer picture exceeds 3 MiB");
    let png = normalize_picture(bytes, false)?;
    let revision = format!("{:x}", Sha256::digest(&png));
    let tx = crate::store::write_transaction(db)?;
    tx.execute("INSERT INTO user_pictures VALUES(?1,?2,?3,strftime('%s','now')) ON CONFLICT(username) DO UPDATE SET data=excluded.data,revision=excluded.revision,updated_at=excluded.updated_at",rusqlite::params![username,png,revision])?;
    tx.execute("DELETE FROM user_pictures WHERE username IN (SELECT username FROM user_pictures ORDER BY updated_at DESC,username LIMIT -1 OFFSET 64)",[])?;
    tx.commit()?;
    Ok(Some(revision))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn png(image: &image::DynamicImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
    #[test]
    fn jpeg_orientation_is_applied_before_its_metadata_is_removed() {
        // Minimal little-endian TIFF with orientation 6 (rotate 90 degrees).
        let exif = vec![
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut jpeg = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100);
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .encode(&[100; 18], 3, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let png = normalize_picture(&jpeg, true).unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 3));
        assert!(!png.windows(4).any(|w| w == b"eXIf"));
    }
    #[test]
    fn png_processing_preserves_small_images_alpha_and_sixteen_bit_samples() {
        let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(17, 9, |x, y| {
            image::Rgba([x as u8, y as u8, 99, if x % 2 == 0 { 0 } else { 127 }])
        }));
        let bytes = normalize_picture(&png(&source), true).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(
            decoded,
            source.to_rgba8(),
            "No enlargement, alpha changes or invisible RGB removal"
        );
        let source16 =
            image::DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(5, 7, |x, y| {
                image::Rgba([x as u16 * 101, y as u16 * 301, 12345, 54321])
            }));
        let bytes = normalize_picture(&png(&source16), true).unwrap();
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().to_rgba16(),
            source16.to_rgba16()
        );
    }
    #[test]
    fn png_processing_fits_both_orientations_and_removes_source_metadata() {
        for (w, h, expected) in [(1024, 256, (512, 128)), (90, 900, (51, 512))] {
            let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w,
                h,
                image::Rgb([10, 20, 30]),
            ));
            let mut encoded = Vec::new();
            let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
            encoder
                .set_icc_profile(b"private source metadata".to_vec())
                .unwrap();
            encoder
                .write_image(source.as_bytes(), w, h, source.color().into())
                .unwrap();
            let bytes = normalize_picture(&encoded, true).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), expected);
            let mut at = 8;
            while at < bytes.len() {
                let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
                let kind = &bytes[at + 4..at + 8];
                assert!(
                    [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"]
                        .contains(&kind.try_into().unwrap()),
                    "Metadata chunk was retained"
                );
                at += length + 12;
            }
        }
    }
    #[test]
    fn uploads_accept_ten_mb_reject_over_limit_and_bound_decoded_pixels() {
        let mut bytes = png(&image::DynamicImage::ImageRgb8(image::RgbImage::new(1, 1)));
        bytes.resize(MAX_INPUT_PICTURE, 0);
        assert!(normalize_picture(&bytes, true).is_ok());
        bytes.push(0);
        assert!(
            normalize_picture(&bytes, true)
                .unwrap_err()
                .to_string()
                .contains("10 MB")
        );
        assert!(normalize_picture(b"\x89PNG\r\n\x1a\nmalformed", false).is_err());
        let large = png(&image::DynamicImage::ImageLuma8(image::GrayImage::new(
            6000, 4001,
        )));
        assert!(
            normalize_picture(&large, false)
                .unwrap_err()
                .to_string()
                .contains("24 million")
        );
    }
    #[test]
    fn profile_normalizes_interests_rejects_unsafe_images_and_survives_restart() {
        let p = Profile {
            description: "Music collector\n<script>literal text</script>".into(),
            likes: vec![" Jazz ".into(), "jazz".into()],
            away_message: "Back later".into(),
            ..Default::default()
        }
        .normalize()
        .unwrap();
        assert_eq!(p.likes, vec!["jazz"]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = crate::store::Store::open(&path).unwrap();
        save(&store.connection, &p).unwrap();
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        assert_eq!(load(&store.connection).unwrap().description, p.description);
        assert!(
            Profile {
                picture: Some(STANDARD.encode(b"<svg onload='alert()'/>")),
                ..Default::default()
            }
            .normalize()
            .is_err()
        );
        assert!(
            Profile {
                likes: vec!["jazz".into()],
                dislikes: vec!["Jazz".into()],
                ..Default::default()
            }
            .normalize()
            .is_err()
        );
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let picture = Profile {
            picture: Some(STANDARD.encode(encoded)),
            ..Default::default()
        }
        .normalize()
        .unwrap();
        assert!(picture.picture_bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            picture.clone().normalize_against(&picture).unwrap().picture,
            picture.picture,
            "Editing text must not recompress an unchanged picture"
        );
    }
    use image::ImageEncoder;
}
