# zbxpatrol — Zabbix Server Inspection & Reporting

**English (default)** | [简体中文](README.zh.md)

Automated inspection for all servers monitored by Zabbix (6.x–7.4, verified on 7.4.14): generates **risk-scored** Excel reports plus structured data (JSON/CSV/HTTP API) for third-party programs (ops platforms). Single static binary, no runtime dependencies, ready for unattended deployment on any server.

- **Full documentation (EN default + 中文)**: [docs/README.md](docs/README.md) — API reference / interactive wizard / CLI reference / architecture & key code, with multi-level anchor TOCs.
- Architecture: `crates/zbxpatrol-core` (all business logic, reusable by your platform) + `crates/zbxpatrol-cli` (CLI / wizard / renderers / HTTP shell).

## Quick Start (3 steps)

**No environment variables needed up front** — run any command in a terminal (e.g. `./zbxpatrol` or `./zbxpatrol check`); when no config is found you enter interactive setup: URL, username, password (hidden), skip-TLS choice, timezone. After connectivity is verified it is saved to **`~/.zbxpatrol/config.env`** (mode 600) and every later command just works.

```bash
# 1. first run in a terminal — answer the prompts
./zbxpatrol check

# 2. generate a report (all hosts + last 24h by default, output to ./reports/)
./zbxpatrol report
```

Run `./zbxpatrol` without a subcommand to enter the **interactive wizard**: pick with numbers/letters, **`?` shows valid inputs per level** (the gray hint line re-prints after help), **`q` goes back, `exit`/`quit` leaves the program**, `m`/`g`/`h` jump across levels (main/groups/hosts), empty Enter re-prompts silently; the prompt carries the **level path prefix** (e.g. `[main>browse><group>><host>]`), and **Tab completion** offers data items only (group/host/item-key names, max 12 shown, unique completes) with a gray "input:" hint line above every prompt.

- **Smart item input**: type a key (fragment/wildcard) in a host context — **unique match executes immediately; multiple matches open a picker** (e.g. `load` lists all load items);
- **Hierarchical browse** (menu 1): groups → hosts → metrics; lists are **cached per session**, filterable (`f` or just type), paged (`n`/`p`); after picking a host you **stay in its context** — items, trend charts (preset or **custom items**, list-pick or smart input), quick daily inspection, key queries;
- **Report flow**: scope → time → strictness → **extra custom items** → output (console / Excel / **CSV file** / **JSON file**, combinable);
- **Metric query**: pick keys from the list or type wildcards; consecutive queries keep the same scope;
- Connectivity self-check.

Non-interactive CLI is feature-equivalent (`query --chart`, `report --keys`, `--from/--to` custom ranges…); every subcommand's `-h` ships examples.

Inspection results can also be viewed directly in the console as a colored table (sorted by risk, green/yellow/red utilization, **OS column** and **CPU/memory trend sparklines** ▁▂▃▅▆█):

```bash
zbxpatrol report --group <group>          # xlsx + console detail table (default with --format table)
```

## Command Overview

Angle-bracket tokens in examples (`<group>`, `<host>`, `<item-key>`) are **variables** — replace with real values (shell completion fills them via Tab).

```bash
zbxpatrol                            # interactive wizard (TTY only)
zbxpatrol check                      # self-check: connectivity/credentials/version/permissions
zbxpatrol serve --listen 127.0.0.1:8787 [--token XXX]   # local HTTP API
zbxpatrol groups [--search <substr>] [--size N --page N]   # list groups (filter + paginate)
zbxpatrol items [--host H|--group G] [--search cpu] [--detail]   # item catalog
zbxpatrol query --key "system.cpu.util" --last 7d       # stats for any item (with trend sparkline)
zbxpatrol query --key "net.if*" --group "Web" --csv out.csv
zbxpatrol query --host <host> --key system.cpu.util --chart   # table + full-size plot of the single matched series

zbxpatrol report --group <group> --keys 'net.if*,proc.num'   # report + custom item sheet

# inspection reports
zbxpatrol report                                     # all hosts, daily
zbxpatrol report --group "<group>"                   # by group (--group repeatable)
zbxpatrol report --host <host>                       # single host
zbxpatrol report --hosts a,b,c                       # multiple hosts
zbxpatrol report --period week|month|year            # weekly/monthly/yearly (yearly adds disk-full forecast)
zbxpatrol report --last 48h                          # any duration (h/d/M/m)
zbxpatrol report --from "2026-09-01" --to "2026-09-15 23:59"   # absolute range
zbxpatrol report --group "<group>" --period week --strictness strict   # strict scoring
zbxpatrol report --group X --all-items --data-json out.json    # all items + structured data
```

