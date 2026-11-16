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

/// 内置 MapleMono 中文字体, 保证 Windows 上中文显示一致.
fn setup_fonts(ctx: &egui::Context) {
  const FONT_NAME: &str = "MapleMonoNormalNL-NF-CN";

  let mut fonts = FontDefinitions::default();
  fonts.font_data.insert(
    FONT_NAME.to_string(),
    Arc::new(FontData::from_static(include_bytes!("MapleMonoNormalNL-NF-CN-Bold.ttf"))),
  );
  for family in [FontFamily::Proportional, FontFamily::Monospace] {
    fonts.families.entry(family).or_default().insert(0, FONT_NAME.to_string());
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
