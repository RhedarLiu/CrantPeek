# Crant Peek

需要时出现，看完即走。Windows / macOS 上的 Rust 浮窗查词与 AI 阅读助手，非 Web UI，MIT 开源。

**开发中的原型，不是完整产品。** 下面的状态区分“已验证”“只编译过”“还没做”，请以此为准。

## 当前入口

- 选区查询：macOS 默认 ⌘E，Windows 默认 Alt+E，可在设置修改。读取失败时打开输入浮窗，不使用旧剪贴板。
- 空白浮窗：macOS ⌘⇧A / Windows Alt⇧A。
- 截图框选、本地 OCR、就地翻译：macOS ⌘⇧D / Windows Alt⇧D，选择鼠标所在显示器。
- 托盘提供打开、截图和退出。双击 Ctrl 钩子代码仍保留，但当前主程序不启动它。

## 当前状态（2026-10-10 代码审查）

| 功能 | 实现与边界 |
| --- | --- |
| 快速翻译与追问 | 可排序的基础翻译/LLM 渠道列表，失败依次回退；追问单独选择一个 LLM。两平台共用 UI 和请求逻辑。 |
| 渠道 | Chat Completions、Responses、Anthropic、DeepSeek（思考强度）、DeepLX、Google 免费翻译、决策模型；有连接测试。Google 请求使用固定地址。 |
| 回答与会话 | 流式、停止、追问、复制、受限高度滚动、历史裁剪；已有本地 HTTP 集成测试。 |
| 浮窗 | 共享卡片布局、顶部悬停工具栏、设置、固定。macOS 原生窗口处理已开发验证；Windows 显隐/移动和输入法由 Win32/GPUI 实现，尚未实机验证。 |
| 选区 | macOS AX 选中文字、范围、祖先及文本标记回退；Windows UI Automation TextPattern。读取发生在浮窗抢焦点之前；兼容性不等于所有应用都可取词。 |
| 截图与 OCR | xcap 获取显示器、内存框选裁剪；macOS Vision / Windows 系统 OCR。Windows 需要适用语言包，多屏及混合 DPI 尚需实机验收。 |
| 离线词典 | ECDICT 约 40 万词条，词形和音标；词库需单独构建，打包脚本会携带已有本地词库及许可。 |
| 决策 | 可配置独立渠道，失败/低置信度回退本地任务判断。任务选择和基础翻译/LLM 的能力分流共用代码。 |
| 图片理解 | 网络层已有三协议图片回答和 Clef 图片判断，但当前 GPUI 界面没有接入入口及上传流程；截图目前走 OCR 后文字查询。 |
| 权限 | macOS 三项状态和逐项直达设置；Windows 显示高权限应用及 OCR 语言包限制说明，不套用 macOS 的授权状态。 |
| 凭据 | 当前渠道 API key 存在用户配置 JSON 中；旧钥匙串接口保留，当前渠道请求未使用。公开发行前应重新完成安全存储设计。 |
| 设置覆盖 | 已接通字体、渠道、快捷键、翻译渠道排序、截图复制后关闭、权限；主题/缩放、界面语言选择、目标语种和翻译风格尚无完整设置入口。首次使用引导未接通。 |
| 打包 | macOS 本地稳定开发证书签名、CI ad-hoc；Windows 提供便携 ZIP 打包脚本。公开发行签名/公证、安装和更新尚未完成。 |

macOS 已经过本地开发及用户交互验证，Windows 当前只做代码对照和可用范围内的交叉检查；不声称 Windows 实机可用。产品规划和用户体验说明包含尚未交付的目标，不能作为实现清单。

## 运行

```sh
cargo run -p peek-gpui
```

### 离线词典

词库不随仓库提交（约 30 MB）。数据来自 [ECDICT](https://github.com/skywind3000/ECDICT)（MIT）：

```sh
mkdir -p local-assets
git clone --depth 1 https://github.com/skywind3000/ECDICT.git local-assets/ecdict
cargo run --release -p peek-dict --bin build-dict -- local-assets/ecdict/ecdict.csv local-assets/ecdict.pkd
```

应用依次查找配置目录、可执行文件旁、macOS Resources 和 `local-assets` 下的 `ecdict.pkd`，找不到则不显示词典卡片，其余功能不受影响。

### 打包

macOS：`bash tools/package-macos.sh`。本地默认使用 Crant Peek Dev 稳定开发签名；尚未公证。

Windows PowerShell：`./tools/package-windows.ps1`，输出 `dist/Crant-Peek-Windows.zip`。解压后运行 CrantPeek.exe；若打包前存在 `local-assets/ecdict.pkd`，会随包携带。CI 不下载词库，缺少词库时仍需按上述步骤构建。

## 开发验证

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p peek-runtime --target x86_64-pc-windows-msvc -- -D warnings   # Windows 交叉检查
```

## 文档

- [用户体验说明](USER_GUIDE.md)
- [产品规划](PRODUCT_PLAN.md)
- [界面多语言与视觉规范](I18N.md)
- [实机验收清单](TESTING.md)
- [性能验证记录](PERFORMANCE.md)

编译通过不等于系统级功能可用。Windows 钩子、权限、多屏截图等需要 Windows 实机验证；macOS 同样需要权限与应用兼容性测试。

仓库中不存放 API key、私有查询或截图。
