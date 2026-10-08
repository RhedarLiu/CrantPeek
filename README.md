# Crant Peek

需要时出现，看完即走。Windows / macOS 上的 Rust 浮窗查词与 AI 阅读助手，非 Web UI，MIT 开源。

**开发中的原型，不是完整产品。** 下面的状态区分“已验证”“只编译过”“还没做”，请以此为准。

## 入口

- 双击 Ctrl：仅查询可读取的选区，无选区保持静默（不回退剪贴板）。
- macOS ⌘⇧A：空白浮窗，不自动带入选区/剪贴板。
- macOS ⌘⇧D：主显示器截图框选 → 本地 OCR → 查询（原型，未交互验收）。
- Windows 空白/截图组合键暂用 Alt+Shift+A / D，产品默认尚待确认。

## 状态

| 功能 | 状态 |
| --- | --- |
| 三类回答接口（OpenAI Chat / Responses、Anthropic）流式、取消 | 已实现；本地 HTTP 集成测试通过；**未对真实服务验证** |
| 浮窗、设置、追问、复制、托盘、全局空白浮窗快捷键 | 已实现；macOS 启动冒烟通过；**视觉与交互未验收** |
| 凭据存系统钥匙串，配置文件不含密钥 | 已实现 |
| 离线词典（ECDICT，约 40 万词条） | 已实现并用真实数据测试：加载约 11ms，单次查询约 2µs；**未在 UI 中目测** |
| macOS 双击 Ctrl + 辅助功能读取选区 | 已写好、单元测试覆盖双击判定；**从未在真实应用实机验证**；需授予辅助功能、输入监控权限，权限引导 UI 未做 |
| Windows 双击 Ctrl（低级钩子）+ UI Automation 读取选区 | 已写好，通过 Windows 目标 clippy；**只证明能编译，从未运行过** |
| 决策模型（Jev / Clef）客户端 | 网络层与响应校验已实现；**尚未接入 UI，也未对真实服务验证** |
| 截图 + OCR | 原型已接线：主显示器截图、拖动框选、内存裁剪、macOS Vision / Windows.Media.Ocr、本地识字后查询；20 项测试通过（含缩放裁剪测试），两平台严格 lint 通过。**未验证真实屏幕框选、OCR 效果；副屏与取消后迟到响应处理尚待优化** |
| 多模态决策 / 回答 | **未实现** |
| 空白/截图快捷键冲突检测、权限引导、首次使用引导 | **未实现** |

## 运行

```sh
cargo run -p peek-app
```

### 离线词典

词库不随仓库提交（约 30 MB）。数据来自 [ECDICT](https://github.com/skywind3000/ECDICT)（MIT）：

```sh
mkdir -p local-assets
git clone --depth 1 https://github.com/skywind3000/ECDICT.git local-assets/ecdict
cargo run --release -p peek-dict --bin build-dict -- local-assets/ecdict/ecdict.csv local-assets/ecdict.pkd
```

应用依次查找配置目录下的 `ecdict.pkd` 和 `local-assets/ecdict.pkd`，找不到则不显示词典卡片，其余功能不受影响。

## 开发验证

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p peek-app --target x86_64-pc-windows-msvc -- -D warnings   # Windows 交叉检查
```

## 文档

- [用户体验说明](USER_GUIDE.md)
- [产品规划](PRODUCT_PLAN.md)

编译通过不等于系统级功能可用。Windows 钩子、权限、多屏截图等需要 Windows 实机验证；macOS 同样需要权限与应用兼容性测试。

仓库中不存放 API key、私有查询或截图。