Common flags: `--format table|json|csv` (json/csv for programs), `--quiet`, `--no-interactive`, `--config patrol.toml`, `--out <dir>`.

**Shell completion** (subcommands/options; `--group`/`--host` complete **real** Zabbix group/host names):

```bash
zbxpatrol completions bash | sudo tee /etc/bash_completion.d/zbxpatrol   # bash, permanent
source <(zbxpatrol completions bash)                                     # session-only
zbxpatrol completions zsh > ~/.zfunc/_zbxpatrol                          # zsh
```
After install: `zbxpatrol <TAB>` lists subcommands; `items --group <TAB>` real group names; `--host <TAB>` real host names; `query --chart <TAB>` plot flag; enum options complete too. `-h` output is grouped (Options / Time / Scope).

**Exit codes**: 0 OK / 2 config or credentials / 3 network or API / 4 partial data missing (report still generated). Data on stdout, logs on stderr; interaction auto-disables outside a TTY.

## Configuration

Three layers, priority high → low (a higher layer wins):

1. **Process env vars**: `ZBX_URL` / `ZBX_USER` / `ZBX_PASSWORD` (for CI, cron, systemd)
2. **`./.env`** in the working directory (project/deploy level)
3. **`~/.zbxpatrol/config.env`** (user level, generated by interactive setup, mode 600)

Non-interactive runs missing config exit with code 2 immediately — they never hang.

| Var | Required | Default | Description |
|---|---|---|---|
| `ZBX_URL` | ✔ | — | Zabbix frontend URL |
| `ZBX_USER` / `ZBX_PASSWORD` | ✔ | — | API account (API access enabled) |
| `ZBX_TIMEOUT` | | 30 | request timeout (s) |
| `ZBX_INSECURE` | | false | skip TLS verification |
| `ZBX_TZ` | | Asia/Shanghai | timezone |
| `PATROL_CONCURRENCY` | | 8 | fetch concurrency |
| `PATROL_RATE_LIMIT` | | 10 | API rate limit (req/s) |

## Report Contents

Excel (`巡检报告_<from>-<to>_<scope>.xlsx`, conditional green/yellow/red):

1. **Overview**: range, availability, risk distribution, TOP10, conclusions
2. **Host details**: CPU/memory/disk(fullest)/swap/inode each **cur/avg/max/min** + load + risk score/level/findings
3. **Partitions**: capacity, space%, inode%, days-to-full (ranges >7d)
4. **Network & services**: NIC bandwidth (Mbps), utilization, errors/drops, port/service probes
5. **Stability & security**: reboots, clock offset, zombies, fd usage, certificate days
6. **Problems & alerts** in range
7. **Scoring notes**: thresholds & weights
8. **All items** (`--all-items`)

**Risk scoring** (0–100) with **three strictness tiers**: `--strictness loose|standard|strict` (default standard; wizard & API too). The baseline is the high-risk line — **loose 90% / standard 80% / strict 70%**; all percentage thresholds shift with it:

| Tiers (critical/high/medium/low) | Loose (90) | Standard (80) | Strict (70) |
|---|---|---|---|
| CPU | 95/85/75/60 | 85/75/65/50 | 75/65/55/40 |
| Memory | 98/90/80/70 | 90/80/70/60 | 80/70/60/50 |
| Disk/inode | 98/95/90/85 | 90/85/80/75 | 80/75/70/65 |
| CPU peak / swap / bandwidth,fd | 99 / 60 / 90 | 95 / 50 / 80 | 85 / 40 / 70 |

Unreachable/failed services are critical outright; event rules (reboots, OOM, zombies, clock offset, certificates, high alerts) do not scale with the baseline. Levels: 0–39 healthy / 40–59 low / 60–74 medium / 75–89 high / 90–100 critical. The active mode and thresholds are recorded in the report and JSON (`strictness`). Custom `[[scoring]]` rules in `patrol.toml` replace the built-ins entirely (see `patrol.example.toml`).

