# AGENTS.md

## 项目目标

`mt` 是一个 Windows 优先的 Rust 桌面 GUI, 用于调用 freeocr.ai 的 OCR API, 把图片中的文字提取为可编辑文本.

三大功能:
1. 配置 API Key: 输入, 自动保存, 掩码显示, 支持后续修改.
2. 图片预览: 窗口获得焦点后按 Ctrl+C 读取剪贴板图片, 支持拖放; 显示缩略图, 文件名, 尺寸.
3. OCR: 上传图片到 freeocr.ai, 展示返回文本, 提供一键复制按钮.

## 全局约定

- **语言**: 本文件及所有输出统一使用中文, 代码标识符与专有名词保留英文原文.
- **标点**: 所有输出(写入文件的注释 / 文档, 以及回显给用户的回复), 无论中文还是英文, 一律使用英文标点(`,` `.` `:` `;` `(` `)` `-` `_` `/`), 不使用任何全角中文标点.
- **Rust 代码**: 2 空格缩进(见 `rustfmt.toml`), edition 2024; 用户可见文案(UI / 日志 / 报错)用中文, 标识符用英文.
- **提交信息**: Conventional Commits, 纯 ASCII 英文(终端回显中文会 GBK 乱码, 故 commit message 一律用英文).

## 构建与检查

```bash
cargo build            # Debug 构建
cargo build --release  # Release 构建
cargo run              # 构建并运行(GUI)
cargo fmt              # 格式化, 必须保持 2 空格缩进
cargo clippy           # Lint, 目标 0 warnings
cargo test             # 单元测试
```

- 根目录尚无 `rustfmt.toml`, 开始写代码前按 ct 项目补齐: `tab_spaces = 2`, `max_width = 150`, `newline_style = "Unix"`, `format_strings = false`.
- release profile 参考 ct: `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `strip = true`.
- 当前没有测试. 新增逻辑至少覆盖: 配置 JSON roundtrip, multipart 请求构造, OCR 响应与错误解析. 真实网络测试需手动提供 Key, 不要提交 Key.

## 技术栈

依赖直接从同级 `D:\Code\ct\Cargo.toml` 取版本, 保持两项目一致:

- **GUI**: `eframe` / `egui` 0.33.3. 入口用 `eframe::run_native`, 事件循环必须在主线程.
- **字体**: 内置 `src/MapleMonoNormalNL-NF-CN-Bold.ttf`(来自 ct 项目), 作为 Proportional / Monospace 主字体, 保证中文显示.
- **HTTP**: `nyquest` + `nyquest-preset`, 使用 `BlockingClient`; 需开启 `multipart` feature(`features = ["json", "blocking-stream", "multipart"]`), 因为 OCR 接口是 `multipart/form-data`.
- **JSON**: `serde` + `serde_json`; **错误**: `anyhow`.
- **剪贴板图片**: `arboard`. eframe / egui 原生只处理文本剪贴板, 读取剪贴板图片必须用 `arboard::Clipboard::get_image`.
- **图片解码**: `image` crate. 上传前统一解码为 RGBA 并重新编码为 PNG, 避免源文件格式或剪贴板 BGRA / RGBA 差异.
- `libu` 是 ct 的私有依赖, 本项目管理简单, 默认不引入.

## freeocr.ai API 契约

官方文档: https://freeocr.ai/zh-sc/platform/docs

- **端点**: `POST https://freeocr.ai/api/v1/platform/ocr`.
- **认证**: 请求头 `Authorization: Bearer focr_xxx`. Key 以 `focr_` 开头, 在 https://freeocr.ai/platform/keys 创建.
- **请求体**: `multipart/form-data`, 必填字段 `image`(图片文件).
- **查询参数**: `format` 可选, `json`(默认) / `text` / `docx`; `version` 可选, 默认 `261029`, 旧版 `260428`.
- **成功响应**: `Content-Type: application/json`, 结构 `{ "text": "..." }`. `json` 格式返回保留布局的 GitHub Flavored Markdown(含 HTML 表格和 LaTeX), `text` 返回纯文本.
- **错误响应**: 401 无效 Key, 402 余额不足, 429 触发限流, 500 服务端处理失败. 响应体形如 `{ "error": { "code": "...", "message": "..." } }`.
- **限流**: 默认每个 Key 200 请求 / 分钟.
- 仅需 `/ocr`; 不要实现 `/table` 或其他端点.
- Key 只能放在 `Authorization` header, 不要拼进 URL, 不要写入日志或错误弹窗.

## 架构建议与边界

```text
src/
├── main.rs       # eframe 入口, 主线程事件循环, 窗口配置
├── app.rs        # UI 状态与渲染, 操作分发, 不直接发 HTTP
├── config.rs     # 配置模型, JSON 读写, 默认值
├── ocr.rs        # OCR 客户端: 校验参数, 构造 multipart, 发送请求, 解析响应
├── image_source.rs # 文件 / 拖放 / 剪贴板 -> RGBA -> PNG bytes
└── constants.rs  # API URL, 配置路径, 窗口尺寸, 请求超时
```

