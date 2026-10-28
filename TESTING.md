# Crant Peek 验收记录与复现

> 2026-10-28 更新。区分用户实测、自动验证和未覆盖场景，不用编译通过代替系统交互验收。

## 已有实机证据

用户在 v0.1.2 上确认：Windows 除安装包安装外，上一轮核心流程清单均已自行测试。v0.1.3 随后补齐打包验收。v0.1.4 新增官方 DeepL 渠道并修复调用顺序弹层和失焦行为，相关模拟与隔离界面测试见下文；v0.1.5 改善 macOS 取词和对象占位符清理，QQ 连续取词由用户确认通过。旧版用户确认不代替新版全部交互的实机验收。

| 范围 | 证据来源与结果 |
| --- | --- |
| Windows 展开顶栏后拖动窗口 | 用户于 2026-10-15 明确确认已解决 |
| Windows 选区取词、主输入与追问中文输入 | 用户确认已测试 |
| Windows 截图 OCR、图片解读 | 用户确认已测试 |
| Windows 设置修改与重启后保留 | 用户确认已测试 |
| Windows 安装包新装、启动、升级、卸载与配置保留 | Windows 构建机实际执行六项检查通过，见下方自动验收记录 |
| macOS Chrome、VS Code、聚焦搜索取词 | 用户在开发过程中确认已能填入 |
| macOS 原生工具顶栏悬停与拖动保持 | 原生窗口自检通过，检查拖动中几何稳定及松开后的判断 |
| macOS Dock 取消固定后的图标 | 原生窗口反复固定/取消固定自检，像素比对通过 |
| macOS OCR 兼容路径 | 合成中英文图片、并发串行化与白底重试冒烟通过；不表示能识别所有截图 |
| 官网下载 | v0.1.3 三个实际 latest 下载均与 GitHub 正式版 SHA-256 一致；校验记录保存在 dist/验收记录/v0.1.3/官网下载校验.json |

以上用户确认不推断具体未提供的 Windows 版本、软件清单、显示器配置或耗时数据。

## 每次构建的自动检查

