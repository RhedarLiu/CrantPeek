# Crant Peek — 简体中文
# 规则：界面不写死任何文本；新增键必须同时加入所有语言文件（测试会检查）。

app-name = Crant Peek

## 顶栏
header-pin = 固定
header-unpin = 取消固定
header-settings = 设置
header-close = 关闭
action-close = 关闭
action-pin = 固定为正式窗口
action-unpin = 恢复即用即走
header-back = 返回
header-paused = 快捷入口已暂停

## 查询页
query-placeholder = 输入或粘贴内容…
query-run = 查询
query-stop = 停止
query-copy = 复制结果
query-copy-source = 复制原文
query-clear = 清空
query-restore-smart = 恢复智能
query-target = 目标语言
query-target-auto = 自动
query-target-custom = 自定义语言
query-followup-placeholder = 继续追问…
query-send = 发送
query-empty-title = 需要时出现，看完即走
query-empty-hint = 选中文字后双击 Ctrl，或在上方输入内容。{ $submit } 查询。
query-section-answer = AI 回答

## 任务
task-translate = 翻译
task-define = 查词
task-explain-code = 代码解释
task-explain-error = 报错分析
task-explain = 解释

## 词典
dict-badge = 离线词典
dict-lemma = 原形：{ $lemma }
form-past = 过去式
form-past-participle = 过去分词
form-present-participle = 现在分词
form-third-person = 第三人称单数
form-comparative = 比较级
form-superlative = 最高级
form-plural = 复数

## 截图
snip-copy-text = 复制文本
snip-to-input = 填入输入框
snip-copied = 已复制原文
snip-hint = 拖动框选 · Esc 取消
snip-image-title = 截图选区
snip-explain = 图片解读
snip-decide = Clef 判断
snip-discard = 丢弃图片
snip-explain-default = 请解释截图中的内容。
snip-decide-default = 请判断截图需要什么阅读辅助。
snip-decide-target = Clef 仅发送到决策服务：{ $endpoint }
snip-explain-target = 图片解读发送到 { $endpoint }；追问默认只发文字
snip-cancel-ocr = 取消识字

## 路由
route-local = 本地判断 · 手动模式
route-deciding = 正在判断任务…
route-decided = { $model } · 置信度 { $confidence }
route-uncertain = 决策不确定 · 使用本地判断
route-unavailable = 决策不可用 · 使用本地判断

## 状态
status-key-missing = 该渠道缺少 API key，请在设置里补上
status-channel-missing = 还没有可用渠道，请先在设置里添加
status-channel-incomplete = 渠道信息不完整：名称和端点必填
status-generating = 生成中…
status-done = 完成
status-done-trimmed = 完成 · 已释放较早追问，保留初始问题与最近对话
status-stopped = 已停止
status-copied = 已复制
status-copied-source = 已复制原文
status-cleared = 已清空当前会话
status-saved = 已保存
status-saved-restart = 已保存，快捷键修改需重启生效
status-input-too-long = 输入超过 64 KiB，请缩短选区或分段查询
status-output-too-long = 回答超过 512 KiB，已停止；请缩小问题范围
status-model-missing = 请先在设置中填写模型
status-worker-crashed = 网络任务异常退出，请重试
status-ocr-running = 本地识字中…
status-ocr-cancelled = 已取消识字
status-ocr-empty = 未识别到文字，请重选区域
status-ocr-failed = OCR 失败：{ $error }
status-ocr-local-only = 本地识字完成 · 可复制原文或手动查询，尚未发送内容到模型
status-capture-failed = 截图失败，请检查屏幕录制权限：{ $error }
status-paused = 快捷入口已暂停（托盘可恢复）
status-resumed = 快捷入口已恢复
status-config-error = 配置读取失败：{ $error }
status-desktop-error = 桌面集成失败：{ $error }

