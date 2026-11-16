use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, TextureHandle, TextureOptions};

use crate::config::{Config, config_path};
use crate::constants::MARGIN_3MM;
use crate::image_source::{self, LoadedImage};
use crate::ocr::{self, OcrClient, OcrUpload};

const SUCCESS_COLOR: Color32 = Color32::from_rgb(0x2e, 0x7d, 0x32);
/// TextEdit 默认左右内边距之和, 用于让输入框外沿精确落在 3mm 边距上.
const TEXT_EDIT_HORIZONTAL_MARGIN: f32 = 8.0;

/// OCR 单次请求的 UI 状态机. 请求中不允许再次提交.
enum OcrState {
  Idle,
  Requesting { started: Instant },
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
  /// 图片区获得焦点后, Ctrl+C / Ctrl+V 读取剪贴板图片
  image_area_active: bool,
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
      image_area_active: false,
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
      if response.gained_focus() {
        self.image_area_active = false;
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

    let response = ui.interact(rect, ui.id().with("image_area"), egui::Sense::click());
    if response.clicked() {
      self.image_area_active = true;
      ui.ctx().memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
          memory.surrender_focus(focused);
        }
      });
    } else if self.image_area_active && ui.ctx().input(|input| input.pointer.any_click()) {
      self.image_area_active = false;
    }

    let ctx = ui.ctx().clone();
    // egui-winit 会把 Ctrl+C 转成 Event::Copy, 不会产生 Key::C.
    let shortcut = self.image_area_active && ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Copy)));
    if shortcut {
      self.load_clipboard(&ctx);
    }

    let background = if response.hovered() || self.image_area_active {
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
        "点击此处后按 Ctrl+C 读取剪贴板图片\n或从文件管理器拖放图片".to_string(),
        egui::FontId::proportional(15.0),
        color,
        (rect.width() - 20.0).max(40.0),
      );
      let pos = rect.center() - galley.size() / 2.0;
      ui.painter().galley(pos, galley, color);
    }

    if self.image_area_active {
      ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.5_f32, ui.visuals().selection.stroke.color),
        egui::StrokeKind::Inside,
      );
    }
  }

  fn load_clipboard(&mut self, ctx: &egui::Context) {
    match image_source::from_clipboard() {
      Ok(image) => self.apply_image(ctx, image),
      Err(error) => self.push_log(format!("{error:#}")),
    }
  }

  fn result_section(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
    let margin = MARGIN_3MM;
    let area = ui.max_rect().shrink2(egui::vec2(margin, margin));
    if area.width() < 1.0 || area.height() < 1.0 {
      return;
    }

    let copy_height = 52.0;
    let mut row_width = 180.0;
    if self.pending_result.is_some() {
      row_width += 130.0 + 12.0;
    }
    let row_rect = egui::Rect::from_min_size(
      egui::pos2(area.center().x - row_width / 2.0, area.max.y - copy_height),
      egui::vec2(row_width, copy_height),
    );
    let text_rect = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, row_rect.min.y - margin));
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
    let text_box_border = egui::Stroke::new(1.0_f32, Color32::from_gray(160));
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
    if response.gained_focus() {
      self.image_area_active = false;
    }
    if response.changed() {
      self.result_dirty = true;
    }

    let show_copied = self.copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2));
    let mut row_ui = ui.new_child(
      egui::UiBuilder::new()
        .max_rect(row_rect)
        .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if self.pending_result.is_some() {
      if row_ui
        .add_sized([130.0, 40.0], egui::Button::new(egui::RichText::new("应用新结果").size(15.0)))
        .clicked()
        && let Some(text) = self.pending_result.take()
      {
        self.result_text = text;
        self.result_dirty = false;
      }
      row_ui.add_space(12.0);
    }
    let copy_text = if show_copied {
      egui::RichText::new("已复制").size(22.0).color(SUCCESS_COLOR)
    } else {
      egui::RichText::new("复制").size(22.0)
    };
    if row_ui.add_sized([180.0, 52.0], egui::Button::new(copy_text)).clicked() {
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
        self.push_log("API Key 已保存");
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
    let upload = OcrUpload::new(image.png.clone(), filename.clone());
    self.push_log(format!("开始识别: {filename}"));
    let (tx, rx) = mpsc::channel();
    self.rx = Some(rx);
    self.ocr_state = OcrState::Requesting { started: Instant::now() };
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
        let elapsed = match &self.ocr_state {
          OcrState::Requesting { started } => started.elapsed(),
          _ => Duration::ZERO,
        };
        if self.result_text.trim().is_empty() || !self.result_dirty {
          self.result_text = text;
          self.result_dirty = false;
          self.pending_result = None;
        } else {
          self.pending_result = Some(text);
        }
        self.push_log(format!("识别完成, 耗时 {:.2} 秒", elapsed.as_secs_f32()));
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

  fn handle_dropped_files(&mut self, ctx: &egui::Context) {
    let dropped = ctx.input(|input| input.raw.dropped_files.clone());
    if dropped.is_empty() {
      return;
    }

    if let Some(path) = dropped.iter().find_map(|file| file.path.as_deref()) {
      self.load_path(ctx, path);
      if dropped.len() > 1 {
        self.push_log("一次只处理第一张图片");
      }
    } else {
      self.push_log("拖放的文件没有本地路径");
    }
  }
}

impl eframe::App for App {
  fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
    egui::Color32::WHITE.to_normalized_gamma_f32()
  }

  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    self.poll_ocr();
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
