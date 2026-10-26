# 第三方组件声明

本项目是一个**封装 / 集成性质的衍生作品**：网关核心能力来自上游开源项目，运行环境来自 Python 官方发行版。下表列出全部第三方组件及其许可。

| 组件 | 版本 / 位置 | 许可 | 来源 |
|---|---|---|---|
| **WorkBuddy2API 网关**（`wb2api.exe`，以及 `install.py` / `addaccount.py` / `scripts/list_models.py` 等脚本所对接的全部网关行为） | 源码见 `gateway/`（基于 `5755fe3` 重建），随包以 `wb2api.exe` 分发 | MIT | [HanawaBanana/workbuddy2api](https://github.com/HanawaBanana/workbuddy2api)（延续仓库）<br>原始作者 [Sliverkiss/workbuddy2api](https://github.com/Sliverkiss/workbuddy2api) |
| **Python Embeddable Package** | 3.12.10，目录 `runtime/python/` | PSF License Agreement | [python.org](https://www.python.org/downloads/windows/)，完整文本见 `runtime/python/LICENSE.txt` |
| **Camoufox 内置登录浏览器** | 156.0.1-beta.36，`browser/camoufox-win-x64.zip`，同时编译进桌面 EXE | MPL-2.0，包含上游第三方组件许可；Cursory 为 LGPL-3.0-or-later | [对应源码和原始发布](https://github.com/daijro/camoufox/tree/v156.0.1-beta.36)，完整 MPL 与 Cursory 许可随内核的 `licenses/` 分发；浏览器自带的其他许可可在 `about:license` 查看 |
| 本项目新增部分（`control_panel.py`、`ui/`、`0_panel.bat` 等） | — | MIT | 本仓库 |

---

## 归属与致谢

### 原始作者

WorkBuddy2API 网关最初由 **Sliverkiss** 开发并以 MIT License 开源：

- https://github.com/Sliverkiss/workbuddy2api

该原始仓库已于 2026-09-24 从 GitHub 消失（删除或转为私有）。

### 延续仓库

**HanawaBanana/workbuddy2api** 是原始仓库的延续副本，收录了原库公开过的全部功能并按原项目的 MIT License 继续维护：

- https://github.com/HanawaBanana/workbuddy2api

本项目所分发的 `wb2api.exe`、以及 `install.py`、`addaccount.py`、`scripts/` 等脚本与 `config.json` 配置结构，均源自上述项目。

### 本项目做了什么

- 网关基于原随包二进制记录的 `5755fe3fac45f5fd01c42e583afa0f70d9d6efc1` 版本源码重建，增加受原有 API 密钥保护的只读实时统计接口及流数据观察器，供悬浮窗每秒采样；生成中为明确标记的估算，结束后使用上游实际用量。新增部分与原始 MIT 声明一起保存在 `gateway/`，详见 `gateway/README.md`；
- 桌面部分包括 Rust 原生控制面板，以及保留的本地 Web 面板（`control_panel.py` + `ui/`）、Windows 批处理入口和绿色 Python 运行环境，用于简化 Windows 上的安装、加号、启动操作。

上游项目 README 中明确表示「不内嵌 Web 管理面板，可视化面板作为独立项目维护」，本项目即遵循该模式。

---

## MIT 合规说明

依据 MIT License 及上游仓库 README 的再分发要求，本项目：

1. 在根目录 `LICENSE` 中**完整保留**了上游 MIT 版权声明与许可声明（含原始作者 Sliverkiss 与延续仓库维护者 HanawaBanana 的署名）；
2. 在 README 与本文件中**注明原始出处** <https://github.com/Sliverkiss/workbuddy2api> 与 <https://github.com/HanawaBanana/workbuddy2api>；
3. 保留 Python 运行环境的 `runtime/python/LICENSE.txt`（PSF License）；
4. 声明本项目**不授予**任何上游（CodeBuddy / WorkBuddy）接口或服务的权利，使用者仍需自行遵守上游平台的服务条款。

若你是上游权利人，认为本仓库的署名或分发方式需要调整，请通过 Issues 联系，会及时处理。
