use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, TextureHandle, TextureOptions};

use crate::config::{Config, config_path};
use crate::constants::MARGIN_3MM;
use crate::image_source::{self, LoadedImage};
use crate::ocr::{self, OcrClient, OcrUpload};

const SUCCESS_COLOR: Color32 = Color32::from_rgb(0x2e, 0x7d, 0x32);
const BOX_BORDER_COLOR: Color32 = Color32::from_gray(160);
const BOX_BORDER_WIDTH: f32 = 1.0;
/// TextEdit 默认左右内边距之和, 用于让输入框外沿精确落在 3mm 边距上.
const TEXT_EDIT_HORIZONTAL_MARGIN: f32 = 8.0;
/// 顶部复制条的最小高度.
const MIN_COPY_HEIGHT: f32 = 24.0;

/// OCR 单次请求的 UI 状态机. 请求中不允许再次提交.
enum OcrState {
  Idle,
  Requesting,
  Done,
  Error,
}

enum OcrOutcome {
  Success(String),
  Failure(String),
}

pub struct App {
  config: Config,
  config_path: PathBuf,
  key_input: String,
  /// 停止输入后延迟保存 API Key
  key_save_due: Option<Instant>,
  /// Ctrl+V 当前是否按下, 用于边沿触发
  paste_combo_down: bool,
  /// 当前帧图片区外框尺寸, 用于让 OCR 文本框保持同样大小
  image_area_size: Option<egui::Vec2>,
  image: Option<LoadedImage>,
  texture: Option<TextureHandle>,
  texture_seq: u64,
  ocr_state: OcrState,
  rx: Option<Receiver<OcrOutcome>>,
  result_text: String,
  /// 用户在结果区手动修改过内容
  result_dirty: bool,
  /// 因结果区已修改而未自动应用的新结果
  pending_result: Option<String>,
  copied_at: Option<Instant>,
}

impl App {
  pub fn new() -> Self {
    let path = config_path().to_path_buf();
    let (config, startup_log) = match Config::load(&path) {
      Ok(config) => (config, None),
      Err(_) if !path.exists() => (Config::default(), Some("未找到配置文件, 已使用默认配置".to_string())),
      Err(error) => (Config::default(), Some(format!("读取配置失败, 已使用默认配置: {error:#}"))),
    };
    let key_input = config.api_key.clone();

    let mut app = Self {
      config,
      config_path: path,
      key_input,
      key_save_due: None,
      paste_combo_down: false,
      image_area_size: None,
      image: None,
      texture: None,
      texture_seq: 0,
      ocr_state: OcrState::Idle,
      rx: None,
      result_text: String::new(),
      result_dirty: false,
      pending_result: None,
      copied_at: None,
    };
    if let Some(message) = startup_log {
      app.push_log(message);
    }
    app
  }

  fn push_log(&mut self, message: impl Into<String>) {
    if !self.result_text.is_empty() && !self.result_text.ends_with('\n') {
      self.result_text.push('\n');
    }
    self.result_text.push_str("[日志] ");
    self.result_text.push_str(&message.into());
  }

  fn config_section(&mut self, ui: &mut egui::Ui) {
    let margin = MARGIN_3MM;
    let mut commit_now = false;
    ui.add_space(margin);
    ui.horizontal(|ui| {
      ui.add_space(margin);
      ui.label("API Key:");
      let response = ui.add(
        egui::TextEdit::singleline(&mut self.key_input)
          .password(true)
          .hint_text("focr_...")
          .margin(egui::Margin::symmetric(4, 2))
          .desired_width((ui.available_width() - margin - TEXT_EDIT_HORIZONTAL_MARGIN).max(80.0)),
      );
      if response.changed() {
        self.key_save_due = Some(Instant::now() + Duration::from_millis(600));
        ui.ctx().request_repaint_after(Duration::from_millis(600));
      }
      if response.lost_focus() {
        commit_now = true;
      }
    });
    ui.add_space(6.0);
    let ctx = ui.ctx().clone();
    self.flush_key_save(&ctx, commit_now);
  }

  fn flush_key_save(&mut self, ctx: &egui::Context, force: bool) {
    if !force {
      match self.key_save_due {
        Some(due) if Instant::now() >= due => {}
        _ => return,
      }
    }
    self.key_save_due = None;
    self.save_key(ctx, force);
  }

