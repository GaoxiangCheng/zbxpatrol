# 自定义评分规则配置指南（中文）

> patrol.toml 完全可选——不使用此文件时使用内置默认评分。
> English version: [SCORING.en.md](SCORING.en.md)

<a id="toc"></a>
## 目录

1. [工作原理](#how)
2. [文件结构](#structure)
3. [自定义指标规则](#metrics)
4. [自定义评分规则](#scoring)
5. [指标路径参考](#paths)
6. [完整示例](#example)
7. [使用方法](#usage)

---

<a id="how"></a>
## 1. 工作原理

```
内置评分（默认）                    patrol.toml [[scoring]]
┌──────────────────────┐          ┌──────────────────────┐
│ 三档严格度             │          │ TOML 声明式规则       │
│ loose  = 基准90%      │          │ 完全替换内置评分       │
│ standard = 基准80%    │   或     │ 不受严格度影响        │
│ strict = 基准70%      │          │                      │
└──────────────────────┘          └──────────────────────┘
```

**关键规则**：`patrol.toml` 中一旦配置了 `[[scoring]]`，内置评分**全部被替换**（不再使用三档严格度）。只配 `[[metrics.rules]]` 时不影响评分。

[↑ 返回目录](#toc)

<a id="structure"></a>
## 2. 文件结构

```toml
# patrol.toml

# ─── 第一部分：自定义指标（可选）───
[metrics]
[[metrics.rules]]
id = "..."
pattern = "..."
label = "..."
unit = "%"
kind = "util | raw | event"

# ─── 第二部分：自定义评分（可选，配了就替换内置）───
[[scoring]]
metric = "..."
op = "..."
value = 85.0
points = 60
floor = false
message = "..."
per_count = false
```

[↑ 返回目录](#toc)

<a id="metrics"></a>
## 3. 自定义指标规则

将自定义 Zabbix 监控项接入报表（不影响评分）。

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | ✔ | 唯一标识（与内置规则同 id 则覆盖） |
| `pattern` | ✔ | key 正则表达式 |
| `label` | ✔ | 展示名称 |
| `unit` | | 单位（默认空） |
| `kind` | | 聚合方式：`util`（进主表）、`raw`（原始值，默认）、`event`（事件计数） |

```toml
[[metrics.rules]]
id = "jvm_heap"
pattern = '^jvm\.memory\.heap\.used\.pct$'
label = "JVM 堆使用率"
unit = "%"
kind = "util"

[[metrics.rules]]
id = "nginx_active"
pattern = '^nginx\.connections\.active$'
label = "Nginx 活跃连接"
kind = "raw"
```

[↑ 返回目录](#toc)

<a id="scoring"></a>
## 4. 自定义评分规则

完全替换内置评分。每条规则独立评估，命中则加分。

| 字段 | 必填 | 说明 |
|---|---|---|
| `metric` | ✔ | 指标路径（见[第5节](#paths)） |
| `op` | ✔ | 比较运算：`gt` / `gte` / `lt` / `lte` / `eq` |
| `value` | ✔ | 阈值 |
| `points` | ✔ | 命中加分 |
| `floor` | | 命中即置底 ≥95（直接严重），默认 `false` |
| `message` | | 描述文字，`{value}` 会被替换为实际值 |
| `per_count` | | 分值 × 命中值（用于「每次重启+15」类），默认 `false` |

**分数计算**：所有命中规则的分值累加，封顶 100。任何 `floor=true` 的规则命中 → 分数至少 95。

**等级映射**：0-39 健康 / 40-59 低危 / 60-74 中危 / 75-89 高危 / 90-100 严重。

[↑ 返回目录](#toc)

<a id="paths"></a>
## 5. 指标路径参考

| 路径 | 含义 | 数值类型 | 示例值 |
|---|---|---|---|
| `cpu.avg` | CPU 平均使用率 | % | 85.5 |
| `cpu.max` | CPU 峰值使用率 | % | 99.2 |
| `mem.avg` | 内存平均使用率 | % | 92.1 |
| `swap.avg` | Swap 平均使用率 | % | 55.0 |
| `disk.avg` | 最满分区平均使用率 | % | 91.3 |
| `inode.avg` | 最满 inode 使用率 | % | 88.0 |
| `load1.avg` | 1分钟平均负载 | 数值 | 3.5 |
| `offset.max` | 时间同步最大偏移 | 秒 | 45.0 |
| `zombies.cur` | 僵尸进程数 | 个 | 12 |
| `fd.cur` | 文件描述符使用率 | % | 85.0 |
| `cert.cur` | 证书剩余天数 | 天 | 15 |
| `icmp_loss` | ICMP 丢包率 | % | 2.5 |
| `bw_util.max` | 网卡带宽峰值利用率 | % | 95.0 |
| `reboots` | 区间内重启次数 | 次 | 2 |
| `oom` | OOM 事件次数 | 次 | 1 |
| `problems` | 未恢复高级别告警数 | 个 | 3 |
| `availability` | 主机可达（1=正常，0=不可达） | 0/1 | 0 |

[↑ 返回目录](#toc)

<a id="example"></a>
## 6. 完整示例

```toml
# patrol.toml — 自定义评分（替换内置）

# CPU 阈值从 85 降到 80
[[scoring]]
metric = "cpu.avg"
op = "gte"
value = 80.0
points = 60
message = "CPU 平均 {value}%，严重（≥80%）"

[[scoring]]
metric = "cpu.avg"
op = "gte"
value = 70.0
points = 45
message = "CPU 平均 {value}%，高危（≥70%）"

# 内存阈值调高到 95
[[scoring]]
metric = "mem.avg"
op = "gte"
value = 95.0
points = 60
message = "内存 {value}%，严重（≥95%）"

# 磁盘 95 即严重
[[scoring]]
metric = "disk.avg"
op = "gte"
value = 95.0
points = 60
floor = true
message = "磁盘 {value}%，严重"

# 每次重启扣 15 分
[[scoring]]
metric = "reboots"
op = "gte"
value = 1
points = 15
per_count = true
message = "区间内 {value} 次重启"

# 主机不可达直接严重
[[scoring]]
metric = "availability"
op = "eq"
value = 0
points = 95
floor = true
message = "主机不可达"
```

[↑ 返回目录](#toc)

<a id="usage"></a>
## 7. 使用方法

```bash
# CLI
zbxpatrol report --config patrol.toml --group <群组名>

# serve 自动加载当前目录
cd /opt/zbxpatrol && cp patrol.toml . && ./zbxpatrol serve
```

验证：`--data-json out.json` 后检查 `strictness` 字段和 `risk.points` 内容是否为自定义规则。

[↑ 返回目录](#toc) · [返回文档索引](README.md)
