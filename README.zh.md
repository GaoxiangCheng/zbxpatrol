# zbxpatrol — Zabbix 服务器巡检报表系统

[English (default)](README.md) | **简体中文**

对 Zabbix（6.x–7.4，已在 7.4.14 验证）监控的全部服务器自动巡检，产出带**风险评分**的 Excel 报表，并输出结构化数据（JSON/CSV/HTTP API）供第三方程序（运维管理平台）消费。单一二进制、无运行时依赖，可直接部署到任意服务器无人值守运行。

- **完整文档（默认英文 + 中文版）**：[docs/README.md](docs/README.md) — API 接口 / 交互向导 / CLI 参考 / 关键代码说明，多级锚点目录可直接跳转。
- 架构：`crates/zbxpatrol-core`（全部业务逻辑，平台可直接依赖） + `crates/zbxpatrol-cli`（CLI/向导/渲染/HTTP 壳）。

## 三步上手

**首次运行无需预设任何变量**——在终端直接运行任意命令（如 `./zbxpatrol` 或 `./zbxpatrol check`），检测到没有配置时会进入交互初始化：逐项提示输入地址、账号、密码（不回显）、是否跳过证书校验、时区，验证连通后自动保存到 **`~/.zbxpatrol/config.env`**（权限 600），之后所有命令直接可用。

```bash
# 1. 首次运行（终端中），按提示输入即可
./zbxpatrol check

# 2. 出报表（默认全部主机 + 近 24 小时，输出到 ./reports/）
./zbxpatrol report
```

不带子命令直接运行 `./zbxpatrol` 进入**交互式向导**：输入数字或字母选择，**`?` 查看当前层可用输入**（帮助后自动重印「可输入」提示行），**`q` 返回上层，`exit`/`quit` 退出程序**，`m`/`g`/`h` 跨级跳转（主菜单/群组/主机列表），空回车静默重试；提示符带**层级路径前缀**（如 `[主菜单>浏览><群组名>><主机名>]`），**Tab 补全**（候选只含数据项：群组名/主机名/监控项 key，最多显示 12 个、唯一候选直接补全；每个输入点上方有灰色「可输入：…」提示行说明当前能填什么）。

- **监控项智能输入**：在主机上下文输入 key（或片段/通配）——**唯一匹配直接出结果，多个匹配自动进选择列表**（如输入 `load` → 列出 3 个 load 项供选择）；
- **分级浏览**（主菜单 1）：群组 → 主机 → 指标 逐级下钻；列表**会话内缓存**、f 过滤、n/p 翻页；选定主机后**停留在主机上下文**反复操作——看监控项、趋势图（预设或**自定义监控项**，支持列表选择/智能输入）、快速巡检、查 key；
- **生成巡检报表**：范围 → 时间 → 严格度 → **附加自定义指标**（列表选择或输入）→ 输出（控制台 / Excel / **CSV 文件** / **JSON 文件** 可组合）；
- **查询指标**：列表挑 key 或直接输入（通配），同范围连续查询；
- 连通性自检。

非交互（CLI）与交互功能对等：`query --chart`、`report --keys`、`--from/--to` 自定义区间等均可脚本化；每个子命令 `-h` 附常用示例。

巡检结果也可以直接在控制台查看彩色明细表（按风险降序，使用率绿/黄/红着色；含**系统类型列**与 **CPU/内存趋势火花线** ▁▂▃▅▆█）——不只想导出 xlsx 时：

```bash
zbxpatrol report --group <群组名>          # 输出 xlsx 同时打印控制台明细表（--format table 默认）
```

## 命令一览

以下示例中 `<群组名>`/`<主机名>`/`<监控项key>` 等尖括号内容为**变量**，请替换为实际值（安装补全后可用 Tab 直接补全真实群组/主机名）。

