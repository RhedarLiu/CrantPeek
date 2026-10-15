# 字体

界面固定使用 Noto Serif，不再提供字体方案切换。代码使用 JetBrains Mono。

| 文件 | 用途 |
|---|---|
| NotoSerif.ttf | 拉丁文字和界面 |
| JetBrainsMono.ttf | 代码与报错 |

中文和日文延用衬线方案的系统字体：macOS 使用 Songti SC / Hiragino Mincho ProN；Windows 使用 SimSun / Yu Mincho。两平台字形可能略有差异。

字体采用 SIL Open Font License 1.1。完整版权和许可保存在仓库 `assets/licenses/`，打包时原样复制至 Windows 程序目录 `licenses/` 和 macOS `Contents/Resources/licenses/`。上游：
- https://github.com/notofonts
- https://github.com/JetBrains/JetBrainsMono

启动时检查内置字体是否成功加载。旧字体偏好不再影响界面。