- **UI 线程不得阻塞**: OCR 请求放到 `std::thread` 后台线程, 通过 `std::sync::mpsc` 把 `Result<String>` 传回 UI; 线程结束时调用 `ctx.request_repaint()`.
- OCR 状态只允许一条状态机: `Idle -> Requesting -> Done` 或 `Error`. 图片加载后自动开始 OCR; 新图片覆盖旧图并开始新的请求, 旧请求结果直接丢弃.
- `ocr.rs` 保持纯函数友好: 请求构造和响应解析可单测, 网络发送与解析分离.
- 配置模型用 `serde` 派生, 未知字段容忍; 读取失败时回退默认值并写入结果日志, 不要 panic.
- 配置路径统一放 `constants.rs`. 默认 `C:\Build\mt-config.json`, 沿用 ct 的 `C:\Build` 目录约定.
- API Key 明文存本地 JSON 可接受; UI 用密码框, 日志与结果文本中不要打印 Key 内容.

## UI 行为

- 窗口用垂直中线分为左右两半, 中间画一条分隔线.
- 左上方配置区很小: 只有 API Key 密码框, 输入有效 Key 后自动保存; 不提供保存 / 显示 / 状态控件.
- 左下方图片区占左侧大部分空间, 不提供任何按钮, 外框始终显示边框. 只要 mt 窗口获得焦点, 按 Ctrl+C 就读取剪贴板图片, 不要求先点击图片区; 支持从文件管理器拖放到窗口任意位置(`ctx.input(|i| i.raw.dropped_files)`).
- Ctrl+C 在窗口级别优先读取剪贴板图片; 复制文本使用顶部的 `复制` 按钮.
- 图片加载后自动开始 OCR, 无需手动触发; 新的 Ctrl+C 或拖放会覆盖旧图并重新识别.
- 图片区与右侧结果区都保持 3mm 窗口边距; 预览使用 `ctx.load_texture` 上传为 `TextureHandle`, 按可用区域等比缩放; 大图先缩放到最长边 2048 再上传, 与 API 内部尺寸一致.
- 右半区是 OCR 结果区: 顶部是横跨结果区宽度的 `复制` 按钮, 下方是可编辑多行文本; 文本框尺寸与左侧图片区外框一致, 复制按钮上方 / 复制按钮与文本框之间 / 文本框底部都保持 3mm. 复制用 egui 自带剪贴板接口(如 `ctx.copy_text`), 不要为文本复制引入 arboard.
- 剪贴板为空或只有文本时, 把 `剪贴板中没有图片` 写入结果文本.
- 状态提示与错误都写入结果文本, 日志行以 `[日志]` 开头.
- 用户可见错误用中文: 401 `API Key 无效`, 402 `账户余额不足`, 429 `请求过于频繁, 请稍后重试`, 500 `服务端处理失败`, 网络错误 `网络请求失败, 请检查连接`.
- OCR 成功后写入耗时日志; 请求期间用户手动修改的结果不会被自动覆盖, 可点击 `应用新结果`.

## 平台与陷阱

- 目标平台 Windows 10 / 11, 开发机为 win32. 剪贴板与拖放都要按 Windows 行为验证.
- 本机与 ct 相同, 存在安全软件压缩子进程栈的问题. 建议复制 ct 的 `.cargo/config.toml`:
  - `[env] RUST_MIN_STACK = 134217728`(`force = true`), 防止 rustc / clippy 栈溢出 `0xc00000fd`.
  - `[target.x86_64-pc-windows-gnu] rustflags = ["-C", "link-args=-Wl,--stack,134217728"]`, 防止 GUI 运行时栈溢出.
- winit 要求事件循环在主线程, 不能用 `std::thread::Builder::stack_size` 把 GUI 放到子线程, 会 panic.
- `cargo run` 报 `failed to remove target\debug\mt.exe (os error 5)` = 应用仍在运行, 先结束 `mt` 进程.
- 中文文案源文件必须保持 UTF-8. 所有 UI 字符串直接写中文, 不要转义或 GBK.
- 上传前重新编码 PNG 可绕开剪贴板像素格式差异; `arboard` 返回 RGBA, 但不要假设所有来源一致.
- 不要假设接口一定接受 webp / bmp 等格式; 统一转 PNG 最稳.

## 参考文档

- `D:\Code\ct\AGENTS.md`: 同机 eframe 项目的构建, 状态管理和 Windows 陷阱, 写 GUI 前先读.
- `D:\Code\ct\src\app.rs`, `state.rs`, `config.rs`: eframe 用法和配置持久化参考; 不要照搬设备 / 流水线业务逻辑.
- https://freeocr.ai/zh-sc/platform/docs: API 字段和错误码的最终依据, 接口变更时以官方文档为准.
