快捷查询，即用即走。

## v0.1.3 更新

- 更新使用指南、测试记录与交付范围，统一为当前默认快捷键、设置、截图和图片解读行为。
- 补齐 Noto Serif、JetBrains Mono 的完整版权与 OFL 许可，随 macOS、Windows 便携包和安装包分发。
- Windows 发布前强制执行实际新装、覆盖升级、卸载、快捷方式及配置保留自动验收，并校验安装文件与便携包一致。
- 记录用户已完成的 Windows 核心功能实测；不改 v0.1.2 已验收的业务交互。

## 下载与使用

提供 macOS Apple Silicon、Windows x64 便携版与安装包。ZIP 内包含完整离线词典及许可证。Windows 便携版请解压后运行 `CrantPeek.exe`，保留同目录资源文件；未提供 Windows ARM64 原生包。`SHA256SUMS.txt` 可用于验证三个下载包的完整性。

macOS CI 包尚无发行签名与公证，Windows 包尚无发行代码签名；Windows 核心功能已由用户实测，安装包通过 Windows 构建机的静默安装、升级与卸载验收；安装向导逐页交互未另做人工验收。首次打开可能受到系统限制，请确认下载来源后按系统提示操作。Windows OCR 需要对应的系统语言包。

离线词典与系统 OCR 在本机运行。在线翻译和 AI 服务需要自行配置，查询文字会发送至选定服务。图片解读仅在手动操作时上传选区，须在渠道设置中启用图片输入能力并加入调用顺序；图片解读后的追问默认只发送文字。

[官网与下载](https://crantpeek.eno.ink) · [GitHub 源码与反馈](https://github.com/RhedarLiu/CrantPeek)
