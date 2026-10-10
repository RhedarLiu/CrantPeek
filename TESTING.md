# Crant Peek 实机验收清单

> 开发中；未勾选的项目尚未取得实机验证证据。测试不能仅以编译通过代替。

## 构建与启动

- [ ] macOS：`bash tools/package-macos.sh`，双击 app 启动，菜单栏图标正常；不出现 Dock 主应用入口。
- [ ] Windows：从 GitHub Actions 下载 exe，正常启动；托盘图标和退出菜单可用。
- [ ] 首次启动欢迎页可读，保存后下次不重复欢迎。
- [ ] 设置重复保存及重新启动后读取成功；当前渠道 key 保存在用户配置 JSON 中；系统安全存储迁移仍待完成。

macOS 本地打包默认使用稳定开发证书，CI 为 ad-hoc，**没有公证**。Windows 使用便携 ZIP 包。两个平台的打包脚本都携带已有本地词库；CI 默认不下载词库，需按 README 构建。

## 三个入口（必须互不混淆）

当前主程序使用组合键取词，双击 Ctrl 钩子未启动；以下双击 Ctrl 项目属于旧规划，需在重新启用时验收。当前默认选区入口为 macOS ⌘E / Windows Alt+E，读取失败打开输入窗。

- [ ] 无选区双击 Ctrl：不弹窗，不产生模型请求。
- [ ] 选中词双击 Ctrl：准确读取当前选区，不读取旧剪贴板。
- [ ] Ctrl+C / Ctrl+其他键，不误触发双击 Ctrl。
- [ ] macOS ⌘⇧A / Windows Alt⇧A：空白输入，不带入选区或剪贴板。
- [ ] macOS ⌘⇧D / Windows Alt⇧D：截图选区。
- [ ] 快捷键冲突可见；修改后重启生效。

分别测试系统文本编辑器、浏览器、代码编辑器、PDF 阅读器；记录每个应用成功/失败，不笼统宣称全系统取词兼容。

## 浮窗与会话

- [ ] Esc、失焦、关闭按钮隐藏；托盘/快捷键可重新打开；固定后失焦保持。
- [ ] 长文本、中文、日文、代码、字体缩放不截断；设置可滚动到保存按钮。
- [ ] 标题拖动、⌘/Ctrl+Enter 查询、追问 Enter 发送、复制结果正常。
- [ ] 手动任务不被智能覆盖；恢复智能不丢弃当前截图。
- [ ] 停止/失败后重试不混入未完成消息；关闭后迟到结果不重新弹窗。

## 离线与截图

- [ ] 断网、未配模型时 stream / given 能显示词典结果；未收录词不崩溃。
- [ ] 截图不包含 Peek 窗口；选区与结果对应，Esc 不提交。
- [ ] 主屏、副屏、负坐标显示器、Retina/混合 DPI 坐标准确。
- [ ] 英文/中文/日文 OCR 识字效果；允许检查并编辑识别原文。
- [ ] 新入口或关闭丢弃旧 OCR 响应。
- [ ] 默认不上传图；点击图片解读或 Clef 判断前能看到服务地址。

## 模型（使用自己的凭据，不向开发日志提交 key）

- [ ] Chat Completions、Responses、Anthropic 文本流式分别实际调用。
- [ ] Jev/Clef 文本路由；缺 key、超时、低置信度回退不阻塞回答。
- [ ] 图片回答三协议分别测试；Clef 图片决策测试。
- [ ] 限流、无效 key、断网、服务错误可读且可恢复。

## 性能与隐私

- [ ] 记录启动、待机、词典加载后内存及 CPU；窗口打开/关闭后的 CPU 回落。
- [ ] 分开测窗口唤起、OCR、决策、首段生成耗时，不把网络延迟归于 UI。
- [ ] 退出后无后台进程；没有原文、截图、key 进入普通日志或 git。

## 当前自动验证

本地单元/HTTP 集成测试、macOS 和 Windows 目标严格 lint；macOS Vision 白图及合成文字图片运行冒烟通过：识别结果为 `Crant Peek OCR test 123`。未采集用户屏幕。此列表不构成视觉或系统交互验收。

可复用的本地 OCR 验证：截图翻译（托盘「截图」或 Command+Shift+D）对
`assets/ocr-test.png` 框选即可，日志在 `PEEK_SELECTION_LOG` 指定路径，识别成功时打印
`[snip] ocr ok: N chars`。

Windows 需要安装支持英语的 OCR 语言包；尚未在 Windows 执行上述命令。

## 2026-10-10 完成度与平台代码审查

以当前 GPUI 主程序的实际调用为准，网络层、旧钩子或配置字段存在不等于界面已交付。

| macOS 已开发功能 | Windows 对应处理 | 还需注意 |
| --- | --- | --- |
| 全局选区、空白、截图快捷键 | 同一 global-hotkey 注册及动作分发，默认 Alt 修饰键 | 注册失败仅写日志，设置未显示冲突状态；修改需重启 |
| 读取当前文字选区 | UI Automation GetFocusedElement / TextPattern / GetSelection | 回退路径少于 macOS；不同应用兼容性仍待验证 |
| 显隐、弹出置顶、顶部扩展、卡片/结果高度 | Win32 ShowWindow / SetWindowPos，GPUI PopUp 置顶无标题栏；UI 共用 | 顶部扩展仍是先移动再 resize，尚无 Windows 同帧行为证据 |
| 主输入和追问中文输入法 | GPUI Windows 文本输入处理；macOS 额外刷新 AppKit inputContext | 无 Windows 实机输入法测试，不将 macOS 专用补丁认定为功能缺失 |
| 截图框选与就地翻译 | xcap + 同一 Snip UI/裁剪/翻译流程 | GPUI Windows PopUp 已有 WS_EX_TOPMOST / WS_EX_TOOLWINDOW，无需重复加遮罩置顶代码 |
| 本地 OCR | Windows.Media.Ocr / SoftwareBitmap / WinRT 初始化 | 依赖系统 OCR 语言包 |
| 权限逐项跳转 | Windows 使用平台限制说明，系统隐私设置接口保留 | 没有对应 macOS TCC 的三项授权检测；不显示虚假的已授权标记 |
| 字体、设置、全部渠道、排序/回退、DeepSeek 强度、追问、token 设置、任务判断 | 相同业务和 UI 代码；字体有 Windows 系统回退 | 真实服务及长会话需补实测 |
| 单实例、配置存取、词典定位 | Windows 文件锁错误处理、原子替换、配置目录、exe 旁词库查找 | 第二次启动只退出，未唤起已有实例 |
| app 打包带资源 | 新增 Windows PowerShell ZIP 打包及 CI 调用 | PowerShell 打包需 Windows CI/本机执行确认；词库并非自动下载 |