[GitHub Actions Rust 工作流](https://github.com/RhedarLiu/CrantPeek/actions/workflows/ci.yml)分别在 macOS / Windows 执行格式、严格 lint、全工作区单元和 HTTP 集成测试、词典生成、release 构建与打包。联网 Google 测试及本地图片路径指定的 OCR 测试不在默认自动运行范围。

图片格式、渠道排序和失败回退、协议流式解析、模型判断回退、配置恢复、语言资源、旧请求丢弃等由对应回归测试覆盖。HTTP 测试使用模拟服务，不据此声称所有真实提供方均已联网实测。

正式 tag 必须与应用版本一致。发布步骤核对两个 ZIP 的完整性、macOS 版本、Windows EXE 和两款字体许可原文，完成后生成 SHA256SUMS.txt，三个程序包一起发布。许可文本比对仅允许 Windows CRLF 与 LF 换行的差异，不忽略其他字符。

## Windows 安装包自动验收

首轮 v0.1.3 安装验收已通过：[运行 38693355118](https://github.com/RhedarLiu/CrantPeek/actions/runs/38693355118)。日志逐项记录六项通过结果，包含 v0.1.2 → v0.1.3 的覆盖升级。此记录对应 Windows 自动验收，不冒充安装向导的人工点击验收。

每次 Windows 构建在打包后执行 `tools/test-windows-installer.ps1`。正式发布依赖这个步骤成功；对应 tag 的 Actions 运行及 `Windows-installer-validation` 日志产物是本版安装验证证据。

该测试在临时 Windows 构建机账户中实际运行 Inno Setup 安装器及卸载器，不仅检查 EXE 是否生成：

1. 新装到包含空格的目录，检查当前用户卸载注册、版本号和开始菜单快捷方式。
2. 将每个安装文件与便携包逐个比对 SHA-256，确认完整词典及两款字体版权/许可文件均安装。
3. 启动安装后的真实程序，运行已有的无窗口诊断，确认原生依赖可以加载、程序正常退出。
4. 卸载，确认程序文件、快捷方式与卸载注册已清理，合成设置和凭据文件原样保留。
5. 安装正式版 v0.1.2，再覆盖安装新包，检查版本升级、文件更新和配置保留。
6. 再次卸载，复查清理和配置保留。

测试只创建合成数据；发现现有个人配置或安装记录即拒绝运行，不读取真实凭据。安装测试失败会阻止对应正式版发布，日志保留用于复查。

手动复现前应使用独立、可丢弃的 Windows 测试账户；当前脚本仅允许在 GitHub Actions Windows 环境运行。

## 快速人工回归

按 README 的实际默认组合键操作；当前未启用双击 Ctrl。

- 选中英文词、句子、代码和报错；分别查询，手动模式不被自动判断覆盖。
- 在主输入和追问中用中文输入法，Enter 发送、Shift+Enter 换行；清空按钮清理输入、结果和会话。
- 截图框选、Esc 取消、复制 OCR、填入输入框、独立截图模式和主动图片解读。
- 确认只上传主动选择的图片区域，关闭截图取消请求，旧结果不覆盖新任务。
- 展开菜单、切换设置页、展开顶栏及拖动；拖动保持展开，固定后切换应用不隐藏。
- 断网时本地词典可用；无效 key、限流和服务错误可读，停止或新查询不会混入旧回答。
- 重启后偏好与渠道仍在；已运行时再次启动不会生成第二个实例。

## 明确未覆盖的范围

- 安装向导逐页人工点击、屏幕阅读器及特殊企业部署策略；自动静默安装不代替这些测试。
- 所有 Windows/macOS 版本、管理员权限应用、全部浏览器和编辑器。
- 多屏负坐标、混合 DPI、全部语言 OCR 的完整实机矩阵。
- 当前版本全场景性能基准、所有第三方 AI/翻译服务及长时间压力测试。
- 发行代码签名、macOS 公证和原生 ARM64 Windows / Intel macOS 包。

平台和签名限制继续在 README、官网及交付说明中披露。

## 界面与本地化验证

应用文案使用 `peek-runtime/locales/zh-CN.ftl` 和 `en.ftl` 的稳定消息 ID，动态内容通过 Fluent 参数传入。界面语言独立于翻译目标语言；新增界面模块需要加入 `peek-runtime/src/i18n.rs` 的资源引用审计清单。

`cargo test -p peek-runtime i18n` 校验中英文键、插值参数、资源引用、回退及异步语言行为。控件 ID、协议字段和模型 ID 不作为界面文案翻译。固定界面字体为 Noto Serif，代码使用 JetBrains Mono，中文和日文使用系统字体回退。

隔离预览只渲染程序自身界面，加载配置时清空 API key，不保存修改、不注册托盘或快捷键、不调用模型。调用顺序预览使用虚构渠道。可用以下命令输出临时图片；它不代替原生交互验收：

```sh
cargo build -p peek-gpui
PEEK_RENDER=/tmp/crantpeek-query.png PEEK_LANGUAGE=en ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-settings.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=settings ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-channel-order.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=channel-order PEEK_CHANNEL_ORDER_SELFTEST=1 ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-channel-popup.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=channel-popup ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-popup-focus.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=popup-focus ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-deepl-official.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=settings PEEK_TAB=2 PEEK_DIALOG=1 PEEK_DEEPL_SELFTEST=1 ./target/debug/peek-gpui
PEEK_RENDER=/tmp/crantpeek-deeplx.png PEEK_LANGUAGE=zh-CN PEEK_PAGE=settings PEEK_TAB=2 PEEK_DIALOG=1 PEEK_DEEPLX_SELFTEST=1 ./target/debug/peek-gpui
```

调用顺序自检点击实际的上移、下移、删除和添加控件，校验首尾边界、共享配置更新和面板持续显示。

`channel-popup` 从主窗口的下拉箭头与渠道图标分别点击打开独立弹层，验证增删排序、主窗口尺寸不变、Esc 关闭及再次打开，覆盖按钮回调中重复访问主窗口状态导致崩溃的回归场景。

`popup-focus` 在未固定且开启失焦隐藏的条件下，对调用顺序与模式菜单模拟激活切换和短暂失焦，校验菜单存在期间不触发主面板自动隐藏；同时覆盖模式选择、Esc、返回主面板及切换外部窗口后的关闭与隐藏判断。

DeepLX 表单自检通过真实类型选择操作验证默认地址为空、缺少地址时拒绝保存、自定义地址空密钥保存和已有服务保留；`cargo test -p peek-network deeplx` 使用本机模拟接口，校验空密钥请求不会携带 Authorization 请求头。

DeepL 官方 API 自检覆盖实际选择、空密钥拒绝、Free/Pro 地址自动选择、编辑时保留原密钥和加入调用顺序。网络模拟测试覆盖官方鉴权、请求结构、错误/超大响应、取消和失败后转用 LLM；没有真实密钥时不代表已完成官方服务端验收。

## macOS 取词可靠性回归（2026-10-28，v0.1.5）

用户于 2026-10-28 在更新并启动本地适配版后，按提示在 QQ 聊天消息中连续测试 Command+E，反馈「连续几次都能取到」。这是本轮 QQ 的实际使用验证；未记录具体次数、QQ/macOS 版本及消息类型，不据此推断所有 QQ 场景均已覆盖。

这一轮依据 Apple 的 [键盘焦点接口](https://developer.apple.com/documentation/applicationservices/kaxfocusedapplicationattribute)、[AX 超时接口](https://developer.apple.com/documentation/applicationservices/1459345-axuielementsetmessagingtimeout)、[Electron 辅助功能文档](https://www.electronjs.org/docs/latest/tutorial/accessibility)，并对照 [Pot Selection](https://github.com/pot-app/Selection/blob/main/src/macos.rs)、[selection-hook](https://github.com/0xfullex/selection-hook/blob/main/src/mac/core/engine.mm) 的实现。也查阅了 [Bob 权限故障说明](https://bobtranslate.com/faq/selection-error.html) 和 [PopClip 的焦点干扰案例](https://forum.popclip.app/t/weird-behaviour-on-copy-when-bettermouse-is-running/2808)。这些资料用于理解平台限制，未整体引入第三方取词库。

取词顺序为：焦点控件的选中文字 → UTF-16 选区/多选区与文本标记 → 有界焦点子节点和祖先 → 应用自身的标准复制菜单 → 定向发送完整的 Command+C 按键序列。取得选区前不展示主面板；用请求编号、键鼠活动及来源应用检查丢弃过期结果。Chromium/Electron 引擎通过应用包的框架和辅助进程识别，涵盖 QQNT，不再只依赖五个应用 ID。

空选区只以当前重试的焦点文本控件为依据；祖先的空范围、先前尝试的空范围和 Cocoa 的 NSNotFound 不阻断复制。真正空选区仍不触发编辑器“复制整行”。辅助功能遍历有整体预算，复制最多等待 1.4 秒；有标准复制菜单时无需等待用户松开 Command。快捷键自动重复不再连续创建取词线程。

复制前保存全部可读取的剪贴板项目及格式，不先清空系统剪贴板。只有请求后的新剪贴板内容才可返回，等待数据就绪和短暂稳定；单次写入可恢复，检测到用户输入或后续写入时保留较新的内容。多次写入无法可靠判断归属，因此允许读取稳定的新文本，但不会冒险覆盖它。剪贴板格式不能完整备份、超过 64MiB、安全输入或密码控件等情况保持保守处理。

自动回归覆盖 NSNotFound、UTF-16 中文和 emoji、延迟/多次写入、空选区、复制命令筛选、完整格式恢复、不覆盖后续复制和失效请求。系统剪贴板测试使用独立命名的测试剪贴板并串行执行，不读取或清空用户剪贴板。完整工作区 131 项通过、2 项原有联网/本地 OCR 测试跳过；严格 clippy 通过。

运行 `bash tools/test-macos-selection.sh` 构建隔离测试程序，再打开它提示的测试窗口（使用与本地应用相同的开发签名，不重置系统权限）。`tools/selection-fixture.swift` 是原生 AppKit 测试窗口，使用合成文本和专用剪贴板；`PEEK_SELECTION_FIXTURE_PID` 诊断入口严格限定 `dev.crant.selection-fixture` 测试应用，可在后台验证辅助功能与复制菜单。每种场景连续读取 5 次：原生文本、仅复制、900ms 延迟复制、多次写入、祖先空范围、临时空范围、真正空选区及复制无响应。此类受控验证不等于真实 QQ 消息列表、全部 Chrome 页面、VS Code 插件、聚焦搜索和远程桌面的全面验收。keyboard 场景需要真实前台键盘焦点，不作为后台测试通过项目。

实际应用诊断可在正式应用启动环境中设置 `PEEK_SELECTION_SELFTEST=1`、`PEEK_SELECTION_LOG`，用 `PEEK_SELECTION_EXPECT` 和 `PEEK_SELECTION_REPEAT` 比较合成选区；日志只记录路径、状态、长度和耗时，不记录选中文字。本次适配需继续在日常应用中观察；不能承诺所有第三方应用 100% 成功。密码框不取词，不开放选区的画布、扫描文档、受保护内容及部分远程桌面需手动截图 OCR 或复制粘贴。

### 选区尾部 OBJ 占位符（2026-10-28）

用户报告从富文本界面选取文字后，末尾混入多个 OBJ 方框。根据 [Unicode 对象占位符说明](https://www.unicode.org/versions/Unicode17.0.0/core-spec/chapter-23/)，U+FFFC 表示文本流中的嵌入对象。现在共用的 `Entry::Selection` 入口移除此字符，再清理首尾空白；macOS 辅助功能、范围/文本标记与复制路径，以及 Windows 取词均使用这个入口。过滤在 UTF-16 选区截取之后执行，避免改变选区位置；手动输入不经过此清理。

针对性回归 68 项通过，1 项原有本地 OCR 测试跳过，严格 clippy 通过。覆盖原问题的连续尾部占位符、开头/中间占位符、只包含对象的选区，以及保留中文、换行、组合重音、emoji ZWJ/肤色/变体选择符、阿拉伯文、真正的替换字符和字面 OBJ 文本。原生 `objects-ax` 与 `objects-copy` 场景各连续 5 次读取通过，使用专用剪贴板；来源字符串保留占位符，返回文字与干净的预期字符串完全一致。