  fn image_section(&mut self, ui: &mut egui::Ui) {
    let margin = MARGIN_3MM;
    ui.add_space(margin);
    let available = ui.available_rect_before_wrap();
    let rect = egui::Rect::from_min_max(
      egui::pos2(available.left() + margin, available.top()),
      egui::pos2(available.right() - margin, available.bottom() - margin),
    );
    if rect.width() < 1.0 || rect.height() < 1.0 {
      return;
    }
    self.image_area_size = Some(rect.size());

    let response = ui.interact(rect, ui.id().with("image_area"), egui::Sense::hover());
    let background = if response.hovered() {
      ui.visuals().widgets.hovered.bg_fill
    } else {
      ui.visuals().faint_bg_color
    };
    ui.painter().rect_filled(rect, 6.0, background);

    if let Some(texture) = &self.texture {
      let size = texture.size_vec2();
      let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
      let image_rect = egui::Rect::from_center_size(rect.center(), size * scale);
      egui::Image::new(texture).paint_at(ui, image_rect);
      if let Some(image) = &self.image {
        let caption = format!("{}  {} x {}", image.filename, image.width(), image.height());
        let galley = ui.painter().layout_no_wrap(caption, egui::FontId::proportional(12.0), Color32::WHITE);
        let padding = egui::vec2(6.0, 3.0);
        let caption_rect = egui::Rect::from_min_size(rect.left_top() + egui::vec2(6.0, 6.0), galley.size() + padding * 2.0);
        ui.painter().rect_filled(caption_rect, 3.0, Color32::from_black_alpha(150));
        ui.painter().galley(caption_rect.min + padding, galley, Color32::WHITE);
      }
    } else {
      let color = ui.visuals().weak_text_color();
      let galley = ui.painter().layout(
        "按 Ctrl+V 读取剪贴板图片\n或从文件管理器拖放图片".to_string(),
        egui::FontId::proportional(15.0),
        color,
        (rect.width() - 20.0).max(40.0),
      );
      let pos = rect.center() - galley.size() / 2.0;
      ui.painter().galley(pos, galley, color);
    }

    ui.painter()
      .rect_stroke(rect, 6.0, egui::Stroke::new(BOX_BORDER_WIDTH, BOX_BORDER_COLOR), egui::StrokeKind::Inside);
  }

  fn load_clipboard_image(&mut self, ctx: &egui::Context) {
    match image_source::from_clipboard_if_image() {
      Ok(Some(image)) => self.apply_image(ctx, image),
      Ok(None) => {}
      Err(error) => self.push_log(format!("{error:#}")),
    }
  }

