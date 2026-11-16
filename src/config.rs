use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::CONFIG_FILE_PATH;

/// 持久化配置. 未知字段容忍, 缺失字段回退默认值.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
  pub api_key: String,
}

impl Config {
  /// 读取配置. 文件不存在或解析失败时返回错误, 由调用方决定是否回退默认值.
  pub fn load(path: &Path) -> Result<Self> {
    let content = fs::read_to_string(path).with_context(|| format!("读取配置文件 {} 失败", path.display()))?;
    let config = serde_json::from_str(&content).with_context(|| format!("解析配置文件 {} 失败", path.display()))?;
    Ok(config)
  }

  /// 保存配置. 自动创建父目录, 使用缩进 JSON 方便人工查看.
  pub fn save(&self, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
      && !parent.as_os_str().is_empty()
    {
      fs::create_dir_all(parent).with_context(|| format!("创建配置目录 {} 失败", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
    fs::write(path, content).with_context(|| format!("写入配置文件 {} 失败", path.display()))?;
    Ok(())
  }

  pub fn is_configured(&self) -> bool {
    !self.api_key.trim().is_empty()
  }
}

/// 默认配置路径
pub fn config_path() -> &'static Path {
  Path::new(CONFIG_FILE_PATH)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrip_json() {
    let config = Config {
      api_key: "focr_test_key".to_string(),
    };
    let json = serde_json::to_string(&config).unwrap();
    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(config, back);
  }

  #[test]
  fn missing_field_uses_default() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config, Config::default());
    assert!(!config.is_configured());
  }

  #[test]
  fn unknown_fields_are_tolerated() {
    let config: Config = serde_json::from_str(r#"{"api_key":"focr_x","legacy_field":42}"#).unwrap();
    assert_eq!(config.api_key, "focr_x");
    assert!(config.is_configured());
  }

  #[test]
  fn blank_key_is_not_configured() {
    let config = Config { api_key: "   ".to_string() };
    assert!(!config.is_configured());
  }
}
