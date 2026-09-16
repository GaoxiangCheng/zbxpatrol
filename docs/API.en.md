# zbxpatrol HTTP API Reference (English)

> Consume zbxpatrol from third-party programs (ops platforms / scripts): health, groups/hosts/items listing, metric stats, inspection reports (JSON / Excel stream).
> 中文版: [API.zh.md](API.zh.md)

<a id="toc"></a>
## Contents

1. [Machine-readable spec & self-test](#machine)
1. [Starting the service](#start)
2. [Conventions](#conventions) — [Auth](#auth) · [Envelope](#envelope) · [Errors](#errors) · [Time](#time) · [Scope](#scope)
3. [Endpoints](#endpoints) — [/health](#health) · [/groups](#groups) · [/hosts](#hosts) · [/items](#items) · [/query](#query) · [/report](#report) · [/report?format=xlsx](#xlsx)
4. [ReportData schema](#schema)
5. [Examples](#examples) (curl / Python / integration tips)

---

<a id="machine"></a>
## 0. Machine-readable spec & self-test

- **OpenAPI 3.0**: [openapi.yaml](openapi.yaml) — complete schemas (envelope, MetricStats, HostInspection, ReportData…), examples and error responses. Import it into Swagger UI, Postman, Insomnia or code generators to explore/test the API automatically.
- **Self-test script**: [`deploy/api-selftest.sh`](../deploy/api-selftest.sh) asserts every endpoint against a live instance (health, catalog, query stats numeric + source, report schema fields, risk score bounds, spark/extra attachment, xlsx magic bytes `PK`, 400/401 error codes) and exits non-zero on any failure:

```bash
zbxpatrol serve --listen 127.0.0.1:8787 --token <TOKEN> &   # terminal 1
./deploy/api-selftest.sh http://127.0.0.1:8787 <TOKEN>        # terminal 2
# RESULT: PASS=22 FAIL=0  → exit code 0
```

<a id="start"></a>
## 1. Starting the service

```bash
zbxpatrol serve --listen 127.0.0.1:8787
# expose externally only with a token:
zbxpatrol serve --listen 0.0.0.0:8787 --token <TOKEN>
```

- Default bind `127.0.0.1:8787` (localhost only). Keep it local or set a token in production.
- Run persistently via systemd (see `deploy/zbxpatrol-serve.service`).
- An optional `patrol.toml` in the working dir is auto-loaded (custom metric rules / scoring rules).

[↑ TOC](#toc)

<a id="conventions"></a>
## 2. Conventions

<a id="auth"></a>
### 2.1 Authentication

> Failed auth (401) is delayed by 200 ms to slow online brute force; the token can be injected via the `PATROL_TOKEN` env var to keep it out of the process command line.

- No `--token` at startup: auth disabled (localhost usage recommended).
- With `--token <TOKEN>`: every request except `/health` must send
  `Authorization: Bearer <TOKEN>`, otherwise `401`.

<a id="envelope"></a>
### 2.2 Envelope

```json
{ "ok": true, "data": <payload> }
{ "ok": false, "error": { "code": <code>, "message": "<human readable>" } }
```

<a id="errors"></a>
### 2.3 Error codes

| HTTP | code | Meaning |
|---|---|---|
| 400 | 2 | Bad request / config / credentials (unknown group, invalid strictness…) |
| 401 | 401 | Missing/wrong token |
| 502 | 3 | Network or Zabbix API error (incl. failed re-login) |
| 500 | 3 | xlsx generation failure |
| 404 | 0 | unknown endpoint |

<a id="time"></a>
### 2.4 Time parameter (pick one)

Both a nested `time` object (recommended, below) and flat fields `period`/`last`/`from`/`to` are accepted; mixing them returns 400.

```json
{"period": "day|week|month|year"}       // 24h / 7d / 30d / 365d
{"last": "48h"}                          // any duration: h/d/M/m
{"from": "2026-09-01", "to": "2026-09-15 23:59"}  // absolute, to optional = now
```

Priority: `from/to` > `last` > `period`. Timezone from `ZBX_TZ` (default Asia/Shanghai).

<a id="scope"></a>
### 2.5 Scope parameter (optional, default = all hosts)
When several scope parameters are given, the more specific one wins: `hosts` > `group` (matching the CLI `--host`/`--hosts` > `--group`).

```json
{"group": "<group>"}                     // single group
{"hosts": ["<host1>", "<host2>"]}        // host list
```

[↑ TOC](#toc)

<a id="endpoints"></a>
## 3. Endpoints

<a id="health"></a>
### 3.1 GET /health — liveness + Zabbix connectivity

```bash
curl -s http://127.0.0.1:8787/health
```
```json
{"ok":true,"data":{"status":"up","zabbix":"https://<ZBX_URL>","zabbix_version":"7.4.14","version":"1.0.0"}}
```
No auth required; auto re-login on expired session.

<a id="groups"></a>
### 3.2 GET /groups — host groups

```json
{"ok":true,"data":[{"groupid":"4","name":"<groupA>"},{"groupid":"5","name":"<groupB>"}]}
```

<a id="hosts"></a>
### 3.3 GET /hosts — hosts

Query: `group=<group>` (optional filter).

```json
{"ok":true,"data":[{"hostid":"10084","host":"<host>","name":"<visible name>","ip":"<IP>","groups":["<group>"],"os_family":"Linux|Windows|..."}]}
```

<a id="items"></a>
### 3.4 GET /items — item keys aggregated

Query: `host=<host>` or `group=<group>` (default all); `search=<substring>` (optional).

```json
{"ok":true,"data":[{"key":"system.cpu.util","name":"CPU utilization","unit":"%","value_type":0,"hosts":72}]}
```

<a id="query"></a>
### 3.5 POST /query — stats for arbitrary items

Body: `{"keys":["<key1>","<wildcard*>"],"group"|"hosts":...,"time":{...}}` (keys required, `*` wildcard).

```bash
curl -s -X POST http://127.0.0.1:8787/query -H "Content-Type: application/json" \
  -d '{"keys":["system.cpu.util"],"hosts":["<host>"],"time":{"last":"24h"}}'
```
```json
{"ok":true,"data":[{"host":"<host>","key":"system.cpu.util","name":"CPU utilization",
  "stats":{"cur":13.1,"avg":12.7,"max":19.6,"min":9.0,"unit":"%","count":1370,"source":"history","missing":false},
  "trend":[null,11.2,12.8,"... up to 32 sparkline buckets"]}]}
```
> `trend` (bucketed series) is attached when ≤30 items matched; `source` shows history/trend.

<a id="report"></a>
### 3.6 POST /report — inspection report (JSON)

Body: `{"group"|"hosts":...,"time":{...},"strictness":"loose|standard|strict","all_items":false,"keys":["<extra keys>"]}` (all optional).

- `strictness`: scoring strictness, loose (baseline 90%) / standard (80%) / strict (70%), default standard;
- `keys`: extra custom item keys (wildcards ok) → results in each host's `extra` field;
- `all_items`: true → attach per-host stats of all numeric items (`all_items` field).

Returns the full [ReportData schema](#schema).

<a id="xlsx"></a>
### 3.7 POST /report — file export (stream or server-side save)

**Stream to caller** (`format=xlsx|csv`, binary download):

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx" \
  -H "Content-Type: application/json" \
  -d '{"group":"<group>","time":{"period":"day"}}' -o report.xlsx
# csv: ?format=csv  (UTF-8 BOM, opens directly in Excel)
```

**Generate the file ON THE SERVER** (`save=1` + format): the service writes the file under `reports/` of the serve working directory and returns the path (plus summary) — ideal when another system should fetch/share the artifact later, or the caller cannot receive streams:

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx&save=1" \
  -H "Content-Type: application/json" \
  -d '{"group":"<group>","time":{"period":"day"}}'
```
```json
{"ok":true,"data":{"saved":true,"format":"xlsx",
  "file":"reports/patrol-report_<from>_<till>_<scope>.xlsx",
  "summary":{"host_total":5,"risk_dist":{...}}}}
```
`format=json&save=1` / `format=csv&save=1` work the same way (.json / .csv under `reports/`).

[↑ TOC](#toc)

<a id="schema"></a>
## 4. ReportData schema (key fields)

```jsonc
{
  "version": "1.0.0",
  "generated_at": 1757900000,
  "range": { "from": 1757813600, "till": 1757899999, "tz": "Asia/Shanghai", "human": "… ~ …" },
  "scope_type": "all|group|hosts",
  "scope_names": ["<group>"],
  "strictness": "标准（基准 80%）",
  "summary": {
    "host_total": 5, "available": 5, "unavailable": 0, "missing_data": 0,
    "risk_dist": { "healthy": 3, "low": 0, "medium": 1, "high": 1, "critical": 0 },
    "top_risk": [ { "host": "<host>", "score": 75, "level": "高危" } ],
    "problem_open": 0
  },
  "hosts": [   // HostInspection[]
    {
      "host": { "hostid":"…", "host":"<host>", "name":"<name>", "ip":"<IP>", "groups":[…], "os_family":"Linux" },
      "os": "<raw uname>", "os_family": "Linux|Windows|…",
      "available": true,
      "metrics": {
        "cpu":  { "cur":…, "avg":…, "max":…, "min":…, "unit":"%", "count":n, "source":"history|trend", "missing":false },
        "mem":  { … },
        "swap": { … } | null,
        "disk_max":  { "mount":"/data", "total_b":…, "used_b":…, "space":{quad}, "inode":{quad}|null, "forecast_days": 17.8|null },
        "inode_max": { … } | null,
        "uptime_days": 123.4
      },
      "disks": [ { "mount":"/", "space":{…}, "inode":{…}|null, "forecast_days":… } ],
      "nets": [ { "ifname":"eth0", "in_mbps":{…}, "out_mbps":{…}, "util_pct":{…}|null,
                  "in_errors":0, "out_errors":0, "in_dropped":0, "out_dropped":0 } ],
      "services": [ { "key":"net.tcp.port[…]", "name":"…", "ok":true } ],
      "stability": { "reboots":0, "time_offset_s":{…}|null, "zombies":{…}|null, "fd_util":{…}|null,
                     "cpu_num":8, "load1":{…}, "load5":{…}, "load15":{…},
                     "icmp_loss":{…}|null, "icmp_latency":{…}|null, "cert_min_days":{…}|null, "oom_events":null },
      "risk": { "score": 75, "level": "高危", "points": ["[+30] …"] },
      "problems": [ { "eventid":"…", "name":"…", "severity":4, "severity_label":"高危",
                      "clock":…, "recovered":false, "acknowledged":false, "hosts":["<host>"] } ],
      "spark":  { "cpu":[…48 buckets], "mem":[…], "disk":[…], "disk_mount":"/data" },
      "extra":  { "<extra key>": {quad} },
      "all_items": { "<key>": {quad} },
      "missing_data": false
    }
  ],
  "problems": [ …all problems in range… ]
}
```

> Additive-only compatibility. `quad` = `{cur, avg, max, min, unit, count, source, missing}`.

[↑ TOC](#toc)

<a id="examples"></a>
## 5. Examples

### 5.1 Python — pull report, list high-risk hosts

```python
import requests

API = "http://127.0.0.1:8787"
H = {"Authorization": "Bearer <TOKEN>"}   # omit if no token

r = requests.post(f"{API}/report", headers=H,
    json={"group": "<group>", "time": {"period": "day"},
          "strictness": "strict", "keys": ["net.if*"]}, timeout=300)
for h in r.json()["data"]["hosts"]:
    if h["risk"]["score"] >= 75:
        print(h["host"]["host"], h["risk"]["score"], h["risk"]["level"], h["risk"]["points"])
```

### 5.2 curl — download Excel

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx" \
  -H "Authorization: Bearer <TOKEN>" -H "Content-Type: application/json" \
  -d '{"hosts":["<host>"],"time":{"from":"2026-09-01","to":"2026-09-15"}}' \
  -o report.xlsx
```

### 5.3 Integration tips

- Poll `/report` to persist full JSON, or just `summary` + `hosts[].risk`;
- Scheduled runs fit the CLI better (`zbxpatrol report --quiet`, see `deploy/crontab.example`); use the API for interactive queries;
- Long ranges (month/year) scale with host count — set `timeout ≥ 300s`.

[↑ TOC](#toc) · [Doc index](README.md)