```bash
zbxpatrol                            # 交互式向导（仅 TTY）
zbxpatrol check                      # 自检：连通/凭据/版本/权限
zbxpatrol serve --listen 127.0.0.1:8787 [--token XXX]   # 本地 HTTP API
zbxpatrol groups                     # 主机群组列表（--search 过滤，--page/--size 分页）
zbxpatrol hosts [--group X] [--search <子串>] [--size 20 --page 2]   # 主机列表（系统类型+过滤+分页）
zbxpatrol items [--host H|--group G] [--search cpu] [--detail]  # 监控项清单
zbxpatrol query --key "system.cpu.util" --last 7d       # 手动查询任意指标（含趋势火花线）
zbxpatrol query --key "net.if*" --group "Web" --csv out.csv
zbxpatrol query --host <主机名> --key system.cpu.util --chart   # 表格 + 唯一系列全尺寸趋势图


zbxpatrol report --group <群组名> --keys 'net.if*,proc.num'    # 报表附加自定义指标（专设 sheet）

# 巡检报表
zbxpatrol report                                     # 全部主机，日报
zbxpatrol report --group "<群组名>"                    # 按群组（--group 可多次）
zbxpatrol report --host <主机名>                       # 单台
zbxpatrol report --hosts a,b,c                       # 多台
zbxpatrol report --period week|month|year            # 周/月/年（年度含满盘预测）
zbxpatrol report --last 48h                          # 任意相对时长（h/d/M/m）
zbxpatrol report --from "2026-09-01" --to "2026-09-15 23:59"   # 任意绝对区间
zbxpatrol report --group "<群组名>" --period week --strictness strict   # 严格评分
zbxpatrol report --group X --all-items --data-json out.json    # 全量指标 + 结构化数据
```

通用参数：`--format table|json|csv`（json/csv 供程序解析）、`--quiet`、`--no-interactive`、`--config patrol.toml`、`--out <目录>`。

**Shell 补全（Tab 补全子命令/选项，`--group`/`--host` 直接补全 Zabbix 真实群组/主机名）**：

```bash
zbxpatrol completions bash | sudo tee /etc/bash_completion.d/zbxpatrol   # bash 永久安装
source <(zbxpatrol completions bash)                                     # 临时生效
zbxpatrol completions zsh > ~/.zfunc/_zbxpatrol                          # zsh
```
安装后：`zbxpatrol <TAB>` 列子命令；`items --group <TAB>` 列真实群组名；`--host <TAB>` 列真实主机名；`--metric/--period/--strictness/--format` 等枚举选项均可补全。CLI `-h` 帮助已按「Options / 时间范围 / 范围」分组分层展示。

**退出码**：0 成功 / 2 配置或凭据错误 / 3 网络或 API 错误 / 4 部分数据缺失（报表仍生成）。数据走 stdout、日志进度走 stderr，管道安全；非 TTY 自动禁用交互。

## 配置

连接信息三层来源，**优先级从高到低**（高层存在则低层忽略）：

1. **进程环境变量**：`ZBX_URL` / `ZBX_USER` / `ZBX_PASSWORD`（CI、cron、systemd 注入用）
2. **当前目录 `.env`**：项目/部署目录级（`deploy/` 示例即此方式）
3. **`~/.zbxpatrol/config.env`**：用户级，交互式初始化自动生成（权限 600）

首次运行（终端环境）自动进入交互初始化并写入家目录；非交互环境（管道/CI/被程序调用）缺少配置时直接报错退出码 2，绝不挂起。

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `ZBX_URL` | ✔ | — | Zabbix 前端地址 |
| `ZBX_USER` / `ZBX_PASSWORD` | ✔ | — | API 账号（需开启 API 访问） |
| `ZBX_TIMEOUT` | | 30 | 请求超时（秒） |
| `ZBX_INSECURE` | | false | 跳过 TLS 证书校验 |
| `ZBX_TZ` | | Asia/Shanghai | 时区 |
| `PATROL_CONCURRENCY` | | 8 | 采集并发 |
| `PATROL_RATE_LIMIT` | | 10 | API 限速（req/s） |

## 报表内容

Excel（`巡检报告_<起>-<止>_<范围>.xlsx`，条件格式绿/黄/红）：

