# zbxpatrol 关键代码与架构说明（中文）

> 面向二次开发/代码评审：模块职责、关键算法、扩展点、测试。
> English version: [CODE.en.md](CODE.en.md)

<a id="toc"></a>
## 目录

1. [总体架构与数据流](#arch)
2. [核心库模块（zbxpatrol-core）](#core)
3. [壳模块（zbxpatrol-cli）](#cli)
4. [关键算法](#algo)
   - [统计聚合](#agg) · [趋势加权](#trend) · [分桶序列](#bucket) · [计数器差分](#counter)
   - [重启检测](#reboot) · [满盘预测](#forecast) · [三档评分](#score) · [火花线/ASCII 图](#spark)
5. [网络与可靠性](#net)（认证/重试/限速/并发/DNS）
6. [扩展点](#ext)（指标/评分/输出/数据源/平台化）
7. [测试与构建](#tests)

---

<a id="arch"></a>
## 1. 总体架构与数据流

```
壳层（zbxpatrol-cli）：CLI(clap) | 交互向导(rustyline) | HTTP 服务(axum)
        └────────── 统一调用 ──────────┘
核心层（zbxpatrol-core）：
  范围解析 → 监控项发现(规则引擎) → 取数(history/trend) → 聚合统计
  → 风险评分 → 结果模型(types::ReportData)
输出层（渲染器）：table / xlsx / json / csv / ascii-chart
```

- **core 与壳严格分离**：全部业务逻辑在 core，未来运维平台可同进程依赖 core 或跨进程调用 HTTP API；
- 一次巡检 = `run_inspection()`：JoinSet 并发逐主机，区间问题一次性拉取后按主机关联，评分在问题关联后执行。

[↑ 返回目录](#toc)

<a id="core"></a>
## 2. 核心库模块（crates/zbxpatrol-core/src）

| 模块 | 职责 |
|---|---|
| `env.rs` | 环境变量加载、`~/.zbxpatrol/config.env` 路径 |
| `errors.rs` | 统一错误 `PatrolError`（Config/MissingEnv/Auth/Network/Api）与退出码映射 |
| `timerange.rs` | 三种时间写法解析、时区换算、区间格式化（文件名/展示） |
| `types.rs` | 全部数据模型（serde）——`ReportData`/`HostInspection`/`MetricStats`/`HostSpark` 等，JSON 即接口契约（只增不改） |
| `zabbix.rs` | JSON-RPC 客户端：Bearer 认证、重登、指数退避重试、令牌桶限速、分批取数 |
| `rules.rs` | 指标识别规则（key 正则→类别/单位/聚合方式），含新旧磁盘模板/Windows 适配；实例名提取 |
| `discovery.rs` | 范围解析（all/group/hosts）、逐主机 item 归类、OS 类型推导 |
| `metrics.rs` | 纯函数算法库：聚合/分桶/差分/回归/通配（全部有单测） |
| `scoring.rs` | 三档严格度评分引擎 + TOML 声明式自定义规则 |
| `patrol_config.rs` | 可选 `patrol.toml` 解析（指标规则/评分规则） |
| `problems.rs` | 严重级别中文映射与统计 |
| `pipeline.rs` | 编排：`run_inspection` / `run_query` / `host_metric_series` |

[↑ 返回目录](#toc)

<a id="cli"></a>
## 3. 壳模块（crates/zbxpatrol-cli/src）

| 模块 | 职责 |
|---|---|
| `main.rs` | clap 定义（分层帮助/示例）、配置三层加载、退出码 |
| `actions.rs` | 各子命令实现（CLI/向导/serve 共用）；shell 补全脚本生成 |
| `render/mod.rs` | 表格/JSON/CSV/火花线/ASCII 趋势图 |
| `render/xlsx.rs` | Excel 报表（9+ sheet，条件格式） |
| `wizard.rs` | 交互向导：rustyline 补全、层级菜单、会话缓存 |
| `serve.rs` | axum HTTP API（Bearer 鉴权、会话复用） |

[↑ 返回目录](#toc)

<a id="algo"></a>
## 4. 关键算法（均在 metrics.rs / scoring.rs，纯函数、可单测）

<a id="agg"></a>
### 4.1 统计聚合
- 当前值：`item.get` 的 `lastvalue`（批量、零额外开销）；
- ≤1 天：`history.get` 全量 avg/max/min（无历史回退 trend）；
- >1 天：走 trend（见下）。

<a id="trend"></a>
### 4.2 趋势加权平均
`avg = Σ(avgᵢ × numᵢ) / Σ(numᵢ)`；`max = max(maxᵢ)`、`min = min(minᵢ)`——trend 的小时 max 即真实峰值，结果与原始 history 一致。

<a id="bucket"></a>
### 4.3 分桶序列（火花线/图表）
`bucket_series(samples, from, till, N)`：区间等分 N 段，段内均值，无样本为 `None`（渲染为空格）。报表 48 桶、查询 32 桶、大图 72 桶。

<a id="counter"></a>
### 4.4 计数器差分
网卡字节数等累计计数器：相邻样本 `Δv/Δt` 得速率；错包/丢包取区间新增量，**回绕（重启清零）后的首个窗口跳过**；`units=bps` 的速率型 item 直接统计并 ÷1e6 折算 Mbps。

<a id="reboot"></a>
### 4.5 重启检测
优先 `system.boottime` 去重计数−1（每次开机值唯一）；无 boottime 用 `system.uptime` 序列向下跳变计数。

<a id="forecast"></a>
### 4.6 满盘预测（区间 >7 天）
对最满分区的 pused 小时序列做**最小二乘回归**求 %/天 斜率，`(100−当前%)/斜率` 得预计满盘天数；仅 ≤90 天时计分。

<a id="score"></a>
### 4.7 三档评分
基准 B（宽松90/标准80/严格70）即高危线：
- CPU 阶梯 `[min(B+5,98), B−5, B−15, B−30]`；内存 `[min(B+10,98), B, B−10, B−20]`；磁盘/inode `[min(B+10,98), B+5, B, B−5]`；
- 峰值/swap/带宽/fd 等联动 `min(B+15,99)/B−30/B/B`；
- 事件类（重启/OOM/证书/告警）不随 B 缩放；不可达或服务失败直接 ≥95；
- 累加封顶 100；等级 0-39 健康/40-59 低危/60-74 中危/75-89 高危/90-100 严重。

<a id="spark"></a>
### 4.8 火花线 / ASCII 图
- `sparkline`：8 级 Unicode 块 `▁▂▃▅▆█`，按序列 min-max 归一；
- `ascii_chart`：14 行高度、Y 轴刻度、`┄` 均值线、时间轴，无数据列留空。

[↑ 返回目录](#toc)

<a id="net"></a>
## 5. 网络与可靠性

- **认证**：`user.login` → Bearer 头（Zabbix 7.x 已移除 body auth 字段）；认证失效自动重登一次（async 递归已装箱）；
- **重试**：网络错误/5xx/429 指数退避重试 2 次；JSON-RPC 业务错误不重试；
- **限速**：令牌间隔（默认 10 req/s）+ `PATROL_CONCURRENCY` 并发（默认 8，JoinSet + 信号量）；
- **DNS**：启用 hickory 纯 Rust 解析器（rustls 生态）——解决 musl 静态二进制与部分内网 DNS 不兼容的问题（已实测内网环境）；错误链完整输出根因并给出 /etc/hosts 兜底提示；
- **容错**：单主机/单指标失败标注「数据缺失」不中断整份报表（退出码 4）。

[↑ 返回目录](#toc)

<a id="ext"></a>
## 6. 扩展点

| 扩展 | 方式 |
|---|---|
| 新指标 | `patrol.toml [metrics.rules]` 加一行 key 正则（无需改代码） |
| 自定义评分 | `patrol.toml [[scoring]]`（metric 路径 + op + 阈值 + 分值，存在即整体替换内置） |
| 新输出格式 | `render/` 增加渲染器（输入统一 ReportData） |
| 新数据源 | core 的 zabbix 客户端抽象后接入多实例/Prometheus（演进中） |
| 平台集成 | 同进程依赖 core crate，或跨进程走 HTTP API |

[↑ 返回目录](#toc)

<a id="tests"></a>
## 7. 测试与构建

- **单元测试 24 项**（`cargo test`）：时间解析/时区、history 聚合、trend 加权、分桶与取反、计数器差分与回绕、重启检测、回归与满盘预测、三档阈值逐项对照与行为、规则正则（真实环境 key 样本）、OS 推导、TOML 解析；
- **质量门**：`cargo fmt`、`cargo clippy --all-targets -D warnings` 零警告；
- **构建**：`cargo build --release`（本机）；Linux 产物 `cargo zigbuild --release --target x86_64-unknown-linux-musl`（或 aarch64），`deploy/Dockerfile` 提供容器构建。

[↑ 返回目录](#toc) · [返回文档索引](README.md)
