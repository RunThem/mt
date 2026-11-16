use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use image::{ImageFormat, RgbaImage, imageops::FilterType};

use crate::constants::MAX_IMAGE_EDGE;

/// 已解码并统一为 RGBA + PNG 的图片. 预览与上传共用同一份数据.
#[derive(Debug)]
pub struct LoadedImage {
  pub filename: String,
  pub png: Vec<u8>,
  pub rgba: RgbaImage,
}

impl LoadedImage {
  pub fn width(&self) -> u32 {
    self.rgba.width()
  }

  pub fn height(&self) -> u32 {
    self.rgba.height()
  }
}

/// 从本地文件加载图片. 支持 image crate 已启用的常见格式.
pub fn from_path(path: &Path) -> Result<LoadedImage> {
  let bytes = fs::read(path).with_context(|| format!("读取图片 {} 失败", path.display()))?;
  let filename = path.file_name().and_then(|name| name.to_str()).unwrap_or("image");
  from_encoded_bytes(&bytes, filename)
}

/// 从剪贴板读取图片. 只有文本或空剪贴板时返回明确错误.
pub fn from_clipboard() -> Result<LoadedImage> {
  let mut clipboard = arboard::Clipboard::new().context("打开剪贴板失败")?;
  let image = match clipboard.get_image() {
    Ok(image) => image,
    Err(arboard::Error::ContentNotAvailable) => return Err(anyhow!("剪贴板中没有图片")),
    Err(error) => return Err(anyhow!("读取剪贴板图片失败: {error}")),
  };

  let rgba = RgbaImage::from_raw(image.width as u32, image.height as u32, image.bytes.into_owned()).context("剪贴板图片数据尺寸不一致")?;
  normalize(rgba, "clipboard.png")
}

/// 解码任意受支持格式, 统一转为 RGBA 后重新编码为 PNG.
pub fn from_encoded_bytes(bytes: &[u8], filename: &str) -> Result<LoadedImage> {
  let image = image::load_from_memory(bytes).context("图片解码失败, 请确认文件格式")?;
  normalize(image.to_rgba8(), filename)
}

fn normalize(rgba: RgbaImage, filename: &str) -> Result<LoadedImage> {
  if rgba.width() == 0 || rgba.height() == 0 {
    bail!("图片尺寸无效");
  }

  let rgba = resize_to_fit(rgba, MAX_IMAGE_EDGE);
  let mut png = Vec::new();
  image::DynamicImage::ImageRgba8(rgba.clone())
    .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
    .context("PNG 编码失败")?;

  Ok(LoadedImage {
    filename: png_filename(filename),
    png,
    rgba,
  })
}

/// 最长边超过 `max_edge` 时等比缩小, 保证预览和上传尺寸一致.
pub fn resize_to_fit(image: RgbaImage, max_edge: u32) -> RgbaImage {
  let (width, height) = image.dimensions();
  let longest = width.max(height);
  if longest <= max_edge {
    return image;
  }

  let scale = max_edge as f64 / longest as f64;
  let new_width = ((width as f64 * scale).round() as u32).max(1);
  let new_height = ((height as f64 * scale).round() as u32).max(1);
  image::imageops::resize(&image, new_width, new_height, FilterType::Triangle)
}

/// 上传文件名统一以 .png 结尾, 并去掉路径与换行等不安全字符.
fn png_filename(filename: &str) -> String {
  let base = Path::new(filename).file_name().and_then(|name| name.to_str()).unwrap_or("image");
  let stem = Path::new(base).file_stem().and_then(|name| name.to_str()).unwrap_or("image");
  let stem: String = stem
    .chars()
    .map(|ch| if ch == '\r' || ch == '\n' || ch == '"' { '_' } else { ch })
    .collect();
  let stem = stem.trim();
  if stem.is_empty() {
    "image.png".to_string()
  } else {
    format!("{stem}.png")
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use image::Rgba;

  fn sample(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]))
  }

  #[test]
  fn small_image_is_not_resized() {
    let image = resize_to_fit(sample(640, 480), MAX_IMAGE_EDGE);
    assert_eq!(image.dimensions(), (640, 480));
  }

  #[test]
  fn large_image_is_scaled_to_max_edge() {
    let image = resize_to_fit(sample(4096, 2048), MAX_IMAGE_EDGE);
    assert_eq!(image.dimensions(), (2048, 1024));
  }

  #[test]
  fn portrait_image_is_scaled_by_longest_edge() {
    let image = resize_to_fit(sample(1000, 3000), MAX_IMAGE_EDGE);
    assert_eq!(image.dimensions(), (683, 2048));
  }

  #[test]
  fn normalize_encodes_png_and_renames_file() {
    let loaded = normalize(sample(2, 2), "photo.jpg").unwrap();
    assert_eq!(loaded.filename, "photo.png");
    assert_eq!(loaded.png[..8], [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let decoded = image::load_from_memory(&loaded.png).unwrap().to_rgba8();
    assert_eq!(decoded.dimensions(), (2, 2));
  }

  #[test]
  fn encoded_bytes_roundtrip() {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(sample(3, 2))
      .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
      .unwrap();
    let loaded = from_encoded_bytes(&png, "sample.png").unwrap();
    assert_eq!(loaded.width(), 3);
    assert_eq!(loaded.height(), 2);
  }

  #[test]
  fn invalid_bytes_report_decode_error() {
    let error = from_encoded_bytes(b"not an image", "bad.png").unwrap_err();
    assert!(error.to_string().contains("图片解码失败"));
  }

  #[test]
  fn filename_is_sanitized() {
    assert_eq!(png_filename("C:\\tmp\\a.jpg"), "a.png");
    assert_eq!(png_filename("line\r\nbreak.png"), "line__break.png");
    assert_eq!(png_filename(""), "image.png");
  }
}
