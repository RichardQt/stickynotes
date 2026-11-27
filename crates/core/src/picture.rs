//! Pictures pasted into a note.
//!
//! A note is one Markdown document: one row in the database, one `.md` file
//! on the WebDAV server. A picture therefore travels inside it as a `data:`
//! URL, `![图片](data:image/png;base64,…)`, and storage, sync, backups and
//! undo need nothing new.
//!
//! The price is size, so a picture is made reasonably small first: one
//! larger than [`MAX_SIDE`] is scaled down, a bitmap (what a screenshot puts
//! on the Windows clipboard) is compressed as PNG, and a photo whose PNG
//! would be large is stored as JPEG. A picture still over [`MAX_BYTES`] is
//! refused rather than making every save and upload of its note slow.

use std::fmt;
use std::io::Cursor;
use std::path::Path;

use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// The alt text of an embedded picture: what the note list, search and a
/// plain-text copy show in its place.
pub const ALT: &str = "图片";

/// Longest side, in pixels, a picture is kept at. Wider than the editor at
/// any common display scale, small enough that a full-screen 4K screenshot
/// does not weigh several megabytes.
pub const MAX_SIDE: u32 = 1920;

/// Largest encoded picture a note takes, before base64 (which adds a third).
pub const MAX_BYTES: usize = 6 * 1024 * 1024;

/// Files and clipboard data larger than this are not decoded at all.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

/// Most pictures one paste inserts (several image files copied at once).
pub const MAX_PER_PASTE: usize = 10;

/// The rules [`embed`] follows; tests use smaller ones.
#[derive(Debug, Clone, Copy)]
struct Rules {
    max_side: u32,
    max_bytes: usize,
    /// A PNG, JPEG, GIF or WebP up to this size that already fits is
    /// embedded as it is: no decoding, no loss, an animated GIF keeps moving.
    keep_as_is: usize,
    /// An opaque picture whose PNG is larger than this is also tried as
    /// JPEG, which is kept when it is less than half the size: photos, not
    /// the flat colours and text of a screenshot, which JPEG would smear.
    try_jpeg: usize,
}

const RULES: Rules = Rules {
    max_side: MAX_SIDE,
    max_bytes: MAX_BYTES,
    keep_as_is: 1024 * 1024,
    try_jpeg: 1024 * 1024,
};

const JPEG_QUALITY: u8 = 85;

/// A picture ready to be embedded in a note.
#[derive(Clone, PartialEq, Eq)]
pub struct Embedded {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl fmt::Debug for Embedded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embedded")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Embedded {
    /// `data:image/png;base64,…`
    pub fn data_url(&self) -> String {
        let mut url = format!("data:{};base64,", self.mime);
        base64::engine::general_purpose::STANDARD.encode_string(&self.bytes, &mut url);
        url
    }

    /// The Markdown that shows the picture: `![图片](data:…)`.
    pub fn markdown(&self) -> String {
        format!("![{ALT}]({})", self.data_url())
    }
}

/// Why a picture was not embedded. The messages are shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PictureError {
    #[error("不支持的图片格式")]
    Unsupported,
    #[error("图片文件太大（{:.0} MB），没有插入", mb(*.0))]
    InputTooLarge(usize),
    #[error("图片太大：压缩后仍有 {:.1} MB，超过 {} MB 的上限", mb(*.0), MAX_BYTES / 1024 / 1024)]
    TooLarge(usize),
    #[error("图片无法读取：{0}")]
    Unreadable(String),
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn unreadable(error: impl fmt::Display) -> PictureError {
    PictureError::Unreadable(error.to_string())
}

/// Make the picture in `bytes` (any format the clipboard or a file brings:
/// PNG, JPEG, GIF, WebP, BMP, TIFF) ready to embed in a note.
pub fn embed(bytes: &[u8]) -> Result<Embedded, PictureError> {
    embed_with(bytes, &RULES)
}

/// The picture in the file at `path`.
pub fn embed_file(path: &Path) -> Result<Embedded, PictureError> {
    let size = std::fs::metadata(path).map_err(unreadable)?.len();
    if size > MAX_INPUT_BYTES as u64 {
        return Err(PictureError::InputTooLarge(size as usize));
    }
    let bytes = std::fs::read(path).map_err(unreadable)?;
    embed(&bytes)
}

/// Whether `path` names a picture, by its extension: a file copied in
/// Explorer is pasted as a picture only then.
pub fn is_picture_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "bmp" | "dib" | "tif" | "tiff"
            )
        })
}

