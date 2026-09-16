# zbxpatrol Architecture & Key Code Notes (English)

> For maintainers and code review: module responsibilities, key algorithms, extension points, tests.
> 中文版: [CODE.zh.md](CODE.zh.md)

<a id="toc"></a>
## Contents

1. [Architecture & data flow](#arch)
2. [Core library modules](#core)
3. [Shell modules](#cli)
4. [Key algorithms](#algo) — [aggregation](#agg) · [trend weighting](#trend) · [bucketing](#bucket) · [counter deltas](#counter) · [reboot detection](#reboot) · [disk forecast](#forecast) · [3-tier scoring](#score) · [sparkline/chart](#spark)
5. [Networking & reliability](#net)
6. [Extension points](#ext)
7. [Tests & build](#tests)

---

<a id="arch"></a>
## 1. Architecture & data flow

```
Shell (zbxpatrol-cli): CLI (clap) | wizard (rustyline) | HTTP API (axum)
        └──────────── unified calls ────────────┘
Core (zbxpatrol-core):
  scope resolve → item discovery (rule engine) → fetch (history/trend) → aggregate
  → risk scoring → result model (types::ReportData)
Renderers: table / xlsx / json / csv / ascii-chart
```

- **Strict core/shell split**: all business logic lives in core; future ops platforms can depend on core in-process or call the HTTP API cross-process.
- One inspection = `run_inspection()`: per-host concurrency via JoinSet, range problems fetched once then correlated per host, scoring runs after problem correlation.

[↑ TOC](#toc)

<a id="core"></a>
## 2. Core modules (crates/zbxpatrol-core/src)

| Module | Responsibility |
|---|---|
| `env.rs` | Env loading; `~/.zbxpatrol/config.env` path |
| `errors.rs` | `PatrolError` (Config/MissingEnv/Auth/Network/Api) + exit-code mapping |
| `timerange.rs` | Parsing of the 3 time forms, timezone conversion, range formatting |
| `types.rs` | All data models (serde) — `ReportData`/`HostInspection`/`MetricStats`/`HostSpark`; the JSON contract is additive-only |
| `zabbix.rs` | JSON-RPC client: Bearer auth, re-login, exponential-backoff retry, token-bucket rate limit, batched fetches |
| `rules.rs` | Item identification rules (key regex → category/unit/aggregation), legacy+modern disk templates & Windows keys; instance-name extraction |
| `discovery.rs` | Scope resolution, per-host item classification, OS family derivation |
| `metrics.rs` | Pure algorithm library: aggregate/bucket/delta/regression/wildcard (fully unit-tested) |
| `scoring.rs` | 3-tier strictness engine + TOML-declarative custom rules |
| `patrol_config.rs` | Optional `patrol.toml` parsing (metric rules / scoring rules) |
| `problems.rs` | Severity labels & summary |
| `pipeline.rs` | Orchestration: `run_inspection` / `run_query` / `host_metric_series` |

[↑ TOC](#toc)

<a id="cli"></a>
## 3. Shell modules (crates/zbxpatrol-cli/src)

| Module | Responsibility |
|---|---|
| `main.rs` | clap definitions (layered help/examples), 3-layer config load, exit codes |
| `actions.rs` | Subcommand implementations (shared by CLI/wizard/serve); shell completion scripts |
| `render/mod.rs` | Table/JSON/CSV/sparkline/ASCII chart |
| `render/xlsx.rs` | Excel report (9+ sheets, conditional formatting) |
| `wizard.rs` | Interactive wizard: rustyline completion, level menus, session caches |
| `serve.rs` | axum HTTP API (Bearer auth, session reuse) |

[↑ TOC](#toc)

<a id="algo"></a>
## 4. Key algorithms (metrics.rs / scoring.rs — pure, unit-tested)

<a id="agg"></a>
### 4.1 Aggregation
- Current value: `lastvalue` from `item.get` (batched, zero extra cost);
- ≤1 day: full `history.get` avg/max/min (fallback to trend when empty);
- >1 day: trend (below).

<a id="trend"></a>
### 4.2 Trend-weighted average
`avg = Σ(avgᵢ × numᵢ) / Σ(numᵢ)`; `max = max(maxᵢ)`, `min = min(minᵢ)` — hourly trend maxima are true peaks, so results match raw history.

<a id="bucket"></a>
### 4.3 Bucketed series (sparklines/charts)
`bucket_series(samples, from, till, N)`: N equal spans, per-span mean, `None` when empty (rendered as blanks). Reports use 48 buckets, queries 32, big charts 72.

<a id="counter"></a>
### 4.4 Counter deltas
Cumulative counters (NIC bytes etc.): adjacent `Δv/Δt` for rates; errors/drops as range increments, **skipping the first window after a wrap (reboot reset)**; rate items with `units=bps` are aggregated directly and ÷1e6 → Mbps.

<a id="reboot"></a>
### 4.5 Reboot detection
Prefer `system.boottime` distinct-count − 1 (unique per boot); fallback: downward jumps of `system.uptime`.

<a id="forecast"></a>
### 4.6 Disk-full forecast (ranges > 7 days)
Least-squares regression on the fullest partition's hourly pused → %/day slope; `(100 − current%)/slope` = days until full; scored only when ≤ 90 days.

<a id="score"></a>
### 4.7 Three-tier scoring
Baseline B (loose 90 / standard 80 / strict 70) is the high-risk line:
- CPU tiers `[min(B+5,98), B−5, B−15, B−30]`; memory `[min(B+10,98), B, B−10, B−20]`; disk/inode `[min(B+10,98), B+5, B, B−5]`;
- peak/swap/bandwidth/fd follow `min(B+15,99)/B−30/B/B`;
- event rules (reboots/OOM/certs/alerts) do not scale with B; unreachable / failed service ⇒ floor ≥ 95;
- sum capped at 100; levels 0-39 healthy / 40-59 low / 60-74 medium / 75-89 high / 90-100 critical.

<a id="spark"></a>
### 4.8 Sparkline / ASCII chart
- `sparkline`: 8-level Unicode blocks `▁▂▃▅▆█`, min-max normalized;
- `ascii_chart`: 14 rows, Y-axis scale, `┄` average line, time axis, blanks for missing columns.

[↑ TOC](#toc)

<a id="net"></a>
## 5. Networking & reliability

- **Auth**: `user.login` → Bearer header (Zabbix 7.x removed body auth); auto re-login once on auth failure (recursion boxed);
- **Retry**: exponential backoff ×2 on network errors/5xx/429; JSON-RPC business errors are not retried;
- **Throttling**: token-bucket interval (default 10 req/s) + `PATROL_CONCURRENCY` (default 8, JoinSet + semaphore);
- **DNS**: hickory pure-Rust resolver (rustls ecosystem) — fixes musl static binaries vs some internal DNS servers (verified in the field); full error-chain output with an /etc/hosts fallback hint;
- **Fault tolerance**: per-host/item failures mark "data missing" without aborting the report (exit code 4).

[↑ TOC](#toc)

<a id="ext"></a>
## 6. Extension points

| Extension | How |
|---|---|
| New metric | one regex line in `patrol.toml [metrics.rules]` (no code change) |
| Custom scoring | `patrol.toml [[scoring]]` (metric path + op + threshold + points; replaces built-ins entirely when present) |
| New output | add a renderer under `render/` (input is always ReportData) |
| New data source | abstract the zabbix client to add multi-instance/Prometheus (roadmap) |
| Platform integration | depend on the core crate in-process, or call the HTTP API cross-process |

[↑ TOC](#toc)

<a id="tests"></a>
## 7. Tests & build

- **24 unit tests** (`cargo test`): time parsing/timezone, history aggregation, trend weighting, bucketing & inversion, counter deltas & wraps, reboot detection, regression & forecast, per-tier threshold conformance & behavior, rule regexes (real-world key samples), OS derivation, TOML parsing;
- **Quality gates**: `cargo fmt`, `cargo clippy --all-targets -D warnings` — zero warnings;
- **Build**: `cargo build --release` (native); Linux artifacts via `cargo zigbuild --release --target x86_64-unknown-linux-musl` (or aarch64); container build in `deploy/Dockerfile`.

[↑ TOC](#toc) · [Doc index](README.md)
