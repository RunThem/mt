use std::borrow::Cow;
use std::fmt;

use nyquest::blocking::{Body, Part, PartBody, Request};
use nyquest::{BlockingClient, ClientBuilder};
use serde::Deserialize;

use crate::constants::{API_KEY_PREFIX, API_URL, REQUEST_TIMEOUT};

/// OCR 失败原因. 用户可见文案集中在 `user_message` 中.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrError {
  MissingKey,
  InvalidKeyFormat,
  Unauthorized,
  InsufficientBalance,
  RateLimited,
  ServerError,
  Network,
  EmptyImage,
  /// 其他非 2xx 响应
  Api {
    status: u16,
    message: String,
  },
  /// 2xx 但响应体不是约定的 `{ "text": "..." }`
  InvalidResponse,
}

impl OcrError {
  pub fn user_message(&self) -> String {
    match self {
      Self::MissingKey => "请先配置 API Key".to_string(),
      Self::InvalidKeyFormat => format!("API Key 格式无效, 应以 {API_KEY_PREFIX} 开头"),
      Self::Unauthorized => "API Key 无效".to_string(),
      Self::InsufficientBalance => "账户余额不足".to_string(),
      Self::RateLimited => "请求过于频繁, 请稍后重试".to_string(),
      Self::ServerError => "服务端处理失败".to_string(),
      Self::Network => "网络请求失败, 请检查连接".to_string(),
      Self::EmptyImage => "图片内容为空".to_string(),
      Self::Api { status, message } if message.is_empty() => format!("OCR 请求失败 (HTTP {status})"),
      Self::Api { status, message } => format!("OCR 请求失败 (HTTP {status}): {message}"),
      Self::InvalidResponse => "无法解析服务端响应".to_string(),
    }
  }
}

impl fmt::Display for OcrError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.user_message())
  }
}

impl std::error::Error for OcrError {}

/// 待上传的图片. 统一为重新编码后的 PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrUpload {
  pub png: Vec<u8>,
  pub filename: String,
}

/// multipart 中单个字段的纯数据描述, 便于单测.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartPart {
  pub name: String,
  pub filename: Option<String>,
  pub content_type: String,
  pub body: Vec<u8>,
}

impl OcrUpload {
  pub fn new(png: Vec<u8>, filename: impl Into<String>) -> Self {
    let filename = filename.into();
    Self {
      png,
      filename: if filename.trim().is_empty() { "image.png".to_string() } else { filename },
    }
  }

  /// OCR 接口只要求一个名为 `image` 的文件字段.
  pub fn parts(&self) -> Vec<MultipartPart> {
    vec![MultipartPart {
      name: "image".to_string(),
      filename: Some(self.filename.clone()),
      content_type: "image/png".to_string(),
      body: self.png.clone(),
    }]
  }

  /// 构造 multipart 请求. 网络发送与响应解析在 `OcrClient::recognize` 中.
  pub fn into_request(self, api_key: &str) -> Result<Request, OcrError> {
    validate_api_key(api_key)?;
    if self.png.is_empty() {
      return Err(OcrError::EmptyImage);
    }

    let parts = self
      .parts()
      .into_iter()
      .map(|part| {
        let mut part_impl = Part::new_with_content_type(part.name, part.content_type, PartBody::bytes(part.body));
        if let Some(filename) = part.filename {
          part_impl = part_impl.with_filename(filename);
        }
        part_impl
      })
      .collect::<Vec<_>>();

    Ok(
      Request::post(API_URL)
        .with_header("Authorization", authorization_value(api_key))
        .with_header("Accept", "application/json")
        .with_body(Body::multipart(parts)),
    )
  }
}

/// 校验 Key 是否可用. 空值与格式错误在发请求前就给出明确提示.
pub fn validate_api_key(api_key: &str) -> Result<(), OcrError> {
  let api_key = api_key.trim();
  if api_key.is_empty() {
    return Err(OcrError::MissingKey);
  }
  if !api_key.starts_with(API_KEY_PREFIX) {
    return Err(OcrError::InvalidKeyFormat);
  }
  Ok(())
}

pub fn authorization_value(api_key: &str) -> Cow<'static, str> {
  Cow::Owned(format!("Bearer {}", api_key.trim()))
}

#[derive(Debug, Deserialize)]
struct OcrResponse {
  text: String,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
  error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
  #[serde(default)]
  code: String,
  #[serde(default)]
  message: String,
}