## Metrics (environment-adaptive)

Auto-discovered by key regex — **present → enabled, absent → no placeholder**: CPU (util/load/cores), memory, swap, disks (space+inode, legacy & modern templates, Windows partitions), NIC traffic/errors/drops, uptime/reboot detection (boottime tolerance dedup + uptime cross-check), clock offset, zombies, fd, port/service probes, ICMP, certificates, Docker/DB/IPMI when configured. Ranges >1 day use hourly trends (weighted average); ≤1 day uses history; cumulative counters are auto-differenced into rates. New metrics plug in via one regex in `patrol.toml`.

## HTTP API (for programs)

```bash
zbxpatrol serve --listen 127.0.0.1:8787 --token mytoken
```

| Endpoint | Description |
|---|---|
| `GET /health` | liveness + Zabbix version |
| `GET /groups` `GET /hosts?group=X` | groups / hosts |
| `GET /items?host=X&search=cpu` | item catalog |
| `POST /query` | `{"keys":["system.cpu.util"],"hosts":["h1"],"time":{"last":"7d"}}` |
| `POST /report` | same body + `"strictness"`, `"keys"`; `?format=xlsx|csv` streams the file; `&save=1` writes it on the server under `reports/` and returns the path |

Envelope `{"ok":true,"data":...}`; auth `Authorization: Bearer <token>`; binds localhost by default — set a token when exposing. OpenAPI spec: [docs/openapi.yaml](docs/openapi.yaml); self-test: `deploy/api-selftest.sh`. Python example:

```python
import requests
r = requests.post("http://127.0.0.1:8787/report",
    headers={"Authorization": "Bearer mytoken"},
    json={"group": "<group>", "time": {"period": "day"}}, timeout=300)
for h in r.json()["data"]["hosts"]:
    print(h["host"]["host"], h["risk"]["score"], h["risk"]["level"])
```

## Long-running Deployment

Artifacts: `target/release/zbxpatrol` (macOS arm64); Linux via musl static cross-build:

```bash
# option 1: zigbuild (recommended)
cargo install cargo-zigbuild && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
cargo zigbuild --release --target x86_64-unknown-linux-musl

# option 2: cross (needs Docker)
cargo install cross && cross build --release --target x86_64-unknown-linux-musl
```

- **Scheduled inspection**: `deploy/crontab.example` (daily/weekly/monthly) or systemd timer (`deploy/zbxpatrol-daily.service`)
- **Persistent API**: `deploy/zbxpatrol-serve.service`
- **Docker**: `deploy/Dockerfile`

> **Internal-DNS compatibility (built-in)**: ships the hickory pure-Rust resolver (glibc-compatible behavior), so static binaries resolve fine in internal-DNS environments with zero configuration (verified on Rocky Linux 9.8). Extreme fallback: append `"<zabbix-ip> <your-zabbix-domain>"` to `/etc/hosts`.

## Install from the Gitea Package Registry (v1.1.0+)

Prebuilt artifacts are published to the Gitea Generic Package Registry (macOS Apple Silicon / Linux x86_64 / Linux aarch64 — same codebase, no branches):

```bash
# download (<gitea-host> = your Gitea base URL; private packages need -u <user>:<pass>)
curl -u <user>:<pass> -o zbxpatrol \
  "http://<gitea-host>/api/packages/OM/generic/zbxpatrol/1.1.0/zbxpatrol-1.1.0-linux-x86_64"
chmod +x zbxpatrol && ./zbxpatrol check

# publish a new version (builds + uploads all platforms; host taken from git remote)
GITEA_USER=<user> GITEA_PASS=<pass> ./deploy/package-upload.sh <version>
```

Also downloadable from the Gitea web UI "Packages" page.

## Development

```bash
cargo test                 # unit tests (time/aggregation/scoring/rules, offline)
cargo clippy --all-targets # zero warnings
cargo build --release
```

Extensions: new metric → one regex in `patrol.toml [metrics.rules]`; custom scoring → `[[scoring]]`; new output → add a renderer under `crates/zbxpatrol-cli/src/render/`; new data source → abstract `zabbix.rs`. See [docs/CODE.en.md](docs/CODE.en.md).
