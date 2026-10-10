<div align="center">

# Crant Peek

**需要时出现，看完即走。**

选中文字，查词、翻译、解释代码，再顺手追问。

macOS · Windows · 原生浮窗 · MIT 开源

[使用方法](#开始使用) · [渠道配置](#选择适合你的渠道) · [源码与打包](#从源码运行与打包)

</div>

[![Crant Peek 使用演示](assets/showcase/preview.gif)](assets/showcase/video.mp4)

<p align="center"><a href="assets/showcase/video.mp4">观看完整演示 · 37 秒 · 含声音</a></p>

阅读文档、浏览网页或看代码时，选中一段内容，用快捷键唤出 Crant Peek。结果就在浮窗里，读完离开；需要继续了解，就在同一个窗口追问。

## 一段内容，多种读法

| 你正在看什么 | Crant Peek 如何帮你 |
| --- | --- |
| 一个英文单词 | 先查离线词典，查看音标、释义和词形；也可以继续问 AI。 |
| 一段外语 | 使用你排好顺序的快速翻译渠道，失败时依次尝试下一个。 |
| 一段源码 | 自动识别为代码解释，理解逻辑和用途。 |
| 报错或堆栈 | 解释问题和可能的处理方向。 |
| 无法选中的文字 | 框选截图，用系统本地 OCR 提取文字后查询。 |

自动模式会判断适合的任务，结果右上角的图标显示判断类型，悬停可看详细信息。你也可以直接指定任务。

回答支持流式显示、停止生成和继续追问。长回答在卡片内滚动；浮窗可以固定，顶部工具栏在鼠标移入时展开。

## 开始使用

1. 打开应用，在托盘菜单中进入浮窗或设置。
2. 在「渠道」添加翻译服务或 AI 服务；在「翻译」排列「快速翻译」的使用顺序，并为「追问」选择一个 LLM。
3. macOS 用户在「权限」中逐项打开对应系统设置，授权辅助功能、输入监控和屏幕录制。
4. 选中文字，按快捷键查询。读取不到选区时，也可以在浮窗里直接输入或粘贴。

| 操作 | macOS 默认快捷键 | Windows 默认快捷键 |
| --- | --- | --- |
| 查询选中文字 | ⌘ E | Alt + E |
| 打开空白输入浮窗 | ⌘ ⇧ A | Alt + Shift + A |
| 截图框选并查询 | ⌘ ⇧ D | Alt + Shift + D |

快捷键可以在设置中修改。输入后按 **Enter** 提交，**Shift + Enter** 换行。

选区读取会受来源应用影响。Windows 截图识别使用系统 OCR，需要安装对应语言包；读取以管理员身份运行的应用时，也可能受系统权限限制。

## 选择适合你的渠道

- **Google 翻译**：无需 API key 或端点地址，使用系统网络与代理设置；网络需要能够连接 Google。
- **DeepLX**：填写你使用的 DeepLX 服务地址。
- **AI 服务**：支持 OpenAI Chat Completions、OpenAI Responses 和 Anthropic Messages 协议，按服务提供方填写地址、API key 和模型。
- **DeepSeek**：支持独立的思考强度设置，翻译时可以关闭思考。
- **决策模型**：可单独配置自动任务判断；服务失败或置信度不足时，回退到本地判断。

「快速翻译」可以混合基础翻译与 LLM 渠道，并调整优先级。「追问」使用单独选定的一个 LLM。每个渠道都可以先测试连接，再开始使用。

## 本地优先

离线词典和截图 OCR 在本机运行，截图不会作为图片上传。使用在线服务时，查询文字会发送到你选择的渠道。

渠道设置和 API key 保存在本机配置文件中；目前 API key 以明文保存，请妥善保护自己的系统账户和配置文件。

词典数据来自 [ECDICT](https://github.com/skywind3000/ECDICT)，采用 MIT 许可。Crant Peek 同样以 [MIT 许可](LICENSE)开源。

## 从源码运行与打包

<details>
<summary>展开运行、词库与打包指引</summary>

需要 Rust 工具链及对应平台的原生构建环境，在项目根目录执行：

```sh
cargo run -p peek-gpui
```

### 准备离线词库（可选）

词库约 30 MB，不随仓库提交。打包前生成后，打包脚本会自动携带词库及许可；没有词库时，在线查询仍可使用。

```sh
mkdir -p local-assets
git clone --depth 1 https://github.com/skywind3000/ECDICT.git local-assets/ecdict
cargo run --release -p peek-dict --bin build-dict -- local-assets/ecdict/ecdict.csv local-assets/ecdict.pkd
```

### macOS

首次本地打包可建立稳定的开发签名，减少重新编译后重复授权：

```sh
bash tools/dev-signing-identity.sh
bash tools/package-macos.sh
open "dist/Crant Peek.app"
```

签名初始化会创建并导入本地开发证书。之后通常只需执行打包和打开应用两步；也可通过 `SIGN_IDENTITY` 指定自己的签名身份。本地开发签名不等同于发行签名，应用尚未公证。

### Windows

在 Windows PowerShell 中执行：

```powershell
./tools/package-windows.ps1
```

输出 `dist/Crant-Peek-Windows.zip`，解压后运行 `CrantPeek.exe`。如果打包前准备了 `local-assets/ecdict.pkd`，词库会一同打包。

</details>
