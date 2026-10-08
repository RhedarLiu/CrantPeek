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
| macOS 双击 Ctrl + 辅助功能读取选区 | 已写好、单元测试覆盖双击判定；**从未在真实应用实机验证**；需授予辅助功能、输入监控权限，设置内可查看权限状态和打开系统设置 |
| Windows 双击 Ctrl（低级钩子）+ UI Automation 读取选区 | 已写好，通过 Windows 目标 clippy；**只证明能编译，从未运行过** |
| 决策模型（Jev / Clef）文本路由 | 已接入设置与查询：独立凭据、完整 endpoint、模型、阈值与超时；有信心时选择任务，不确定/超时/缺少凭据回退本地模式。词典单词和追问不重复决策；显示来源与置信度。**未对真实服务验证** |
| 截图 + OCR | 鼠标所在显示器截图、框选、内存裁剪、系统本地 OCR、文字查询；加载可取消，旧响应丢弃。大选区等比例缩小，透明白底合成。macOS Vision 白图运行冒烟通过。**真实框选、识字质量和混合 DPI 多屏坐标未验收** |
| 多模态决策 / 回答 | 已接入显式按钮：图片解读发送至启用 vision 的回答服务；Clef 判断发送至决策服务。目标地址可见，默认不上传图片。三类回答协议及 Clef 官方图片 schema 有测试。**未对真实服务与界面交互验收** |
| 权限诊断、首次使用引导、快捷键设置 | 已加入设置页：首次欢迎说明、macOS 权限状态/系统设置入口、快捷键格式与重复检查；修改快捷键需重启。跨应用冲突以注册失败报告，尚无交互验收。 |
| 界面多语言（i18n） | 中英文文案全部外置到 Fluent 资源，界面内不写死文案；可跟随系统或手动选择，保存后立即生效（含托盘菜单）。测试覆盖键一致、参数一致、资源引用、回退与异步语言。**新增语言需按 [I18N.md](I18N.md) 补全键。** |
| 浮窗视觉 | 黑白灰圆角卡片风格（原生 egui，无 Web 技术），浅色/深色/跟随系统、界面缩放、图标操作与固定页脚。已用隔离预览目测中英文浅深色；**Windows 实机与真实交互未验收** |

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
- [界面多语言与视觉规范](I18N.md)
- [实机验收清单](TESTING.md)
- [性能验证记录](PERFORMANCE.md)

编译通过不等于系统级功能可用。Windows 钩子、权限、多屏截图等需要 Windows 实机验证；macOS 同样需要权限与应用兼容性测试。

仓库中不存放 API key、私有查询或截图。
