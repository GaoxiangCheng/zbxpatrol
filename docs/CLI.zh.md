# zbxpatrol 命令行（CLI）参考文档（中文）

> 非交互全量能力：可脚本化、可 cron、供程序调用。与交互向导功能对等。
> English version: [CLI.en.md](CLI.en.md)

<a id="toc"></a>
## 目录

1. [安装与配置](#setup)（三层配置优先级）
2. [全局参数与退出码](#global)
3. [子命令详解](#cmds)
   - [check](#check) · [groups](#groups) · [items](#items) · [query](#query)
   - [chart](#chart) · [report](#report) · [serve](#serve) · [completions](#completions) · [__complete](#dunder)
4. [时间参数格式](#time) · [范围参数与通配符](#scope)
5. [Shell 补全安装](#bash)（多级 Tab 补全真实数据）
6. [定时任务与部署](#cron)

---

<a id="setup"></a>
## 1. 安装与配置

单二进制，无运行时依赖。放置任意目录 `chmod +x` 即用（产物见 `dist/`：Linux x86_64/aarch64 musl 静态、macOS arm64）。

**配置三层来源（优先级高→低，均不覆盖更高层）**：

1. 进程环境变量：`ZBX_URL` / `ZBX_USER` / `ZBX_PASSWORD`（必填项）；`ZBX_TIMEOUT`(30s)、`ZBX_INSECURE`(false)、`ZBX_TZ`(Asia/Shanghai)、`PATROL_CONCURRENCY`(8)、`PATROL_RATE_LIMIT`(10/s) 可选；
2. 当前目录 `.env`（项目/部署目录级）；
3. `~/.zbxpatrol/config.env`（交互式初始化自动生成，600 权限）。

> 终端首次运行任意命令可交互初始化并写入第 3 层；非交互环境缺配置直接退出码 2，绝不挂起。

[↑ 返回目录](#toc)

<a id="global"></a>
## 2. 全局参数与退出码

| 参数 | 说明 |
|---|---|
| `--format table\|json\|csv` | 输出格式（默认 table；json/csv 供程序解析，数据走 stdout） |
| `--quiet` | 静默（不输出进度） |
| `--no-interactive` | 禁用交互（非 TTY 自动生效） |
| `--verbose` | 调试日志（日志一律走 stderr） |
| `-h/--help` | 帮助（各子命令均附示例；示例中尖括号为变量） |

**退出码**：`0` 成功 · `2` 配置/凭据错误 · `3` 网络/API 错误 · `4` 部分数据缺失（报表仍完整生成）。

[↑ 返回目录](#toc)

<a id="cmds"></a>
## 3. 子命令详解

<a id="check"></a>
### 3.1 check — 连通性/凭据/权限自检

```bash
zbxpatrol check
```
依次验证：地址连通（证书问题时提示 ZBX_INSECURE）→ 登录 → 数据读取权限；每步 ✓/✗ 与结论。

<a id="groups"></a>
### 3.2 groups — 群组列表

```bash
zbxpatrol groups [--search <子串>] [--page N] [--size N]
```
`--search` 名称过滤；`--page`（1 起，缺省 1）+ `--size` 分页（只给 `--size` 即第 1 页）。

<a id="hosts"></a>
### 3.3 items — 监控项清单

```bash
zbxpatrol items [--host <主机名>|--group <群组名>] [--search <子串>] [--detail]
```
默认按 key 聚合（key/名称/单位/类型/覆盖主机数）；`--detail` 需配合 `--host` 逐条列出（含当前值）。

<a id="query"></a>
### 3.4 query — 任意监控项统计

```bash
zbxpatrol query --key <K> [--key <K2>…] [范围] [时间] [--csv <文件>] [--chart] [--format json]
```
- key 支持 `*` 通配（如 `net.if*`）；可重复 `--key` 或逗号分隔；
- `--chart`：表格输出后，对**唯一命中**的系列追加全尺寸趋势图（72 列，Y 轴刻度+均值线）；命中多个系列时退出码 2 并提示用 `--host`/精确 `--key` 收窄；与 `--format json/csv` 互斥；
- 输出 当前/平均/最大/最小 + **趋势火花线**（命中 ≤30 项时）；
- `--csv` 另存 CSV（UTF-8 BOM，Excel 直开）。

<a id="chart"></a>
### 3.5 chart — 单主机趋势图（ASCII）

```bash
zbxpatrol chart --host <主机名> --metric cpu|mem|disk [时间]
zbxpatrol chart --host <主机名> --key <监控项key或通配> [时间]   # 任意监控项，通配须唯一命中
```
Y 轴刻度、均值虚线、时间轴、最小/平均/最大摘要；`--format json` 输出序列数据。

<a id="report"></a>
### 3.6 report — 巡检报表（核心）

```bash
zbxpatrol report [范围] [时间] [--strictness loose|standard|strict]
                 [--keys <k1,k2,…>] [--all-items]
                 [--data-json <文件>] [--out <目录>] [--config patrol.toml]
```
- 范围：`--group`（可多次）/ `--host` / `--hosts a,b,c`（缺省全部主机）；
- 时间：见[第 4 节](#time)；
- `--strictness`：宽松(基准90%)/标准(80%)/严格(70%)；
- `--keys`：附加自定义指标（通配支持）→ Excel「自定义指标明细」sheet + JSON `extra` 字段；
- `--all-items`：附加全部数值指标 sheet；
- 输出：xlsx（`巡检报告_<起>-<止>_<范围>.xlsx`，条件格式着色）+ 控制台彩色明细表（`--format table` 默认）；`--format csv` 输出平面 CSV；`--data-json` 另存结构化 JSON。

<a id="serve"></a>
### 3.7 serve — 本地 HTTP API

```bash
zbxpatrol serve --listen 127.0.0.1:8787 [--token <TOKEN>]
```
详见 [API 文档](API.zh.md)。

<a id="completions"></a>
### 3.8 completions — 生成 Shell 补全脚本

```bash
zbxpatrol completions bash | sudo tee /etc/bash_completion.d/zbxpatrol
source <(zbxpatrol completions bash)      # 临时生效
zbxpatrol completions zsh > ~/.zfunc/_zbxpatrol
```

<a id="dunder"></a>
### 3.9 __complete（隐藏）— 补全数据源

```bash
zbxpatrol __complete groups              # 群组名
zbxpatrol __complete hosts               # 主机名
zbxpatrol __complete items --host <主机名> # 该主机数值监控项 key
```
供补全脚本调用；失败静默（输出空）。

[↑ 返回目录](#toc)

<a id="time"></a>
## 4. 时间参数（三选一，优先级从高到低）

```bash
--from "2026-09-01[ 08:00:00]" [--to "…"]   # 绝对区间（to 缺省=现在）
--last 48h|15d|12M|30m                       # 相对时长
--period day|week|month|year                 # 预设周期（默认 day）
```

<a id="scope"></a>
### 范围参数与通配符

- `--group <群组名>` 可多次；`--host <主机名>` 单台；`--hosts a,b,c` 多台；缺省全部主机；
- key 通配：`*` 任意段（`vfs.fs*pused*`），严格区分大小写；主机名需与 Zabbix 完全一致（补全可避免手误）。

[↑ 返回目录](#toc)

<a id="bash"></a>
## 5. Shell 补全（多级 Tab）

安装 [3.9](#completions) 后：

| 输入阶段 | Tab 效果 |
|---|---|
| `zbxpatrol <TAB>` | 子命令（前缀过滤） |
| `zbxpatrol items <TAB>` | 该子命令全部选项 |
| `items --group <TAB>` | **真实群组名**（来自 Zabbix） |
| `--group <值> <TAB>` | 继续给剩余选项（可继续下一项） |
| `chart --host <主机> --key <TAB>` | **该主机真实监控项 key**（前缀过滤） |
| `--metric/--period/--strictness/--format <TAB>` | 枚举值 |

[↑ 返回目录](#toc)

<a id="cron"></a>
## 6. 定时任务与部署

```bash
# crontab（示例见 deploy/crontab.example）
0 8 * * * /opt/zbxpatrol/zbxpatrol report --period day --quiet >> /var/log/zbxpatrol.log 2>&1
30 8 * * 1 /opt/zbxpatrol/zbxpatrol report --group <群组名> --period week --quiet >> /var/log/zbxpatrol.log 2>&1
```

systemd timer / 常驻 API / Docker 见 `deploy/` 与 [README](../README.md)。

[↑ 返回目录](#toc) · [返回文档索引](README.md)
