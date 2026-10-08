# Crant Peek

需要时出现，看完即走。Windows / macOS 上的 Rust 浮窗查词与 AI 阅读助手，非 Web UI，MIT 开源。

## 当前状态

开发中。初版 egui 桌面应用已在 macOS 编译并完成进程启动冒烟检查（没有崩溃输出，未进行视觉/交互验收）。手动输入、模式选择、流式回答、追问、取消、复制、设置与系统凭据存储已接线。已加入全局空白浮窗快捷键和托盘菜单，可从菜单重新打开窗口；失焦/关闭默认隐藏，菜单提供退出。截图快捷键目前仅显示开发中提示，不是已完成截图功能。macOS 已加入双击 Ctrl 监听（CGEventTap，仅监听）与通过辅助功能 API 读取选区：无选区/读取失败静默，不回退剪贴板；双击判定有单元测试，Ctrl+C 等组合键会打断。13 项测试通过，Clippy 严格检查通过。**该取词路径尚未在真实应用中实机验证**：需要在“系统设置 → 隐私与安全性”授予辅助功能和输入监控权限，权限引导 UI 还没做；Electron/浏览器等应用的辅助功能支持因应用而异。Windows 取词与双击 Ctrl、词典、截图/OCR、决策客户端接入 UI、真实模型调用尚待完成。不要把原型当完整产品。

启动开发原型：

```sh
cargo run -p peek-app
```

- 双击 Ctrl：仅查询可读取的选区，无选区保持静默。
- macOS Command+Shift+A：空白浮窗，不自动带入选区/剪贴板。
- macOS Command+Shift+D：截图入口。
- Windows 空白/截图组合键当前代码使用 Alt+Shift+A / D 作为临时默认值，产品默认尚待确认。

## 开发验证

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## 文档

- [用户体验说明](USER_GUIDE.md)
- [产品规划](PRODUCT_PLAN.md)

不把编译通过等同于系统级功能可用。Windows 钩子、权限、多屏截图等需要 Windows 实机验证；macOS 同样需要权限与应用兼容性测试。

不在仓库里存放 API key、私有查询或截图。