fn embed_with(bytes: &[u8], rules: &Rules) -> Result<Embedded, PictureError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(PictureError::InputTooLarge(bytes.len()));
    }
    let format = image::guess_format(bytes).map_err(|_| PictureError::Unsupported)?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tiff
    ) {
        return Err(PictureError::Unsupported);
    }
    // The header alone says how large the picture is.
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(unreadable)?;
    if let Some(mime) = web_mime(format) {
        if width <= rules.max_side && height <= rules.max_side && bytes.len() <= rules.keep_as_is {
            return Ok(Embedded {
                mime,
                bytes: bytes.to_vec(),
                width,
                height,
            });
        }
    }

    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decode_limits());
    let mut picture = reader.decode().map_err(unreadable)?;
    if format == ImageFormat::Bmp {
        picture = opaque_if_alpha_unset(picture);
    }
    if picture.width() > rules.max_side || picture.height() > rules.max_side {
        // Keeps the aspect ratio, fitting inside max_side × max_side.
        picture = picture.resize(rules.max_side, rules.max_side, FilterType::CatmullRom);
    }
    let opaque = is_opaque(&picture);
    // No alpha channel where every pixel is opaque: a quarter less to store.
    let picture = if opaque {
        DynamicImage::ImageRgb8(picture.to_rgb8())
    } else {
        DynamicImage::ImageRgba8(picture.to_rgba8())
    };

    let mut best = Embedded {
        mime: "image/png",
        bytes: encode_png(&picture)?,
        width: picture.width(),
        height: picture.height(),
    };
    if opaque && best.bytes.len() > rules.try_jpeg {
        let jpeg = encode_jpeg(&picture)?;
        if jpeg.len() * 2 < best.bytes.len() {
            best.mime = "image/jpeg";
            best.bytes = jpeg;
        }
    }
    if best.bytes.len() > rules.max_bytes {
        return Err(PictureError::TooLarge(best.bytes.len()));
    }
    Ok(best)
}

/// Formats every browser and Markdown editor shows, kept as they are.
fn web_mime(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

/// Guards against a small file that claims to be a gigantic picture.
fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits
}

/// A 32-bit clipboard bitmap often leaves its alpha bytes at zero, meaning
/// "no alpha" rather than "invisible". Only a picture with *every* pixel at
/// zero is read that way; real transparency is kept.
fn opaque_if_alpha_unset(picture: DynamicImage) -> DynamicImage {
    if !picture.color().has_alpha() {
        return picture;
    }
    let rgba = picture.to_rgba8();
    if rgba.pixels().all(|p| p[3] == 0) {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        picture
    }
}

fn is_opaque(picture: &DynamicImage) -> bool {
    if !picture.color().has_alpha() {
        return true;
    }
    match picture {
        DynamicImage::ImageRgba8(rgba) => rgba.pixels().all(|p| p[3] == u8::MAX),
        other => other.to_rgba8().pixels().all(|p| p[3] == u8::MAX),
    }
}

fn encode_png(picture: &DynamicImage) -> Result<Vec<u8>, PictureError> {
    let mut out = Cursor::new(Vec::new());
    picture
        .write_to(&mut out, ImageFormat::Png)
        .map_err(unreadable)?;
    Ok(out.into_inner())
}