1. **巡检总览**：区间、可用性统计、风险分布、TOP10、结论建议
2. **主机明细**：状态（正常/不可达/**停用**——停用主机不参与评分）+ CPU/内存/磁盘(最满分区)/swap/inode 各 **当前/平均/最大/最小** + 负载 + 风险分/等级/风险点
3. **磁盘分区明细**：容量、空间%、inode%、预计满盘天数（>7 天区间）
4. **网络与服务**：网卡出入带宽（Mbps）、带宽利用率、错包/丢包、端口/服务探测
5. **稳定性与安全**：重启次数、时间同步偏移、僵尸进程、fd 使用率、证书天数
6. **问题与告警**：区间内 problem（级别/状态/ack）；触发器已停用/主机已停用的问题标记「**停用**」，不计入未恢复统计、不参与评分
7. **评分说明**：阈值与权重
8. **全部指标明细**（`--all-items`）

**风险评分**（0–100）**三档严格度**：`--strictness loose|standard|strict`（默认 standard，交互向导与 HTTP API 同样支持），基准即高危线——**宽松 90% / 标准 80% / 严格 70%**，全部百分比类阈值随基准联动（宽松较标准放宽 10 个点、严格收紧 10 个点）：

| 阶梯（严重/高危/中危/低危） | 宽松（基准90） | 标准（基准80） | 严格（基准70） |
|---|---|---|---|
| CPU | 95/85/75/60 | 85/75/65/50 | 75/65/55/40 |
| 内存 | 98/90/80/70 | 90/80/70/60 | 80/70/60/50 |
| 磁盘/inode | 98/95/90/85 | 90/85/80/75 | 80/75/70/65 |
| CPU 峰值 / swap / 带宽、fd | 99 / 60 / 90 | 95 / 50 / 80 | 85 / 40 / 70 |

可用性/服务失败直接严重；重启、OOM、僵尸、时间偏移、证书、高危告警等事件类规则不随基准缩放。等级：0–39 健康 / 40–59 低危 / 60–74 中危 / 75–89 高危 / 90–100 严重。所用模式与阈值写入报表总览、评分说明 sheet 与 JSON（`strictness` 字段）。`patrol.toml` 的 `[[scoring]]` 自定义规则存在时整体替换内置评分，不受模式影响（示例见 `patrol.example.toml`，**完整自定义打分文档：[docs/SCORING.zh.md](docs/SCORING.zh.md)**）。已停用的主机、监控项、触发器不参与评分；停用主机在报表中标记「停用」并单独计数。

## 指标体系（环境自适应）

按 key 正则自动发现，**存在才启用、不存在不占位**：CPU（利用/负载/核数）、内存、swap、磁盘（空间+inode，兼容 `vfs.fs.size` 老格式与 `vfs.fs.dependent.size` 新格式、Windows 分区）、网卡流量/错包/丢包、运行时长/重启检测（boottime 容差去重 + uptime 交叉验证）、时间同步偏移、僵尸进程、fd、端口/服务探测、ICMP、证书、Docker/数据库/IPMI（如配置）。>1 天区间自动用小时级 trend（加权平均），≤1 天用 history；累计计数器自动差分折算速率。自定义指标在 `patrol.toml` 加一条正则即可接入。

## HTTP API（第三方调用）

```bash
zbxpatrol serve --listen 127.0.0.1:8787 --token mytoken   # 生产建议：export PATROL_TOKEN=mytoken（避免 token 出现在 ps/命令行）
```

| 端点 | 说明 |
|---|---|
| `GET /health` | 存活 + Zabbix 版本 |
| `GET /groups` `GET /hosts?group=X` | 群组/主机 |
| `GET /items?host=X&search=cpu` | 监控项清单 |
| `POST /query` | `{"keys":["system.cpu.util"],"hosts":["h1"],"time":{"last":"7d"}}` |
| `POST /report` | `{"scope":...}` 同上，另支持 `"strictness":"strict"`；`?format=xlsx` 返回文件流 |

响应统一 `{"ok":true,"data":...}`；鉴权 `Authorization: Bearer <token>`；默认仅本机监听，对外必须设 token。Python 调用示例：

```python
import requests
r = requests.post("http://127.0.0.1:8787/report",
    headers={"Authorization": "Bearer mytoken"},
    json={"group": "<群组名>", "time": {"period": "day"}}, timeout=300)
for h in r.json()["data"]["hosts"]:
    print(h["host"]["host"], h["risk"]["score"], h["risk"]["level"])
```

## 部署到服务器长期运行

产物：`target/release/zbxpatrol`（macOS arm64）；Linux 用 musl 静态交叉编译：

```bash
# 方式一：zigbuild（推荐）
cargo install cargo-zigbuild && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
cargo zigbuild --release --target x86_64-unknown-linux-musl

# 方式二：cross（需要 Docker）
cargo install cross && cross build --release --target x86_64-unknown-linux-musl
```

- **定时巡检**：`deploy/crontab.example`（日/周/月报）或 systemd timer（`deploy/zbxpatrol-daily.service`）
- **常驻 API**：`deploy/zbxpatrol-serve.service`
- **Docker**：`deploy/Dockerfile`

> **内网 DNS 兼容（已内置解决）**：程序内置 hickory 纯 Rust DNS 解析器（查询行为与 glibc/curl 一致），静态二进制在各类内网 DNS 环境下均可正常解析，**无需任何额外配置**。此特性在 Rocky Linux 9.8 + 内网 DNS 环境实测通过（此前 musl 自带解析器与部分内网 DNS 不兼容的问题已根治）。若极端环境仍报 `Name does not resolve`，兜底方案：`echo "<Zabbix服务器IP> <你的Zabbix域名>" >> /etc/hosts`（示例，替换为实际值）。

## 从 GitHub Releases 安装

各平台预编译静态二进制附带在每个 [GitHub Release](https://github.com/GaoxiangCheng/zbxpatrol/releases)（Linux x86_64 / Linux aarch64 / macOS Apple 芯片，无运行时依赖，SHA-256 见 `checksums-sha256.txt`）。`releases/latest/download` 直链永远指向最新版本：

```bash
curl -LO https://github.com/GaoxiangCheng/zbxpatrol/releases/latest/download/zbxpatrol-linux-x86_64
chmod +x zbxpatrol && ./zbxpatrol check          # ./zbxpatrol --version 查看发布版本

# macOS Apple Silicon（M 系列）+ 未签名提示：
curl -LO https://github.com/GaoxiangCheng/zbxpatrol/releases/latest/download/zbxpatrol-macos-arm64
chmod +x zbxpatrol-macos-arm64 && xattr -d com.apple.quarantine zbxpatrol-macos-arm64 2>/dev/null; ./zbxpatrol-macos-arm64 --help

## 许可与第三方声明

- 本项目代码以 **GNU AGPL-3.0** 许可发布（1.1.0 及更早版本使用旧的自定义非商业协议，见对应 tag 的 LICENSE）。
- **Zabbix 声明**：本工具是独立程序，仅通过 Zabbix 官方 JSON-RPC API 与 Zabbix 服务端交互；**不包含、不修改、不分发** Zabbix 任何源码或组件。Zabbix 本身采用 AGPL-3.0 授权，其源码与版权归属见官方仓库 <https://github.com/zabbix/zabbix>；"Zabbix" 为 Zabbix SIA 的商标，本项目与 Zabbix SIA 无任何关联。
- 第三方组件清单（SBOM，CycloneDX 格式）：[SBOM/zbxpatrol.cdx.json](SBOM/zbxpatrol.cdx.json)；摘要与 Zabbix 声明详见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。

## 开发

```bash
cargo test                 # 27 个单元测试（时间/聚合/评分/规则/重启检测，离线）
cargo clippy --all-targets # 零警告
cargo build --release
```

扩展：新指标 → `patrol.toml [metrics.rules]` 加正则；新评分规则 → `[[scoring]]`；新输出格式 → `crates/zbxpatrol-cli/src/render/` 加渲染器；新数据源 → core 的 `zabbix.rs` 抽象 trait 后接入。详细扩展性设计见设计文档第四部分。
