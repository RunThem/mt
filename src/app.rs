use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, TextureHandle, TextureOptions};

use crate::config::{Config, config_path};
use crate::constants::API_KEY_PREFIX;
use crate::image_source::{self, LoadedImage};
use crate::ocr::{self, OcrClient, OcrUpload};

const SUCCESS_COLOR: Color32 = Color32::from_rgb(0x2e, 0x7d, 0x32);
const WARN_COLOR: Color32 = Color32::from_rgb(0xb2, 0x6a, 0x00);

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

  fn is_requesting(&self) -> bool {
    matches!(self.ocr_state, OcrState::Requesting { .. })
  }

  fn key_panel(&mut self, ctx: &egui::Context) {
    let mut commit_now = false;
    egui::TopBottomPanel::top("api_key_panel").show(ctx, |ui| {
      ui.add_space(6.0);
      ui.horizontal(|ui| {
        ui.label("API Key:");
        let response = ui.add(
          egui::TextEdit::singleline(&mut self.key_input)
            .password(true)
            .hint_text("focr_...")
            .desired_width(ui.available_width()),
        );
        if response.changed() {
          self.key_save_due = Some(Instant::now() + Duration::from_millis(600));
          ctx.request_repaint_after(Duration::from_millis(600));
        }
        if response.lost_focus() {
          commit_now = true;
        }
      });
      ui.add_space(6.0);
    });
    self.flush_key_save(commit_now);
  }

  fn flush_key_save(&mut self, force: bool) {
    if !force {
      match self.key_save_due {
        Some(due) if Instant::now() >= due => {}
        _ => return,
      }
    }
    self.key_save_due = None;
    self.save_key(force);
  }

  fn image_section(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
    let requesting = self.is_requesting();
    ui.horizontal(|ui| {
      if ui.add_enabled(!requesting, egui::Button::new("打开图片")).clicked() {
        self.pick_file(ctx);
      }
      if ui.add_enabled(!requesting, egui::Button::new("粘贴图片")).clicked() {
        self.paste_image(ctx);
      }
      if self.image.is_some() && ui.add_enabled(!requesting, egui::Button::new("清除图片")).clicked() {
        self.clear_image();
      }

      ui.add_space(12.0);
      if let Some(image) = &self.image {
        ui.label(egui::RichText::new(&image.filename).strong());
        ui.label(format!("{} x {}", image.width(), image.height()));
      } else {
        ui.label(egui::RichText::new("可拖放图片到窗口").weak());
      }
    });
  }

  fn preview(&self, ui: &mut egui::Ui) {
    let height = (ui.available_height() * 0.42).clamp(140.0, 420.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());

    if let Some(texture) = &self.texture {
      let size = texture.size_vec2();
      let scale = (rect.width() / size.x).min(rect.height() / size.y).min(1.0);
      let image_rect = egui::Rect::from_center_size(rect.center(), size * scale);
      egui::Image::new(texture).paint_at(ui, image_rect);
    } else {
      ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "尚未选择图片",
        egui::FontId::proportional(16.0),
        ui.visuals().weak_text_color(),
      );
    }
  }

  fn result_section(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.separator();
    ui.horizontal(|ui| {
      let can_ocr = self.image.is_some() && !self.key_input.trim().is_empty() && !self.is_requesting();
      if ui.add_enabled(can_ocr, egui::Button::new("开始 OCR")).clicked() {
        self.start_ocr(ctx);
      }

      ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add_enabled(!self.result_text.is_empty(), egui::Button::new("复制")).clicked() {
          ctx.copy_text(self.result_text.clone());
          self.copied_at = Some(Instant::now());
        }
        if self.copied_at.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
          ui.colored_label(SUCCESS_COLOR, "已复制");
        }
        if ui.add_enabled(!self.result_text.is_empty(), egui::Button::new("清空")).clicked() {
          self.result_text.clear();
          self.result_dirty = false;
          self.pending_result = None;
        }
      });
    });

    if self.pending_result.is_some() {
      let mut apply = false;
      let mut discard = false;
      ui.horizontal(|ui| {
        ui.colored_label(WARN_COLOR, "已保留手动修改的内容, 新结果未自动覆盖");
        if ui.button("应用新结果").clicked() {
          apply = true;
        }
        if ui.button("丢弃").clicked() {
          discard = true;
        }
      });
      if apply && let Some(text) = self.pending_result.take() {
        self.result_text = text;
        self.result_dirty = false;
      }
      if discard {
        self.pending_result = None;
      }
    }

    ui.add_space(4.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
      let response = ui.add(
        egui::TextEdit::multiline(&mut self.result_text)
          .desired_width(f32::INFINITY)
          .desired_rows(12)
          .hint_text("OCR 结果会显示在这里, 可手动修改后再复制")
          .code_editor(),
      );
      if response.changed() {
        self.result_dirty = true;
      }
    });
  }

  fn pick_file(&mut self, ctx: &egui::Context) {
    let Some(path) = rfd::FileDialog::new()
      .add_filter("图片", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"])
      .pick_file()
    else {
      return;
    };
    self.load_path(ctx, &path);
  }

  fn paste_image(&mut self, ctx: &egui::Context) {
    if self.is_requesting() {
      return;
    }
    match image_source::from_clipboard() {
      Ok(image) => self.apply_image(ctx, image),
      Err(error) => self.push_log(format!("{error:#}")),
    }
  }

  fn load_path(&mut self, ctx: &egui::Context, path: &Path) {
    if self.is_requesting() {
      return;
    }
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
    self.pending_result = None;
  }

  fn clear_image(&mut self) {
    self.image = None;
    self.texture = None;
    self.ocr_state = OcrState::Idle;
    self.pending_result = None;
  }

  fn save_key(&mut self, force: bool) {
    let key = self.key_input.trim().to_string();
    if key == self.config.api_key {
      return;
    }
    if key.is_empty() {
      if force {
        self.push_log("API Key 不能为空");
      }
      return;
    }
    if !key.starts_with(API_KEY_PREFIX) {
      if force {
        self.push_log(format!("API Key 格式无效, 应以 {API_KEY_PREFIX} 开头"));
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
      }
      Err(error) => self.push_log(format!("保存配置失败: {error:#}")),
    }
  }

  fn start_ocr(&mut self, ctx: &egui::Context) {
    if self.is_requesting() {
      return;
    }
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
    if self.is_requesting() {
      return;
    }
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
  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    self.poll_ocr();
    self.handle_dropped_files(ctx);

    if self.copied_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(2)) {
      self.copied_at = None;
    }
    if self.is_requesting() || self.copied_at.is_some() {
      ctx.request_repaint_after(Duration::from_millis(100));
    }

    self.key_panel(ctx);
    egui::CentralPanel::default().show(ctx, |ui| {
      self.image_section(ctx, ui);
      ui.add_space(6.0);
      self.preview(ui);
      ui.add_space(6.0);
      self.result_section(ctx, ui);
    });
  }
}