fn encode_jpeg(picture: &DynamicImage) -> Result<Vec<u8>, PictureError> {
    let mut out = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    picture.write_with_encoder(encoder).map_err(unreadable)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn encoded(picture: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        picture.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    /// Flat colours and a few lines: what a screenshot looks like.
    fn screenshot(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            if y % 40 == 0 || x % 120 == 0 {
                Rgb([40, 40, 40])
            } else if y < 60 {
                Rgb([230, 236, 245])
            } else {
                Rgb([255, 255, 255])
            }
        }))
    }

    /// Smooth gradients with sensor-like noise: what a photo looks like.
    fn photo(width: u32, height: u32) -> DynamicImage {
        let mut seed = 0x2545_f491_u32;
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = (seed % 24) as u8;
            Rgb([
                ((x * 255 / width) as u8).saturating_add(n),
                ((y * 255 / height) as u8).saturating_add(n / 2),
                (((x + y) * 127 / (width + height)) as u8).saturating_add(n),
            ])
        }))
    }

    fn decode(embedded: &Embedded) -> DynamicImage {
        image::load_from_memory(&embedded.bytes).unwrap()
    }

    #[test]
    fn a_small_png_is_embedded_as_it_is() {
        let png = encoded(&screenshot(300, 200), ImageFormat::Png);
        let embedded = embed(&png).unwrap();
        assert_eq!(embedded.mime, "image/png");
        assert_eq!(embedded.bytes, png);
        assert_eq!((embedded.width, embedded.height), (300, 200));
    }

    #[test]
    fn a_clipboard_bitmap_is_compressed_as_png() {
        let bmp = encoded(&screenshot(400, 300), ImageFormat::Bmp);
        let embedded = embed(&bmp).unwrap();
        assert_eq!(embedded.mime, "image/png");
        assert!(embedded.bytes.len() < bmp.len() / 4, "{embedded:?}");
        let back = decode(&embedded);
        assert_eq!((back.width(), back.height()), (400, 300));
        assert_eq!(back.to_rgb8().get_pixel(5, 5), &Rgb([230, 236, 245]));
        // Opaque: stored without an alpha channel.
        assert!(!back.color().has_alpha());
    }

    #[test]
    fn a_large_picture_is_scaled_to_fit_and_keeps_its_aspect() {
        let png = encoded(&screenshot(3840, 1200), ImageFormat::Png);
        let embedded = embed(&png).unwrap();
        assert_eq!((embedded.width, embedded.height), (1920, 600));
        let back = decode(&embedded);
        assert_eq!((back.width(), back.height()), (1920, 600));
    }

    #[test]
    fn a_photo_becomes_jpeg_and_a_screenshot_stays_png() {
        let rules = Rules {
            try_jpeg: 64 * 1024,
            keep_as_is: 0,
            ..RULES
        };
        let photo = embed_with(&encoded(&photo(600, 400), ImageFormat::Png), &rules).unwrap();
        assert_eq!(photo.mime, "image/jpeg", "{photo:?}");
        let shot = embed_with(&encoded(&screenshot(600, 400), ImageFormat::Png), &rules).unwrap();
        assert_eq!(shot.mime, "image/png", "{shot:?}");
    }

    #[test]
    fn transparency_is_kept_and_unset_bitmap_alpha_is_not_invisible() {
        let mut rgba = RgbaImage::from_pixel(50, 50, Rgba([10, 20, 30, 255]));
        rgba.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        let png = encoded(&DynamicImage::ImageRgba8(rgba.clone()), ImageFormat::Png);
        let rules = Rules {
            keep_as_is: 0,
            ..RULES
        };
        let embedded = embed_with(&png, &rules).unwrap();
        assert_eq!(embedded.mime, "image/png");
        assert_eq!(decode(&embedded).to_rgba8().get_pixel(0, 0)[3], 0);

        // A bitmap whose alpha bytes were all left at zero.
        let unset = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([200, 100, 50, 0])));
        let fixed = opaque_if_alpha_unset(unset);
        assert!(!fixed.color().has_alpha());
        assert_eq!(fixed.to_rgb8().get_pixel(3, 3), &Rgb([200, 100, 50]));
    }

    #[test]
    fn a_picture_that_stays_too_large_is_refused() {
        let rules = Rules {
            max_bytes: 1024,
            keep_as_is: 0,
            ..RULES
        };
        let png = encoded(&photo(300, 300), ImageFormat::Png);
        assert!(matches!(embed_with(&png, &rules), Err(PictureError::TooLarge(_))));
        assert!(
            PictureError::TooLarge(7 * 1024 * 1024)
                .to_string()
                .contains("7.0 MB")
        );
    }

    #[test]
    fn what_is_not_a_picture_is_refused() {
        assert_eq!(embed(b"hello world"), Err(PictureError::Unsupported));
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
        assert_eq!(embed(svg), Err(PictureError::Unsupported));
        // A PNG signature with nothing behind it.
        assert!(matches!(
            embed(b"\x89PNG\r\n\x1a\n"),
            Err(PictureError::Unreadable(_))
        ));
    }

    #[test]
    fn the_markdown_is_an_image_with_a_data_url() {
        let embedded = Embedded {
            mime: "image/png",
            bytes: vec![1, 2, 3, 250],
            width: 1,
            height: 1,
        };
        assert_eq!(embedded.data_url(), "data:image/png;base64,AQID+g==");
        assert_eq!(embedded.markdown(), "![图片](data:image/png;base64,AQID+g==)");
    }

    #[test]
    fn a_picture_file_is_read_and_others_are_not_pictures() {
        let dir = std::env::temp_dir().join(format!("stickynotes-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.PNG");
        let png = encoded(&screenshot(64, 32), ImageFormat::Png);
        std::fs::write(&path, &png).unwrap();
        assert!(is_picture_file(&path));
        assert_eq!(embed_file(&path).unwrap().bytes, png);
        assert!(!is_picture_file(Path::new("notes.docx")));
        assert!(!is_picture_file(Path::new("README")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
