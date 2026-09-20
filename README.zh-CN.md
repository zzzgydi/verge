# Verge

[English](README.md)

Verge 是使用 Rust、GPUI Kit 和 Mihomo 开发的 macOS 原生代理客户端，采用原生界面、常驻守护进程、类型明确的应用命令，以及权限范围受限的 helper。

> 开发状态：项目仍在重写阶段。GPUI 分支已经可以开发和测试，但发布构建目前只支持 macOS 13 及以上的 Apple Silicon 设备。

## 功能

- GPUI 原生界面，支持中文和英文。
- 菜单栏守护进程常驻；关闭窗口不会停止代理。
- 从本地 YAML 或远程订阅导入 Mihomo 配置，支持自定义 User-Agent 和定时更新。
- 支持 Merge 配置、合并结果预览、配置校验、原子写入、健康检查和失败回滚。
- 支持 Rule、Global、Direct 模式，以及代理组切换和延迟测试。
- 实时查看流量、内存、连接、规则、provider 和日志。
- 管理 macOS HTTP、HTTPS、SOCKS、PAC 和 bypass，并保存恢复记录。
- 通过版本化特权 helper 管理 TUN 生命周期。
- 支持开机启动、全局快捷键、系统通知、诊断导出、设置导入导出。
- 校验并更新 Mihomo，也可校验签名后更新应用本身。

## 架构

Verge 用同一个可执行文件承载两种模式：

```text
Verge.app
  |
  +-- GUI 模式：GPUI 窗口和临时界面状态
  |
  +-- --daemon：菜单栏、命令处理、持久化、Mihomo、系统集成
          |
          +-- Unix socket IPC <data_dir>/daemon.sock
          +-- Mihomo sidecar
          +-- 管理 TUN 的特权 helper
```

GUI 通过本机 Unix socket 发送 typed request，不直接调用 Mihomo 或 macOS 系统 API。守护进程持有长期状态，校验并执行命令，管理 Mihomo，再把批量实时事件发回 GUI。

workspace 包含主应用、特权 helper 及共用协议：

| 路径 | 职责 |
|---|---|
| `apps/verge` | 主应用 crate，包含 GPUI、守护进程、IPC、命令、Profile、Mihomo 和 macOS 集成 |
| `apps/verge-helper` | 管理 TUN 的特权 helper |
| `crates/verge-helper-protocol` | 主应用和特权 helper 共用的窄协议 |
| `assets/icons` | 应用图标和菜单栏图标 |
| `assets/branding` | 可复用的品牌素材 |
| `assets/mihomo/manifest.json` | 固定 Mihomo 版本和 SHA-256 |

主应用内部按 `apps/verge/src/` 下的 Rust module 划分职责。UI、守护进程、协议、应用编排、配置、Mihomo 和平台代码仍保持边界，但不再为每一层单独建立 Cargo package。

早期 React/Tauri/sing-box 实现和 Phase 0 spike 工程已经移除。

UI 依赖和 API 使用约定见 [GPUI Kit 接入说明](docs/gpui-kit.md)。

