# mt

Windows 优先的极简 OCR 桌面工具. 把剪贴板或本地图片交给 [freeocr.ai](https://freeocr.ai), 几秒内得到可编辑文本.

mt 只有一个窗口, 没有菜单和多层对话框: 窗口获得焦点后按 `Ctrl+V`, 或把图片拖进窗口, 识别会自动开始.

## 功能

- **剪贴板直读**: 窗口聚焦时按 `Ctrl+V` 读取剪贴板图片, 不需要先点击图片区.
- **拖放载入**: 从文件管理器把图片拖到窗口任意位置.
- **自动识别**: 图片载入后立即请求 freeocr.ai, 无需手动触发; 新图片会覆盖旧图并重新识别.
- **结果可编辑**: 右侧文本框可直接修改识别结果, 一键复制. 请求期间手动修改的内容不会被覆盖, 可点击 `应用新结果`.
- **纯文本结果**: 请求使用 `format=text`, 直接显示剥离 Markdown 标记的纯文本, 便于复制.
- **配置自保存**: 输入 `focr_` 开头的 API Key 后自动保存, 输入框掩码显示.
- **统一 PNG 上传**: 支持 PNG / JPEG / GIF / BMP / WebP / TIFF 与剪贴板位图; 上传前统一解码为 RGBA 并重新编码为 PNG.

## 使用

### 1. 准备 API Key

在 <https://freeocr.ai/platform/keys> 创建 Key, 形如 `focr_xxx`, 并确认账户有可用余额.

### 2. 构建并运行

需要 Rust stable 与 edition 2024:

```bash
cargo build --release
```

运行 `target\release\mt.exe`. 开发时也可以直接:

```bash
cargo run
```

Debug 构建保留控制台输出; Release 构建使用 GUI 子系统, 双击不会弹出控制台窗口.

### 3. 识别图片

1. 启动 mt, 在左上角 `API Key` 输入框填入 Key, 停止输入约 0.6 秒或移开焦点后自动保存.
2. 保持 mt 窗口焦点, 按 `Ctrl+V` 粘贴剪贴板图片, 或把图片文件拖进窗口.
3. 图片载入后自动开始 OCR, 结果显示在右侧.
4. 按需修改文本, 点击 `复制` 写入剪贴板.

> `Ctrl+C` 保留给文本选择复制, mt 不会拦截. 剪贴板中没有图片时, `Ctrl+V` 保持普通文本粘贴行为.

## 配置

| 项目 | 值 |
| --- | --- |
| 配置文件 | `C:\Build\mt-config.json` |
| API 端点 | `https://freeocr.ai/api/v1/platform/ocr` |
| 请求格式 | `multipart/form-data`, 字段 `image` |
| 响应格式 | `format=text`, 返回纯文本 (`format` 的 API 默认值为 `json`) |
| 请求超时 | 120 秒 |
| 图片最长边 | 2048 px, 超出则等比缩小 |

配置为普通 JSON, API Key 以明文保存. 文件不存在或解析失败时, mt 会回退默认配置并在结果区写入 `[日志]` 提示, 不会崩溃.

## 错误提示

| HTTP / 情况 | 提示 |
| --- | --- |
| 401 | `API Key 无效` |
| 402 | `账户余额不足` |
| 429 | `请求过于频繁, 请稍后重试` |
| 500 | `服务端处理失败` |
| 网络异常 | `网络请求失败, 请检查连接` |

启动日志, 配置读写失败和 OCR 失败都会写入结果区, 以 `[日志]` 开头; OCR 成功时不会追加日志.

## 工作原理

```text
src/
├── main.rs         # eframe 入口, 主题与内置字体, 窗口配置
├── app.rs          # UI 状态机, 快捷键 / 拖放, 后台线程调度
├── config.rs       # 配置模型与 JSON 读写
├── ocr.rs          # API Key 校验, multipart 请求构造, 响应与错误解析
├── image_source.rs # 文件 / 剪贴板 -> RGBA -> PNG
└── constants.rs    # 端点, 配置路径, 尺寸, 超时
```

- GUI 使用 `eframe` / `egui`, 内置 Maple Mono 中文字体, 保证中文显示一致.
- HTTP 使用 `nyquest` 的阻塞客户端; 请求在后台线程执行, 通过 `mpsc` 把结果交回 UI, 不阻塞界面.
- OCR 状态机为 `Idle -> Requesting -> Done / Error`, 同一时间只处理一张图片.

## 开发

```bash
cargo fmt
cargo clippy
cargo test
```

仓库内 `.cargo/config.toml` 为 Windows 下的 rustc 和 GUI 进程预留了较大的栈, 用于规避部分安全软件压缩子进程栈导致的 `0xc00000fd`.

## 注意

- 仅支持 Windows 10 / 11. `Ctrl+V` 检测使用 Win32 `GetAsyncKeyState`, 依赖窗口焦点.
- 图片会上传到 freeocr.ai 进行识别, 请勿处理敏感内容, 或先自行评估其服务条款.
- API Key 以明文保存在本机配置文件中, 不要提交配置文件, 也不要分享 Key.
- 仓库内置的 Maple Mono 字体遵循 SIL Open Font License 1.1, 许可证文本见 [`src/MapleMono-OFL.txt`](src/MapleMono-OFL.txt).

## 许可证

本项目代码使用 [MIT License](LICENSE) 授权.

内置的 `src/MapleMonoNormalNL-NF-CN-Bold.ttf` 字体来自 [Maple Mono](https://github.com/subframe7536/maple-font), 遵循 SIL Open Font License 1.1, 许可证文本见 [`src/MapleMono-OFL.txt`](src/MapleMono-OFL.txt).