/// 解析 HTTP 状态与响应体. 与网络发送分离, 可直接单测.
pub fn parse_ocr_response(status: u16, body: &str) -> Result<String, OcrError> {
  match status {
    401 => return Err(OcrError::Unauthorized),
    402 => return Err(OcrError::InsufficientBalance),
    429 => return Err(OcrError::RateLimited),
    500 => return Err(OcrError::ServerError),
    _ => {}
  }

  if (200..300).contains(&status) {
    let response: OcrResponse = serde_json::from_str(body).map_err(|_| OcrError::InvalidResponse)?;
    return Ok(response.text);
  }

  let message = serde_json::from_str::<ErrorEnvelope>(body)
    .ok()
    .map(|envelope| {
      let mut message = envelope.error.message;
      if message.is_empty() {
        message = envelope.error.code;
      }
      message
    })
    .unwrap_or_default();
  Err(OcrError::Api { status, message })
}

/// OCR 客户端. 每次 OCR 在后台线程中创建一个实例并复用其连接.
pub struct OcrClient {
  client: BlockingClient,
}

impl OcrClient {
  pub fn new() -> Result<Self, OcrError> {
    let client = ClientBuilder::default()
      .request_timeout(REQUEST_TIMEOUT)
      .build_blocking()
      .map_err(|_| OcrError::Network)?;
    Ok(Self { client })
  }

  /// 发送请求并解析结果.
  pub fn recognize(&self, upload: OcrUpload, api_key: &str) -> Result<String, OcrError> {
    let request = upload.into_request(api_key)?;
    let response = self.client.request(request).map_err(|_| OcrError::Network)?;
    let status = response.status().code();
    let body = response.text().map_err(|_| OcrError::Network)?;
    parse_ocr_response(status, &body)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn upload() -> OcrUpload {
    OcrUpload::new(vec![0x89, b'P', b'N', b'G'], "sample.png")
  }

  #[test]
  fn builds_single_image_part() {
    let parts = upload().parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "image");
    assert_eq!(parts[0].filename.as_deref(), Some("sample.png"));
    assert_eq!(parts[0].content_type, "image/png");
    assert_eq!(parts[0].body, vec![0x89, b'P', b'N', b'G']);
  }

  #[test]
  fn blank_filename_falls_back() {
    let upload = OcrUpload::new(vec![1], "  ");
    assert_eq!(upload.filename, "image.png");
  }

  #[test]
  fn request_rejects_empty_image() {
    let result = OcrUpload::new(Vec::new(), "empty.png").into_request("focr_test");
    assert_eq!(result.unwrap_err(), OcrError::EmptyImage);
  }

  #[test]
  fn key_validation() {
    assert_eq!(validate_api_key(""), Err(OcrError::MissingKey));
    assert_eq!(validate_api_key("  "), Err(OcrError::MissingKey));
    assert_eq!(validate_api_key("abc"), Err(OcrError::InvalidKeyFormat));
    assert_eq!(validate_api_key("focr_test"), Ok(()));
    assert_eq!(validate_api_key("  focr_test  "), Ok(()));
  }

  #[test]
  fn authorization_header_value() {
    assert_eq!(authorization_value("  focr_test  ").as_ref(), "Bearer focr_test");
  }

  #[test]
  fn parses_success_text() {
    let text = parse_ocr_response(200, r##"{"text":"# Title\n\nHello"}"##).unwrap();
    assert_eq!(text, "# Title\n\nHello");
  }

  #[test]
  fn parses_empty_text_as_success() {
    assert_eq!(parse_ocr_response(200, r#"{"text":""}"#).unwrap(), "");
  }

  #[test]
  fn maps_known_error_statuses() {
    assert_eq!(parse_ocr_response(401, "{}"), Err(OcrError::Unauthorized));
    assert_eq!(parse_ocr_response(402, "{}"), Err(OcrError::InsufficientBalance));
    assert_eq!(parse_ocr_response(429, "{}"), Err(OcrError::RateLimited));
    assert_eq!(parse_ocr_response(500, "{}"), Err(OcrError::ServerError));
  }

  #[test]
  fn parses_other_error_message() {
    let error = parse_ocr_response(400, r#"{"error":{"code":"bad_request","message":"image is required"}}"#).unwrap_err();
    assert_eq!(
      error,
      OcrError::Api {
        status: 400,
        message: "image is required".to_string()
      }
    );
    assert_eq!(error.user_message(), "OCR 请求失败 (HTTP 400): image is required");
  }

  #[test]
  fn malformed_success_body_is_invalid_response() {
    assert_eq!(parse_ocr_response(200, "<html>proxy</html>"), Err(OcrError::InvalidResponse));
    assert_eq!(OcrError::InvalidResponse.user_message(), "无法解析服务端响应");
  }

  #[test]
  fn user_messages_match_contract() {
    assert_eq!(OcrError::Unauthorized.user_message(), "API Key 无效");
    assert_eq!(OcrError::InsufficientBalance.user_message(), "账户余额不足");
    assert_eq!(OcrError::RateLimited.user_message(), "请求过于频繁, 请稍后重试");
    assert_eq!(OcrError::ServerError.user_message(), "服务端处理失败");
    assert_eq!(OcrError::Network.user_message(), "网络请求失败, 请检查连接");
  }
}
