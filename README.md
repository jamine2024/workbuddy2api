# WorkBuddy2API 便携版

把 WorkBuddy 账号变成本地 OpenAI 兼容接口的 Windows 桌面程序：Rust 原生控制面板 + Go 网关 + 内置指纹浏览器，解压即用，不需要另装 Python 或 Go。

当前版本 **0.1.6** · Windows 10 / 11 x64

---

## 这是什么

网关核心来自上游开源项目 [HanawaBanana/workbuddy2api](https://github.com/HanawaBanana/workbuddy2api)（原始作者 [Sliverkiss](https://github.com/Sliverkiss/workbuddy2api)，MIT）。本仓库是在其之上做的 Windows 便携化封装：

- 网关源码按上游记录的提交重建，新增受 API 密钥保护的只读实时统计接口，供悬浮窗每秒采样；
- 新增 Rust 原生控制面板（托盘常驻、单文件 EXE），把「装配置 → 加账号 → 起服务」三步收进一个窗口；
- 内置 Camoufox 指纹浏览器，账号登录环境互相隔离；
- 保留本地 Web 面板（`control_panel.py` + `ui/`）作为轻量替代入口。

上游 README 明确「不内嵌 Web 管理面板，可视化面板作为独立项目维护」，本项目沿用这个思路：网关只管转发，界面单独做。

## 功能特性

| | |
|---|---|
| **多账号池** | 多个 WorkBuddy 账号轮转，每账号独立浏览器环境与登录态 |
| **OpenAI 兼容** | `/v1/models`、`/v1/chat/completions`，支持 SSE 流式与聚合 JSON |
| **自动故障转移** | 上游返回 429 / 503 且尚未输出内容时，自动改用同池其他账号的同一模型 |
| **免费网关** | 内置 OpenCode / Kilo 匿名通道，用 `opencode:` / `kilo:` 前缀调用，无需账号 |
| **实时用量** | `/v1/usage/live` 返回聚合计数（进行中请求、已完成输出 token、估算中的 token），不落盘任何生成内容 |
| **悬浮窗 HUD** | 每秒采样网关计数，生成中显示带 `≈` 的估算值，请求结束替换为上游实际用量 |
| **绿色运行** | 内置 Camoufox 浏览器内核，不写系统目录，卸载保留用户数据；面板运行不需要另装 Python 或 Go |

## 下载与安装

到 [Releases](../../releases) 下载 `WorkBuddy2API-<版本>-Setup.exe`，双击安装，按当前用户安装、**不需要管理员权限**。

- 默认程序目录：`%LOCALAPPDATA%\Programs\WorkBuddy2API`
- 默认数据目录：`%LOCALAPPDATA%\WorkBuddy2API`
- 桌面与开始菜单会创建快捷方式；卸载入口在「已安装的应用」，卸载保留用户数据
- 同目录 `.sha256` 文件是安装包校验值

> 安装包约 178 MB，超出仓库单文件限制，因此只作为 Release 附件分发，不进 Git 历史。

首次登录会释放内置浏览器内核，建议预留 1 GB 空间。

## 从源码运行

需要 Windows + Python 3.12（仓库自带 `runtime/` 绿色运行时可直接用）。

```powershell
# 1) 生成 config.json（交互式：端口 / API 密钥 / 对外 IP）
runtime\python\python.exe install.py --dry-run   # 先预览改动
runtime\python\python.exe install.py

# 2) 添加账号（拉起内置 Camoufox 完成登录）
runtime\python\python.exe addaccount.py

# 3) 启动网关
.\wb2api.exe
```

也可以走 Web 面板：`runtime\python\python.exe control_panel.py`，浏览器打开 `http://127.0.0.1:8765`（只监听本机回环）。

## 构建

三个部分相互独立，产物顺序是：内置浏览器内核 → 网关 → 桌面 → 安装包。

```powershell
# 0) 备齐内置浏览器内核（桌面端编译的前置，见下方说明）
python tools\prepare_login_browser.py

# 网关（Go）
cd gateway
go test ./cmd/server ./internal/freegateway ./internal/server ./internal/upstream
go build -trimpath -ldflags '-s -w' -o ../wb2api.exe ./cmd/server

# 桌面（Rust，会内嵌上一步的 wb2api.exe，需先停掉正在运行的网关）
cd ../rust
cargo build --release

# 安装包（NSIS）
.\tools\Build-Installer.ps1
```

第 0 步不能省：`rust/src/login_browser.rs` 用 `include_bytes!` 把 `browser/camoufox-win-x64.zip` 直接编进桌面 EXE，而这个 zip 被 `.gitignore` 排除、也不在 Releases 里，**clone 后不跑第 0 步则 `cargo build` 会以 `couldn't read ...camoufox-win-x64.zip` 失败**。该脚本从上游 Camoufox 发布页下载 1.22 GiB 原包、剔除跨平台字体后重打包成 164 MB，需要相应带宽与磁盘空间；校验值记录在 `browser/manifest.json`。

`go test ./...` 会有一条 `TestRunSchoolNowBuildsCommand` 失败——便携源码包未包含仓库根 `scripts/` 下的 `school_open_day_2026.py` 等活动脚本（该目录当前只有 `list_models.py`），属已知情况，原因与影响见 [gateway/README.md](gateway/README.md)。

## 目录结构

```
gateway/          Go 网关源码（含上游 MIT 声明与新增统计接口）
rust/             Rust 控制面板 / 悬浮窗 / 托盘
ui/               本地 Web 控制面板前端
control_panel.py  Web 面板后端（仅标准库）
install.py        命令行安装器
addaccount.py     账号登录辅助
scripts/          辅助脚本（如模型列表）
runtime/          绿色 Python 运行时
browser/          Camoufox 内核清单（内核 zip 走 Release）
installer/        NSIS 安装脚本
tools/            构建与冒烟测试脚本
```

以下目录不入库：`auths/`、`data/`、`logs/`、`config.json`（含本机密钥）、`rust/target/`、`release/`。

## 配置

`config.json` 由安装器生成，关键字段：

```jsonc
{
  "listen": ":8080",              // 监听地址
  "api_key": "……",                // 客户端调用需携带的密钥
  "auth_dir": "./auths",          // 账号凭据目录
  "state_file": "./data/state.json",
  "free_gateways": {
    "opencode": { "enabled": true },
    "kilo":     { "enabled": true }
  }
}
```

改动后需要重启网关生效。

## 调用示例

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer <你的 api_key>" \
  -H "Content-Type: application/json" \
  -d '{"model":"<模型 ID>","messages":[{"role":"user","content":"你好"}],"stream":true}'
```

OpenAI SDK 把 `base_url` 指到 `http://127.0.0.1:8080/v1` 即可。免费通道用 `opencode:<模型>` 或 `kilo:<模型>` 作为 model ID。

## 数据与隐私

- 账号凭据、API 密钥、运行状态、日志全部留在本机数据目录，不进安装包、不进仓库
- `/v1/usage/live` 只返回聚合计数，不返回也不保存提示词、生成内容、账号标识
- 悬浮窗的生成中数值是按字符数换算的**估算**，不是计费口径；请求结束以上游实际用量为准
- Kilo 部分免费模型可能记录提示词用于改进服务，敏感内容不要发

## 许可与致谢

本项目以 MIT 发布，见 [LICENSE](LICENSE)。上游版权声明完整保留：

- 原始作者 [Sliverkiss/workbuddy2api](https://github.com/Sliverkiss/workbuddy2api)
- 延续仓库 [HanawaBanana/workbuddy2api](https://github.com/HanawaBanana/workbuddy2api)

第三方组件（Python Embeddable Package、Camoufox 内核、OpenCode 兼容层等）的版本与许可见 [NOTICE.md](NOTICE.md)，网关部分的改动说明见 [gateway/README.md](gateway/README.md)。

## 免责声明

本项目只做本地协议转换与账号管理，**不授予**任何上游平台接口或服务的权利。使用者需自行遵守 WorkBuddy / CodeBuddy 及各家免费网关的服务条款与当地法律法规，因使用产生的账号风险、配额限制或合规问题由使用者自行承担。