macOS 系统代理读写使用 [sysproxy-rs](https://github.com/zzzgydi/sysproxy-rs) 的 Git `main` 分支，实际提交记录在 `Cargo.lock` 中。恢复快照、回滚、写后验证和代理守卫仍由 Verge 管理。

## 环境要求

- 开发环境按 GPUI Kit 当前要求使用 macOS 15 或更高版本。
  应用包最低版本仍为 13.0；升级后的 macOS 13/14 运行兼容性尚未复核。
- 当前 `.app` 打包流程要求 Apple Silicon。
- Xcode Command Line Tools。
- Rust `1.97.1`，并安装 `rustfmt` 和 `clippy`。
- 打包和真实契约测试需要 Mihomo `v1.19.26` arm64 二进制。

```bash
xcode-select --install
rustup toolchain install 1.97.1 --component rustfmt --component clippy
```

## 开发

克隆仓库并启动 GPUI 应用：

```bash
git clone git@github.com:zzzgydi/verge.git
cd verge
make dev
```

`make dev` 会构建并打开 `dist/Verge Dev.app`，应用标识为
`com.zzzgydi.verge.dev`，图标带 DEV 标记。数据保存在
`~/Library/Application Support/Verge Dev`，配置、日志、socket、锁和 Keychain
服务都与正式版分开。脚本自动准备固定版本的 Mihomo，校验 SHA-256 后放进 Dev
应用包；下载缓存位于 `.cache/mihomo/`。目前支持 Apple Silicon macOS。

编译和签名校验成功后，脚本只退出旧 Dev daemon，再启动新版本；编译失败时保留
旧 Dev。`make dev-stop` 会停止开发版的后台、内核和窗口；`make dev-build` 替换
开发包并退出旧 Dev，但不打开新窗口。使用自定义 `VERGE_DATA_DIR` 时，停止命令
也要传入同一个值。测试性能可运行 `VERGE_DEV_PROFILE=release make dev`，优化
编译后仍是 Dev 身份。

开发版可以与 `/Applications/Verge.app` 同时运行。Dev 后台禁止修改系统代理和
PAC、开启 TUN、安装或卸载 helper、注册登录项和全局快捷键，以及替换应用；导入
Profile 或 Merge 也无法开启 TUN。日常开发可测试界面、规则、连接、配置和 AI，
系统网络接管功能需另行安排测试。端口由你手动错开，除了 mixed-port，还要检查
DNS、HTTP/SOCKS/redir/tproxy 和外部控制器等监听项。测试流量显式发往 Dev 的
代理端口；正式版的 TUN、系统 DNS 仍可能影响开发版的出站流量。

Dev 首次使用新数据目录，按需导入配置，不要整目录复制正式版数据或密钥引用。
Dev 会拒绝正式版默认数据目录及其父子路径，也会检查符号链接。

只准备 Mihomo，不启动应用：

```bash
make mihomo
```

第一个 GUI 进程会用 `--daemon` 参数拉起同一个可执行文件，再通过本机 socket 连接守护进程。关闭窗口后，守护进程和菜单栏图标仍会运行；需要完全退出时，请从菜单栏选择“退出”。

开发时可以隔离数据目录，并指定本地 Mihomo：

```bash
VERGE_DATA_DIR=/tmp/verge-dev \
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
make dev
```

常用开发环境变量：

| 变量 | 用途 |
|---|---|
| `VERGE_DATA_DIR` | 自定义绝对路径；Dev 默认 `~/Library/Application Support/Verge Dev`，正式版默认 `~/Library/Application Support/Verge` |
| `VERGE_DEV_PROFILE` | `dev`（默认）或 `release` 优化编译，均保留 Dev 身份 |
| `VERGE_MIHOMO_BIN` | 指定 Mihomo 可执行文件 |
| `VERGE_MIHOMO_CACHE_DIR` | 覆盖 `make dev` 使用的仓库 Mihomo 缓存目录 |
| `VERGE_MIHOMO_MANIFEST` | 指定 sidecar manifest |
| `VERGE_CONTROLLER` | 指定回环 controller 地址 |
| `VERGE_SECRET` | 指定 controller secret |
| `VERGE_NETWORK_SERVICES` | 指定逗号分隔的 macOS 网络服务 |
| `VERGE_HELPER_SOCKET` | 指定特权 helper socket |

这些变量只用于开发和测试。不要把真实密钥写进 shell 历史、提交文件或问题报告。

## 构建 macOS 应用

本机 Release 应用、拖拽安装 DMG 和自动更新 ZIP 可一键生成：

```bash
make release
```

命令会自动准备并校验固定版本的 Mihomo，使用 Rust `1.97.1` 和
`--release --locked` 构建主程序及 helper，移除包内 Rust 二进制的局部符号，
再签名应用。Cargo 原始产物保留，方便性能分析。Mihomo 保留上游签名和固定
SHA-256；helper 的摘要在签名完成后生成。

产物：

- `dist/Verge.app`：包含 Mihomo 和 TUN helper 的独立应用。
- `dist/Verge-macos-arm64.dmg`：拖拽安装包。
- `dist/Verge-macos-arm64.dmg.sha256`：DMG 摘要。
- `dist/Verge-macos-arm64.zip`：供自动更新使用的应用压缩包。
- `dist/Verge-macos-arm64.zip.sha256`：压缩包摘要。

打开 DMG，将 Verge 拖到 Applications，然后从“应用程序”启动。
`make release` 只生成产物，不安装、启动或退出应用。日常使用安装后的版本，
不要长期从 `dist` 运行；下次构建会替换这个目录中的应用包。

DMG 脚本自动下载并校验固定版本的 `create-dmg` v1.3.0，缓存在 `.cache/`。
Finder 布局需要 macOS 图形会话，并允许执行命令的终端控制 Finder。ZIP 继续
用于应用自动更新。之后可用 `make release-size` 查看各产物体积，无需重新构建。

```bash
open dist/Verge-macos-arm64.dmg
```

在终端中构建并启动包内程序：

```bash
make release-run
# 使用独立数据目录测试：
VERGE_DATA_DIR=/tmp/verge-release-test make release-run
```

`make release-run` 使用正式版身份和数据目录。测试 `dist` 中的正式包前，先从
菜单栏退出已有正式 daemon；仅关闭窗口不会结束后台。同时开发或测试性能时，
使用 `make dev` 或 `VERGE_DEV_PROFILE=release make dev`。
`make release-run` 在 GUI 退出后返回终端，daemon 仍需通过菜单栏“退出”结束。

对比运行内存时，应使用相同配置、页面、流量和观察时长。在“活动监视器”中同时
观察两个 `verge-gpui` 进程（GUI 和 daemon）及其 `mihomo` 子进程。文件体积和
运行内存是不同指标，Release 文件变小不代表内存会同比下降。

仍可指定 Mihomo 路径和签名证书：

```bash
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
VERGE_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
make release
```

默认使用供本机测试的 ad-hoc 签名，并执行 `codesign --verify --deep --strict`
验证应用包。脚本不负责公证或发布。只给外层应用签 Developer ID 还不足以完成
公证：重新签名 Mihomo 会改变固定摘要，需要同时调整构建和运行时校验。

## 测试

运行仓库检查：

```bash
./scripts/check-rust-workspaces.sh
```

检查脚本会测试统一的 Rust workspace，并对全部 target 执行 `clippy -D warnings`。

真实 Mihomo 契约测试默认忽略：

```bash
MIHOMO_BIN=/absolute/path/to/mihomo \
  cargo +1.97.1 test -p verge --test mihomo_contract -- --ignored

MIHOMO_BIN=/absolute/path/to/mihomo \
  cargo +1.97.1 test -p verge --test mihomo_runtime_contract -- --ignored
```

传入的二进制必须符合 `assets/mihomo/manifest.json` 中记录的摘要。

## 使用

1. 从 DMG 安装 Verge，再从“应用程序”启动。开发时使用 `make dev`，系统网络仍由正式版管理。
2. 打开“配置”，粘贴本地 YAML 内容或填写远程订阅地址。
3. 启用配置。Verge 会先校验并生成私有运行配置，再启动或重载 Mihomo。
4. 在“概览”或菜单栏打开系统代理，并选择 Rule、Global 或 Direct 模式。
5. 在“代理”“规则”“连接”和“日志”中查看实时状态。
6. 在“设置”中管理 TUN、DNS、IPv6、SOCKS/PAC/bypass、开机启动、快捷键、更新和诊断。

安装或卸载特权 helper、开启 TUN 和替换应用都会修改系统状态。Verge 会先请求确认，macOS 也可能要求管理员授权。

应用数据默认保存在 `~/Library/Application Support/Verge`。日志位于该目录下的 `logs/verge.log`，另保留 3 份轮转文件，每份最多 2 MiB。GUI 与 daemon 通过文件锁协调轮转。诊断导出会遮盖 controller secret、订阅 URL、认证头和用户主目录。

## AI 助手

在侧栏 **AI 助手 → 模型设置** 填写 Base URL（包含接口前缀，例如 `/v1`）、模型和 API key，保存后测试连接。密钥通过 macOS Keychain 保存，`ai.json` 只保存普通配置和密钥引用；更换服务地址需要重新填写密钥或清除旧密钥。本地 loopback 服务可以使用 HTTP。

发送问题会采集有上限的运行状态、代理组、规则类型、连接数量和错误数量，回答可以引用本轮依据。完整配置、连接目标和日志正文不自动发送。推理在独立后台任务中执行，可停止；关闭窗口或 IPC 断线会取消，已生成内容保留在 daemon 内存中，完整退出后清除。目前尚未使用真实 Provider 验收。

## 平台状态

领域层和应用层已经通过 adapter 隔离平台 API，但当前可发布实现仍以 macOS 为先。Windows、Linux 和 Intel macOS 暂无安装包。AI 助手已接通兼容 OpenAI Chat Completions 的流式只读诊断；节点建议、配置预览和经确认的写操作仍待实现。

## 许可证

[GNU General Public License v3.0](LICENSE)