## 错误（来自核心/网络层的错误码）
error-key-missing = 尚未配置 API key，请打开设置
error-keychain-open = 无法打开系统凭据存储
error-keychain-save = 无法保存凭据到系统存储
error-network = 网络请求失败：{ $detail }
error-http = 服务返回 HTTP { $code }
error-invalid-response = 服务响应无效：{ $detail }
error-cancelled = 请求已取消
error-truncated = 回答达到 token 上限，内容可能不完整；可提高上限后重试
error-content-filter = 回答被服务内容过滤中止
error-service = 服务报告错误
error-hotkey-blank = 空白浮窗快捷键无效：{ $detail }
error-hotkey-screenshot = 截图快捷键无效：{ $detail }
error-hotkey-duplicate = 两个入口不能使用相同快捷键
error-config-version = 不支持的配置版本
error-config-double-ctrl = 双击 Ctrl 间隔需在 150–800 ms
error-config-shortcut-same = 两个入口快捷键不能相同
error-config-language = 目标语言不能为空
error-config-appearance = 外观设置无效
error-config-style = 翻译风格无效
error-config-tokens = 回答 token 上限需在 128–16384
error-config-endpoint = 地址无效：需 HTTPS，或仅本机回环使用 HTTP，且不能包含账号密码
error-config-decision-model = 决策模型不能为空
error-config-decision-timeout = 决策超时需在 100–10000 ms
error-config-decision-threshold = 决策阈值无效

## 设置
settings-tab-appearance = 外观
settings-tab-translation = 翻译
settings-tab-permissions = 权限
settings-title = 设置
settings-save = 保存
settings-cancel = 取消
settings-section-general = 通用
settings-section-appearance = 外观
settings-section-shortcuts = 快捷键
settings-section-answer = 回答模型
settings-section-decision = 决策模型
settings-section-translation = 翻译
settings-section-privacy = 隐私与权限
settings-language = 界面语言
settings-language-system = 跟随系统
settings-theme = 主题
settings-theme-system = 跟随系统
settings-theme-light = 浅色
settings-theme-dark = 深色
settings-zoom = 界面缩放
channel-kind-chat = OpenAI Chat Completions
channel-kind-responses = OpenAI Responses
channel-kind-anthropic = Anthropic Messages
channel-kind-deeplx = DeepLX
channels-add-title = 添加渠道
channels-edit-title = 编辑渠道
channels-edit = 编辑
channels-save = 保存
channels-cancel = 取消
channels-key-keep = 留空则保留原密钥
channels-title = 渠道
channels-empty = 还没有渠道，先添加一个
channels-add = 添加
channels-remove = 删除
channels-kind = 类型
channels-name = 名称
channels-endpoint = 端点地址
channels-key = API key
channels-model = 模型 ID
channels-usage = 使用位置
channels-basic = 基础翻译
channels-ai = AI 任务
channels-none = 未选择
channels-basic-pick = 基础翻译渠道
error-config-channel-model = 渠道缺少模型 ID
error-config-channel-missing = 选择的渠道不存在或不能用于该位置
error-translation-size = 翻译响应超过大小限制
settings-font-set = 字体方案
settings-shortcut-blank = 空白浮窗
settings-shortcut-screenshot = 截图
settings-double-ctrl = 双击 Ctrl 间隔（ms）
settings-shortcut-hint = 示例：Super+Shift+A（macOS 为 Command）、Alt+Shift+A（Windows）。修改后需重启生效。
settings-protocol = 协议
settings-base-url = Base URL
settings-base-url-hint = 通常包含 /v1
settings-model = 模型
settings-max-tokens = 回答 token 上限
settings-vision = 支持图片输入（仅在手动图片解读时上传选区）
settings-api-key = API key
settings-api-key-hint = 留空则保留已保存的凭据
settings-test = 测试连接
settings-test-cost = 会产生少量 API 用量
settings-test-cancel = 取消测试
settings-test-running = 测试中…
settings-test-ok = 连接成功 · 流式响应正常
settings-test-failed = 连接失败：{ $error }
settings-test-timeout = 连接测试超时
settings-test-crashed = 连接测试任务失败
settings-test-cancelled = 已取消连接测试
settings-decision-enable = 启用专用决策服务
settings-decision-endpoint = Endpoint
settings-decision-endpoint-hint = 完整 URL：TypeSafe 或 Cloudflare 模型路由
settings-decision-model = 决策模型
settings-decision-key = 决策 API key / Token
settings-decision-threshold = 采用阈值
settings-decision-timeout = 超时（ms）
settings-decision-note = 只发送当前查询文本；不可用时回退本地判断。
settings-default-target = 默认目标语言
settings-chinese-target = 中文翻译为
settings-style = 翻译风格
settings-style-natural = 自然表达
settings-style-literal = 直译
settings-style-technical = 技术文档
settings-smart-mode = 智能判断任务
settings-ocr-auto = 截图识字后自动查询
settings-ocr-auto-hint = 关闭后只在本地识字，点击查询或图片按钮才调用外部服务。
settings-hide-on-blur = 失焦时自动隐藏
settings-snip-close-on-copy = 复制文本后关闭截图窗口
settings-permissions = 系统权限
settings-permission-granted = 已授权
settings-permission-denied = 未授权
settings-permission-accessibility = 辅助功能（读取选区）
settings-permission-input = 输入监控（双击 Ctrl）
settings-permission-screen = 屏幕录制（截图）
settings-permission-hint = 双击 Ctrl 只读取选区，无选区不弹窗。应用不支持读取选区时，请用空白浮窗粘贴或截图。
settings-permission-windows = 无法读取高权限应用或安全输入；系统 OCR 需已安装语言包。
settings-permission-restart = macOS 修改权限后可能需要重启 Peek。
settings-open-privacy = 打开系统隐私设置
settings-open-privacy-failed = 无法打开系统设置

