快捷查询，即用即走。

## v0.1.2 更新

- 修复 Windows 展开工具顶栏后无法拖动窗口的问题：可拖动左侧标题和空白区域，按钮保持正常点击。
- 改善 macOS 截图 OCR 的稳定性：统一图片格式，并对系统图像读取的暂时性错误进行兼容重试。
- 修复 macOS 取消固定、隐藏窗口时 Dock 图标短暂变为系统占位图的问题。
- 更新 README 和交付材料中的演示视频。

## 下载与使用

提供 macOS Apple Silicon、Windows x64 便携版与安装包。ZIP 内包含完整离线词典及许可证。Windows 便携版请解压后运行 `CrantPeek.exe`，保留同目录资源文件；未提供 Windows ARM64 原生包。`SHA256SUMS.txt` 可用于验证三个下载包的完整性。

macOS CI 包尚无发行签名与公证，Windows 包尚无发行代码签名；Windows 安装包及新功能的实机验收仍未完成。首次打开可能受到系统限制，请确认下载来源后按系统提示操作。Windows OCR 需要对应的系统语言包。

离线词典与系统 OCR 在本机运行。在线翻译和 AI 服务需要自行配置，查询文字会发送至选定服务。图片解读仅在手动操作时上传选区，须在渠道设置中启用图片输入能力并加入调用顺序；图片解读后的追问默认只发送文字。

[官网与下载](https://crantpeek.eno.ink) · [GitHub 源码与反馈](https://github.com/RhedarLiu/CrantPeek)
