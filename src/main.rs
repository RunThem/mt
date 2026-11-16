// 仅 release 链接为 GUI 子系统(双击不弹控制台); debug 保留控制台方便开发看输出
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, ViewportBuilder};

mod app;
mod config;
mod constants;
mod image_source;
mod ocr;

/// 强制浅色主题, 并把面板/输入框底色统一为白色.
fn setup_theme(ctx: &egui::Context) {
  ctx.set_theme(egui::Theme::Light);

  let mut visuals = egui::Visuals::light();
  visuals.panel_fill = egui::Color32::WHITE;
  visuals.window_fill = egui::Color32::WHITE;
  visuals.extreme_bg_color = egui::Color32::WHITE;
  visuals.faint_bg_color = egui::Color32::from_gray(248);
  ctx.set_visuals(visuals);
}

/// 加载 Windows 自带中文字体作为 fallback, 避免 UI 中文显示为方框.
fn setup_fonts(ctx: &egui::Context) {
  const CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\msyhbd.ttc",
    "C:\\Windows\\Fonts\\simhei.ttf",
    "C:\\Windows\\Fonts\\simsun.ttc",
  ];

  let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) else {
    eprintln!("未找到可用的中文字体, 界面中文可能无法正常显示");
    return;
  };

  let mut fonts = FontDefinitions::default();
  fonts.font_data.insert("cjk".to_string(), Arc::new(FontData::from_owned(bytes)));
  for family in [FontFamily::Proportional, FontFamily::Monospace] {
    fonts.families.entry(family).or_default().push("cjk".to_string());
  }
  ctx.set_fonts(fonts);
}

fn main() {
  let options = eframe::NativeOptions {
    viewport: ViewportBuilder::default()
      .with_inner_size([constants::WINDOW_WIDTH, constants::WINDOW_HEIGHT])
      .with_min_inner_size([constants::MIN_WINDOW_WIDTH, constants::MIN_WINDOW_HEIGHT]),
    ..Default::default()
  };

  if let Err(error) = eframe::run_native(
    constants::APP_TITLE,
    options,
    Box::new(|cc| {
      setup_theme(&cc.egui_ctx);
      setup_fonts(&cc.egui_ctx);
      Ok(Box::new(app::App::new()))
    }),
  ) {
    eprintln!("启动失败: {error}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn theme_uses_white_background() {
    let ctx = egui::Context::default();
    setup_theme(&ctx);

    let visuals = ctx.style().visuals.clone();
    assert_eq!(visuals.panel_fill, egui::Color32::WHITE);
    assert_eq!(visuals.window_fill, egui::Color32::WHITE);
    assert_eq!(visuals.extreme_bg_color, egui::Color32::WHITE);
  }
}