本轮修复：Google 连接测试遗漏、渠道类型变化后决策引用失效、编辑重置渠道能力、渠道保存失败无反馈、截图启动失败只有日志、网络错误暴露 URL 查询原文、未配渠道时离线词典被错误阻断，以及 Windows 发布版控制台窗口与打包入口缺口。新增回归测试覆盖渠道角色修复和错误 URL 脱敏；CI 纳入 GUI 几何单元测试。

### 距离可交付首版的剩余工作

1. 优先补齐设置闭环：目标语种/方向及风格、界面语言、主题/缩放；快捷键注册失败应在界面可见。首次引导、托盘设置及暂停入口没有接通。
2. 图片理解：网络层有实现但 UI 没有图片任务、vision 配置或明确上传确认入口；不能算已交付的截图多模态功能。
3. 故障恢复：配置读取失败当前回退默认值，后续保存可能覆盖原配置；需保留坏文件并阻止无提示覆盖。截图遮罩关闭后尚未直接取消其网络请求。重复取词线程的晚到结果没有统一入口代次检查。
4. 生命周期：空白入口实际保留当前内容；失焦/关闭隐藏不会停止网络请求。应确定是否需要严格“打开空白即新建”和“关闭即取消”，再与用户当前使用习惯统一。
5. 安全及交付：当前渠道 key 明文存用户配置；完善安全存储/迁移与删除。发布签名、公证、安装/升级、词库获取和许可展示尚未闭环。
6. 验收及性能：真实服务错误恢复，Windows 输入法/显示器/DPI/选区和截图，全平台隐藏待机与峰值资源实测。现有性能记录不是当前版本的完整基准。

功能审查完成不等于这些项目已实现；不把可选扩展（收藏、发音、云同步等）算作当前必须完成的核心缺口。

### 本轮验证结果

- macOS 全工作区 80 项单元/HTTP 集成测试通过，严格 clippy、格式和差异检查通过。
- Windows runtime（包含选区、截图/OCR、配置、权限）目标严格 clippy 通过；修复了 macOS 诊断函数在 Windows 的 dead-code 错误。
- 尝试完整 Windows GUI 交叉检查，在第三方 GPUI 资源构建阶段因本机缺少 `llvm-rc` 中止；不宣称 GUI 目标已通过。Windows 原生 CI 已保留完整构建、测试和新增打包步骤，本轮未执行远端 CI。
- 隔离预览 `PEEK_RENDER=/tmp/peek-offline-audit.png PEEK_OFFLINE_SELFTEST=1 target/release/peek-gpui` 通过：使用空渠道配置，词典命中，无网络任务，无缺渠道错误。测试要求本地已有词库。
- release 通过稳定开发证书打包及签名验证，并按固定的打包、关闭旧实例、open 启动、进程确认流程重启。

Windows 窗口/选区的系统边界可参考 [SetWindowPos 文档](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos) 和 [高权限应用的 UI Automation 限制](https://learn.microsoft.com/en-us/power-automate/desktop-flows/how-to/enable-ui-access)。本项目未开启 UIAccess 或自动提高权限。

## 自动决策结果图标

- 自动模式在回答框右上角显示实际采用任务的图标；只有词典结果时显示在词典框右上角。手动模式不显示。
- 悬停显示任务、本地规则或决策渠道/模型、模型返回的置信度；低置信度/服务不可用时明确说明本地回退。本地规则不伪造置信度。
- 追问保留原查询的自动决策任务和信息；新的查询更新，过期决策由入口代次丢弃。决策请求期间切换手动任务时，手动选择优先。
- 回归测试覆盖模型成功、低置信度、失败回退及中英文详情；隔离预览可设置 `PEEK_DECISION_PREVIEW=1` 检查回答/对话卡片的图标位置，不调用真实服务。

## Google 翻译系统代理回归

2026-10-10：已开启系统 HTTP/HTTPS/SOCKS 代理时，Google 固定测试词直连超时，通过系统 HTTPS 代理返回 HTTP 200。原项目关闭 reqwest 默认功能后没有显式启用 `system-proxy`，导致 GUI 进程不使用已配置的系统代理。本轮显式开启 `system-proxy` 和 `socks`；同一请求客户端覆盖 macOS/Windows。

可选联网测试：`cargo test -p peek-network google_uses_the_desktop_transport -- --ignored`。它仅发送固定测试词 hello；默认 CI 不运行，以免受 Google 网络可达性影响。无需额外填写 Google 地址或 key，不修改系统代理配置。

本机 DeepLX/LLM 地址（localhost、IPv4/IPv6 回环）使用不经过代理的客户端，防止启用系统代理后本机服务也被转发。回归测试覆盖回环识别和基础翻译/LLM 请求。