## 欢迎
welcome-title = 欢迎使用 Crant Peek
welcome-selection = 双击 Ctrl：只查询选区；没有选区不弹窗。
welcome-shortcuts = { $blank }：空白输入 · { $screenshot }：截图
welcome-dismiss = Esc 或失焦收起；固定后可对照阅读；托盘菜单可重新打开或退出。
welcome-privacy = 词典与 OCR 在本地运行。AI 查询文本会发往你配置的服务；不会自动上传剪贴板或整屏。
welcome-start = 开始使用

## 托盘
tray-tooltip = Crant Peek
tray-open = 打开空白 Peek
tray-screenshot = 截图
tray-settings = 设置
tray-pause = 暂停 / 恢复快捷入口
tray-quit = 退出

language-zh = 简体中文
language-en = English
language-ja = 日文

error-image-size = 图片超过上传大小限制

error-image-base64 = 图片编码无效

error-image-header = PNG 图片头无效

error-image-pixels = 图片超过像素上限

error-image-disabled = 回答服务未启用图片能力

error-message-missing = 请求缺少消息

error-image-message = 图片请求需要用户消息

error-sse-buffer = 流式缓冲超过上限

error-sse-encoding = 流式响应编码无效

error-sse-event = 单次流式事件超过上限

error-sse-json = 流式数据格式无效

error-response-incomplete = 回答失败或未完成

error-sse-type = 服务未返回流式响应

error-stream-ended = 流式响应意外结束，内容可能不完整

error-output-size = 回答超过大小限制

error-decision-size = 决策响应超过大小限制

error-decision-json = 决策响应格式无效

error-decision-schema = 决策响应结构无效

error-decision-image-model = 图片决策需使用 Clef 或 Clef Flash

error-decision-probabilities = 决策概率无效

error-decision-missing-task = 决策概率中缺少选中任务

error-decision-unknown-task = 决策包含未知任务

error-decision-inconsistent = 决策选项与概率不一致

error-config-directory = 无法定位有效的配置目录。
error-ocr-language-pack = 请在 Windows 设置中安装 OCR 语言包。
error-no-display = 没有可用显示器。

preview-answer = 最好的工具，尊重你的注意力。
    
    好的阅读助手帮你理解眼前的内容，然后安静退场。

protocol-chat = OpenAI · Chat Completions
protocol-responses = OpenAI · Responses
protocol-anthropic = Anthropic · Messages

preview-source = The best tools respect your attention.
error-instance = 无法获取应用实例锁：{ $detail }
