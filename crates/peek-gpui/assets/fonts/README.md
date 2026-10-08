# 内置字体 · Bundled fonts

这些字体随 Crant Peek 一起分发，目的是让 macOS 与 Windows 上的界面**完全一致**，不依赖各平台的系统字体。

## 清单

| 文件 | family | 体积 | 用途 | 授权 |
|---|---|---|---|---|
| `Inter.ttf` | Inter | 0.9 MB | 拉丁 / UI（默认方案） | SIL OFL 1.1 |
| `NotoSansSC.ttf` | Noto Sans SC | 17.8 MB | 简体中文（默认方案） | SIL OFL 1.1 |
| `NotoSansJP.ttf` | Noto Sans JP | 9.6 MB | 日文（默认方案） | SIL OFL 1.1 |
| `JetBrainsMono.ttf` | JetBrains Mono | 0.2 MB | 代码 / 报错 | SIL OFL 1.1 |
| `NotoSerif.ttf` | Noto Serif | 1.9 MB | 拉丁衬线（「衬线」主题） | SIL OFL 1.1 |

默认方案随包体积约 **28.5 MB**。

**均为可变字体**（文件名中的 `[wght]`），一个文件覆盖全字重，粗细由 `font_weight` 控制。

## 授权

全部为 **SIL Open Font License 1.1**，允许自由使用、修改与**随软件分发**，要求：

1. 随附 OFL 许可证全文；
2. 不得单独出售字体本身；
3. 修改后的字体不得使用保留字体名（Reserved Font Name）。

> ⚠️ **发布前必办**：仓库目前**尚未包含 OFL 许可证全文**。首次发布前必须把每个字体上游的 `OFL.txt` 放到本目录（或合并为一份 `OFL.txt` 并在上方表格中指明对应关系）。这是 OFL 的硬性要求，不是可选项。

上游与许可证地址：

- Inter — https://github.com/rsms/inter
- Noto Sans SC / Noto Sans JP / Noto Serif — https://github.com/notofonts / https://fonts.google.com/noto
- JetBrains Mono — https://github.com/JetBrains/JetBrainsMono

## 为什么不按语种拆分一个 CJK 文件

gpui 的 `Font` 只有 `family` / `features` / `fallbacks` / `weight` / `style`，**没有 language / script 字段**，因此无法为单个文本 run 启用 OpenType `locl` 来按语言挑选区域字形。

而中日韩存在 **Han 统一（Han unification）**：同一 Unicode 码位在中文与日文里字形不同（`骨`、`直`、`海`、`画`、`今` 等）。若只为中文打包一套字体，日文译文里的汉字会显示成中文字形。

**所以中文字体与日文字体必须分别打包**，由调用方按文本语种选择 `font_family`——见 `crates/peek-gpui/src/fonts.rs` 的 `FontSet`。语种信息本来就有：翻译目标语言已知，源语种由 `peek-core` 检测。

## 一个必须避开的陷阱

市面上常见的"精简版 / 压缩版"中文字体每字重只有 0.8–1.8 MB。经查证，其字符集通常是 **2500 常用字 + 1000 次常用字 ≈ 3500 字**。

Crant Peek 需要渲染任意用户选中的文本、ECDICT 罕见词条与 AI 回答，**必然超出该字符集 → 显示为豆腐块**。

**因此必须使用完整覆盖的 CJK 字体**，这就是 Noto Sans SC 需要 17.8 MB 的原因，没有捷径可走。

## 已知缺陷

「衬线」主题的 CJK 目前**未打包**：`Noto Serif SC` 的多个镜像反复在 10.5 MB 处截断（完整应约 20 MB），gpui 加载时报 `Could not load an embedded font`。因此该主题的中文/日文暂取系统字体（macOS `Songti SC` / `Hiragino Mincho ProN`，Windows `SimSun` / `Yu Mincho`），**跨平台不一致**。

修复方式：拿到完整的思源宋体 SC 后替换，并把 `fonts.rs` 中 `SERIF` 的 `cjk_bundled` 改为 `true`。

## 完整性自检

字体文件截断不会在编译期报错。`fonts::missing()` 在注册后反查 `text_system().all_font_names()`，截断的字体不会出现，因此启动时会打印缺失项——这是实测抓到 `NotoSerifSC.ttf` 截断的方法。