  fn result_section(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
    let margin = MARGIN_3MM;
    let area = ui.max_rect().shrink2(egui::vec2(margin, margin));
    if area.width() < 1.0 || area.height() < 1.0 {
      return;
    }

    // 文本框与图片区外框同尺寸; 复制按钮占据顶部剩余高度.
    let fallback_size = egui::vec2(area.width(), (area.height() - margin - MIN_COPY_HEIGHT).max(1.0));
    let image_size = self.image_area_size.unwrap_or(fallback_size);
    let text_width = image_size.x.min(area.width());
    let max_text_height = (area.height() - margin - MIN_COPY_HEIGHT).max(1.0);
    let text_height = image_size.y.min(max_text_height);
    let copy_height = (area.height() - margin - text_height).max(1.0);
    let content_left = area.center().x - text_width / 2.0;

    let copy_rect = egui::Rect::from_min_size(egui::pos2(content_left, area.top()), egui::vec2(text_width, copy_height));
    let text_rect = egui::Rect::from_min_max(
      egui::pos2(content_left, copy_rect.bottom() + margin),
      egui::pos2(content_left + text_width, area.bottom()),
    );
    if text_rect.width() < 1.0 || text_rect.height() < 1.0 {
      return;
    }

    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let rows = ((text_rect.height() - 8.0) / row_height).floor().max(3.0) as usize;
    let mut text_ui = ui.new_child(
      egui::UiBuilder::new()
        .max_rect(text_rect)
        .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    // 文本框边框常显, 不依赖鼠标 hover.
    let text_box_border = egui::Stroke::new(BOX_BORDER_WIDTH, BOX_BORDER_COLOR);
    text_ui.visuals_mut().widgets.inactive.bg_stroke = text_box_border;
    text_ui.visuals_mut().widgets.hovered.bg_stroke = text_box_border;
    let response = text_ui.add_sized(
      [text_rect.width(), text_rect.height()],
      egui::TextEdit::multiline(&mut self.result_text)
        .desired_width(f32::INFINITY)
        .desired_rows(rows)
        .hint_text("OCR 结果与日志会显示在这里, 可手动修改后再复制")
        .code_editor(),
    );
    if response.changed() {
      self.result_dirty = true;
    }

    let show_copied = self.copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2));
    let copy_text = if show_copied {
      egui::RichText::new("已复制").size(15.0).color(SUCCESS_COLOR)
    } else {
      egui::RichText::new("复制").size(15.0)
    };
    let mut copy_ui = ui.new_child(
      egui::UiBuilder::new()
        .max_rect(copy_rect)
        .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if self.pending_result.is_some() {
      let apply_width = 110.0;
      if copy_ui
        .add_sized(
          [apply_width, copy_height],
          egui::Button::new(egui::RichText::new("应用新结果").size(13.0)),
        )
        .clicked()
        && let Some(text) = self.pending_result.take()
      {
        self.result_text = text;
        self.result_dirty = false;
      }
      copy_ui.add_space(margin);
      let copy_width = (copy_rect.width() - apply_width - margin).max(80.0);
      if copy_ui.add_sized([copy_width, copy_height], egui::Button::new(copy_text)).clicked() {
        ctx.copy_text(self.result_text.clone());
        self.copied_at = Some(Instant::now());
      }
    } else if copy_ui
      .add_sized([copy_rect.width(), copy_height], egui::Button::new(copy_text))
      .clicked()
    {
      ctx.copy_text(self.result_text.clone());
      self.copied_at = Some(Instant::now());
    }
  }

  fn load_path(&mut self, ctx: &egui::Context, path: &Path) {
    match image_source::from_path(path) {
      Ok(image) => self.apply_image(ctx, image),
      Err(error) => self.push_log(format!("{error:#}")),
    }
  }

