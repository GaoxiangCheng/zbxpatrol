# Custom Scoring Rules Guide (English)

> patrol.toml is fully optional — built-in scoring is used when absent.
> 中文版: [SCORING.zh.md](SCORING.zh.md)

<a id="toc"></a>
## Contents

1. [How It Works](#how)
2. [File Structure](#structure)
3. [Custom Metric Rules](#metrics)
4. [Custom Scoring Rules](#scoring)
5. [Metric Path Reference](#paths)
6. [Complete Example](#example)
7. [Usage](#usage)

---

<a id="how"></a>
## 1. How It Works

```
Built-in scoring (default)              patrol.toml [[scoring]]
┌──────────────────────┐               ┌──────────────────────┐
│ 3 strictness tiers   │               │ Declarative rules     │
│ loose  = base 90%    │    OR         │ Fully replaces built-in│
│ standard = base 80%  │               │ Ignores strictness    │
│ strict = base 70%    │               │                       │
└──────────────────────┘               └──────────────────────┘
```

**Key rule**: Once `[[scoring]]` is configured in patrol.toml, the built-in scoring is **entirely replaced**. Configuring only `[[metrics.rules]]` does not affect scoring.

[↑ TOC](#toc)

<a id="structure"></a>
## 2. File Structure

```toml
# patrol.toml

# ─── Part 1: Custom metrics (optional) ───
[metrics]
[[metrics.rules]]
id = "..."
pattern = "..."
label = "..."
unit = "%"
kind = "util | raw | event"

# ─── Part 2: Custom scoring (optional, replaces built-in) ───
[[scoring]]
metric = "..."
op = "..."
value = 85.0
points = 60
floor = false
message = "..."
per_count = false
```

[↑ TOC](#toc)

<a id="metrics"></a>
## 3. Custom Metric Rules

Integrate custom Zabbix items into reports (does not affect scoring).

| Field | Required | Description |
|---|---|---|
| `id` | ✔ | Unique ID (overrides built-in if same id) |
| `pattern` | ✔ | Key regex |
| `label` | ✔ | Display name |
| `unit` | | Unit (default: empty) |
| `kind` | | Aggregation: `util` (main table), `raw` (default), `event` (count) |

```toml
[[metrics.rules]]
id = "jvm_heap"
pattern = '^jvm\.memory\.heap\.used\.pct$'
label = "JVM Heap Usage"
unit = "%"
kind = "util"
```

[↑ TOC](#toc)

<a id="scoring"></a>
## 4. Custom Scoring Rules

Fully replaces built-in scoring. Each rule is evaluated independently.

| Field | Required | Description |
|---|---|---|
| `metric` | ✔ | Metric path (see [Section 5](#paths)) |
| `op` | ✔ | Comparison: `gt` / `gte` / `lt` / `lte` / `eq` |
| `value` | ✔ | Threshold |
| `points` | ✔ | Points added on match |
| `floor` | | Match → minimum score 95 (critical), default `false` |
| `message` | | Description; `{value}` replaced with actual value |
| `per_count` | | points × matched value (for "per reboot +15"), default `false` |

**Score calculation**: All matched rules sum up, capped at 100. Any `floor=true` rule → score at least 95.

**Level mapping**: 0-39 Healthy / 40-59 Low / 60-74 Medium / 75-89 High / 90-100 Critical.

[↑ TOC](#toc)

<a id="paths"></a>
## 5. Metric Path Reference

| Path | Meaning | Unit | Example |
|---|---|---|---|
| `cpu.avg` | CPU avg utilization | % | 85.5 |
| `cpu.max` | CPU peak | % | 99.2 |
| `mem.avg` | Memory avg | % | 92.1 |
| `swap.avg` | Swap avg | % | 55.0 |
| `disk.avg` | Fullest partition avg | % | 91.3 |
| `inode.avg` | Fullest inode avg | % | 88.0 |
| `load1.avg` | 1-min load average | value | 3.5 |
| `offset.max` | Max clock offset | seconds | 45.0 |
| `zombies.cur` | Zombie processes | count | 12 |
| `fd.cur` | File descriptor usage | % | 85.0 |
| `cert.cur` | Certificate days remaining | days | 15 |
| `icmp_loss` | ICMP packet loss | % | 2.5 |
| `bw_util.max` | Peak bandwidth utilization | % | 95.0 |
| `reboots` | Reboots in range | count | 2 |
| `oom` | OOM events | count | 1 |
| `problems` | Unresolved high-severity alerts | count | 3 |
| `availability` | Host reachable (1=up, 0=down) | 0/1 | 0 |

[↑ TOC](#toc)

<a id="example"></a>
## 6. Complete Example

```toml
# patrol.toml — Custom scoring (replaces built-in)

[[scoring]]
metric = "cpu.avg"
op = "gte"
value = 80.0
points = 60
message = "CPU avg {value}%, critical (>=80%)"

[[scoring]]
metric = "mem.avg"
op = "gte"
value = 95.0
points = 60
message = "Memory {value}%, critical (>=95%)"

[[scoring]]
metric = "disk.avg"
op = "gte"
value = 95.0
points = 60
floor = true
message = "Disk {value}%, critical"

[[scoring]]
metric = "reboots"
op = "gte"
value = 1
points = 15
per_count = true
message = "{value} reboots in range"

[[scoring]]
metric = "availability"
op = "eq"
value = 0
points = 95
floor = true
message = "Host unreachable"
```

[↑ TOC](#toc)

<a id="usage"></a>
## 7. Usage

```bash
# CLI
zbxpatrol report --config patrol.toml --group mygroup

# serve auto-loads from working directory
cd /opt/zbxpatrol && cp patrol.toml . && ./zbxpatrol serve
```

Verify: Use `--data-json out.json` and check the `strictness` field and `risk.points` content.

[↑ TOC](#toc) · [Doc index](README.md)
