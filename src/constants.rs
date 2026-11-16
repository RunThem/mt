use std::time::Duration;

/// 窗口标题
pub const APP_TITLE: &str = "mt - OCR 文字提取";

/// freeocr.ai OCR 接口
pub const API_URL: &str = "https://freeocr.ai/api/v1/platform/ocr";

/// 配置固定保存在 C:\Build, 沿用 ct 的目录约定
pub const CONFIG_FILE_PATH: &str = "C:\\Build\\mt-config.json";

/// 默认宽度: 960 时结果文本框约 457px, 增加约半个文本框宽度后文本框约 577px.
pub const WINDOW_WIDTH: f32 = 1200.0;
pub const WINDOW_HEIGHT: f32 = 760.0;
pub const MIN_WINDOW_WIDTH: f32 = 720.0;
pub const MIN_WINDOW_HEIGHT: f32 = 560.0;

/// 单次 OCR 请求超时
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 预览与上传图片的最长边上限(与 API 内部处理尺寸一致)
pub const MAX_IMAGE_EDGE: u32 = 2048;

/// API Key 固定前缀
pub const API_KEY_PREFIX: &str = "focr_";

/// 3mm 对应的逻辑像素(96 DPI), 用于窗口边距
pub const MARGIN_3MM: f32 = 11.34;