  fn apply_image(&mut self, ctx: &egui::Context, image: LoadedImage) {
    let color_image = egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.rgba.as_raw());
    let texture = ctx.load_texture(format!("preview-{}", self.texture_seq), color_image, TextureOptions::LINEAR);
    self.texture_seq += 1;
    self.texture = Some(texture);
    self.image = Some(image);
    self.ocr_state = OcrState::Idle;
    self.result_text.clear();
    self.result_dirty = false;
    self.pending_result = None;
    self.copied_at = None;
    self.start_ocr(ctx);
  }

  fn save_key(&mut self, ctx: &egui::Context, force: bool) {
    let key = self.key_input.trim().to_string();
    if key == self.config.api_key {
      return;
    }
    if let Err(error) = ocr::validate_api_key(&key) {
      if force {
        self.push_log(error.user_message());
      }
      return;
    }

    let mut config = self.config.clone();
    config.api_key = key;
    match config.save(&self.config_path) {
      Ok(()) => {
        self.config = config;
        self.key_input = self.config.api_key.clone();
        if self.image.is_some() && matches!(self.ocr_state, OcrState::Idle | OcrState::Error) {
          self.start_ocr(ctx);
        }
      }
      Err(error) => self.push_log(format!("保存配置失败: {error:#}")),
    }
  }

  fn start_ocr(&mut self, ctx: &egui::Context) {
    let Some(image) = &self.image else {
      self.push_log("请先选择图片");
      self.ocr_state = OcrState::Error;
      return;
    };

    let api_key = self.key_input.trim().to_string();
    if let Err(error) = ocr::validate_api_key(&api_key) {
      self.push_log(error.user_message());
      self.ocr_state = OcrState::Error;
      return;
    }

    let filename = image.filename.clone();
    let upload = OcrUpload::new(image.png.clone(), filename);
    let (tx, rx) = mpsc::channel();
    self.rx = Some(rx);
    self.ocr_state = OcrState::Requesting;
    self.pending_result = None;

    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
      .name("mt-ocr".to_string())
      .stack_size(16 * 1024 * 1024)
      .spawn(move || {
        let outcome = match OcrClient::new() {
          Ok(client) => match client.recognize(upload, &api_key) {
            Ok(text) => OcrOutcome::Success(text),
            Err(error) => OcrOutcome::Failure(error.user_message()),
          },
          Err(error) => OcrOutcome::Failure(error.user_message()),
        };
        let _ = tx.send(outcome);
        ctx.request_repaint();
      });

    if let Err(error) = spawned {
      self.rx = None;
      self.push_log(format!("无法启动 OCR 线程: {error}"));
      self.ocr_state = OcrState::Error;
    }
  }

  fn poll_ocr(&mut self) {
    let Some(rx) = self.rx.take() else {
      return;
    };

    match rx.try_recv() {
      Ok(OcrOutcome::Success(text)) => {
        if self.result_text.trim().is_empty() || !self.result_dirty {
          self.result_text = text;
          self.result_dirty = false;
          self.pending_result = None;
        } else {
          self.pending_result = Some(text);
        }
        self.ocr_state = OcrState::Done;
      }
      Ok(OcrOutcome::Failure(message)) => {
        self.push_log(format!("识别失败: {message}"));
        self.ocr_state = OcrState::Error;
      }
      Err(TryRecvError::Empty) => {
        self.rx = Some(rx);
      }
      Err(TryRecvError::Disconnected) => {
        self.push_log("OCR 线程意外结束");
        self.ocr_state = OcrState::Error;
      }
    }
  }

  /// 只要 mt 窗口获得键盘焦点, Ctrl+V 就读取剪贴板图片, 不要求先点击图片区.
  fn handle_paste_shortcut(&mut self, ctx: &egui::Context) {
    let focused = ctx.input(|input| input.focused);
    if !focused {
      self.paste_combo_down = false;
      return;
    }

    let (ctrl_down, v_down) = ctrl_v_down();
    let combo_down = ctrl_down && v_down;
    if combo_down && !self.paste_combo_down {
      self.load_clipboard_image(ctx);
    }
    self.paste_combo_down = combo_down;
  }

  fn handle_dropped_files(&mut self, ctx: &egui::Context) {
    // 文件拖到窗口任意位置都接收, 不要求落在图片区.
    let dropped = ctx.input(|input| input.raw.dropped_files.clone());
    if dropped.is_empty() {
      return;
    }

    if let Some(path) = dropped.iter().find_map(|file| file.path.as_deref()) {
      self.load_path(ctx, path);
    } else {
      self.push_log("拖放的文件没有本地路径");
    }
  }
}

/// 读取当前 Ctrl 与 V 的物理按键状态.
#[cfg(target_os = "windows")]
fn ctrl_v_down() -> (bool, bool) {
  use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_V};

  // SAFETY: GetAsyncKeyState 只读取全局按键状态, 没有内存安全风险.
  unsafe { (GetAsyncKeyState(VK_CONTROL as i32) < 0, GetAsyncKeyState(VK_V as i32) < 0) }
}

#[cfg(not(target_os = "windows"))]
fn ctrl_v_down() -> (bool, bool) {
  (false, false)
}

impl eframe::App for App {
  fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
    egui::Color32::WHITE.to_normalized_gamma_f32()
  }

  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    self.poll_ocr();
    self.handle_paste_shortcut(ctx);
    self.handle_dropped_files(ctx);

    if self.copied_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(2)) {
      self.copied_at = None;
    }
    if self.copied_at.is_some() || self.key_save_due.is_some() {
      ctx.request_repaint_after(Duration::from_millis(100));
    }

    let screen = ctx.content_rect();
    let mid_x = screen.center().x;
    egui::SidePanel::left("left_panel")
      .exact_width(mid_x - screen.left())
      .resizable(false)
      .show_separator_line(false)
      .frame(egui::Frame::NONE)
      .show(ctx, |ui| {
        self.config_section(ui);
        self.image_section(ui);
      });
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
      self.result_section(ctx, ui);
    });

    let divider_color = ctx.style().visuals.widgets.noninteractive.bg_stroke.color;
    ctx
      .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("center_divider")))
      .vline(mid_x, screen.y_range(), egui::Stroke::new(1.0_f32, divider_color));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clear_color_is_white() {
    let app = App::new();
    let clear = eframe::App::clear_color(&app, &egui::Visuals::dark());
    assert_eq!(clear, Color32::WHITE.to_normalized_gamma_f32());
  }
}
