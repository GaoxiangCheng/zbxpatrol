# zbxpatrol HTTP API 接口文档（中文）

> 供第三方程序（运维平台/脚本）调用 zbxpatrol 能力：健康检查、群组/主机/监控项查询、指标统计、巡检报表（JSON / Excel 文件流）。
> English version: [API.en.md](API.en.md)

<a id="toc"></a>
## 目录

0. [机器可读规范与自测](#machine)
1. [启动服务](#start)
2. [通用约定](#conventions)
   - [认证](#auth) · [响应包裹](#envelope) · [错误码](#errors) · [时间参数](#time) · [范围参数](#scope)
3. [接口清单](#endpoints)
   - [GET /health](#health) · [GET /groups](#groups) · [GET /hosts](#hosts) · [GET /items](#items)
   - [POST /query](#query) · [POST /report](#report) · [文件导出（流式 / 服务器端落盘）](#xlsx)
4. [ReportData 数据结构](#schema)
5. [使用示例](#examples)（curl / Python / 集成建议）

---

<a id="machine"></a>
## 0. 机器可读规范与自测

- **OpenAPI 3.0**：[openapi.yaml](openapi.yaml)——完整 schema（响应包裹/MetricStats/HostInspection/ReportData 等）+ 示例 + 错误响应，可导入 Swagger UI / Postman / 代码生成器自动探索与测试；
- **自测脚本**：[`deploy/api-selftest.sh`](../deploy/api-selftest.sh) 对运行中的实例逐接口断言（健康/目录/查询数值与来源/报表结构字段/风险分区间/spark与extra/xlsx 魔数 PK/400/401），任一失败退出非 0：

```bash
zbxpatrol serve --listen 127.0.0.1:8787 --token <TOKEN> &   # 终端1
./deploy/api-selftest.sh http://127.0.0.1:8787 <TOKEN>        # 终端2
# 结果 RESULT: PASS=22 FAIL=0 → 退出码 0
```

<a id="start"></a>
## 1. 启动服务

```bash
# 交互式初始化配置后（见 交互文档），启动：
zbxpatrol serve --listen 127.0.0.1:8787
# 对外暴露时必须设置 token：
zbxpatrol serve --listen 0.0.0.0:8787 --token <你的TOKEN>
```

- 默认监听 `127.0.0.1:8787`（仅本机）；生产环境建议保持仅本机或加 token。
- 常驻运行用 systemd（见仓库 `deploy/zbxpatrol-serve.service`）。
- 服务自动加载当前目录 `patrol.toml`（可选业务配置：自定义指标规则/评分规则）。

[↑ 返回目录](#toc)

<a id="conventions"></a>
## 2. 通用约定

<a id="auth"></a>
### 2.1 认证

- 启动时未设 `--token`：不鉴权（仅建议本机使用）。
- 设置了 `--token <TOKEN>`：除 `/health` 外所有请求必须携带请求头
  `Authorization: Bearer <TOKEN>`，否则返回 `401`。

<a id="envelope"></a>
### 2.2 响应包裹

```json
// 成功
{ "ok": true, "data": <各接口数据> }
// 失败
{ "ok": false, "error": { "code": <业务码>, "message": "<人可读信息>" } }
```

<a id="errors"></a>
### 2.3 错误码

| HTTP | 业务码 code | 含义 |
|---|---|---|
| 400 | 2 | 请求参数/配置/凭据错误（如未知群组、strictness 非法） |
| 401 | 401 | token 缺失或错误 |
| 502 | 3 | 网络或 Zabbix API 错误（含认证失效重登仍失败） |
| 404 | 0 | 未知端点 |
| 500 | 3 | 报表 xlsx 生成失败 |

<a id="time"></a>
### 2.4 时间参数（三选一）

支持嵌套 `time` 对象（推荐，见下）或平铺字段 `period`/`last`/`from`/`to`；两者不可混用（混用返回 400）。

```json
{"period": "day|week|month|year"}      // 24h / 7d / 30d / 365d
{"last": "48h"}                          // 任意相对时长：h/d/M/m
{"from": "2026-09-01", "to": "2026-09-15 23:59"}  // 绝对区间，to 可省略=现在
```

优先级：`from/to` > `last` > `period`。时区取 `ZBX_TZ`（默认 Asia/Shanghai）。

<a id="scope"></a>
### 2.5 范围参数（scope，二选一；缺省=全部主机）
范围参数同时给出时按精确度取优先级：`hosts` > `group`（与 CLI 的 `--host`/`--hosts` > `--group` 一致）。

```json
{"group": "<群组名>"}          // 单群组
{"hosts": ["<主机1>", "<主机2>"]} // 主机列表
```

[↑ 返回目录](#toc)

<a id="endpoints"></a>
## 3. 接口清单

<a id="health"></a>
### 3.1 GET /health — 存活与 Zabbix 连通

```bash
curl -s http://127.0.0.1:8787/health
```
```json
{"ok":true,"data":{"status":"up","zabbix":"https://<ZBX_URL>","zabbix_version":"7.4.14","version":"1.0.0"}}
```
免鉴权；会话过期时自动重登。

<a id="groups"></a>
### 3.2 GET /groups — 主机群组列表

```json
{"ok":true,"data":[{"groupid":"4","name":"<群组A>"},{"groupid":"5","name":"<群组B>"}]}
```

<a id="hosts"></a>
### 3.3 GET /hosts — 主机列表

Query 参数：`group=<群组名>`（可选过滤）。

```json
{"ok":true,"data":[{"hostid":"10084","host":"<主机名>","name":"<可见名>","ip":"<IP>","groups":["<群组>"],"os_family":"Linux|Windows|..."}]}
```

<a id="items"></a>
### 3.4 GET /items — 监控项清单（按 key 聚合）

Query 参数：`host=<主机名>` 或 `group=<群组名>`（缺省全部）；`search=<子串>`（可选，匹配 key/名称）。

```json
{"ok":true,"data":[{"key":"system.cpu.util","name":"CPU utilization","unit":"%","value_type":0,"hosts":72}]}
```

<a id="query"></a>
### 3.5 POST /query — 任意监控项统计

请求体：`{"keys": ["<key1>","<通配*>"], "group"|"hosts": ..., "time": {...}}`（keys 必填，支持 `*` 通配）。

```bash
curl -s -X POST http://127.0.0.1:8787/query -H "Content-Type: application/json" \
  -d '{"keys":["system.cpu.util"],"hosts":["<主机名>"],"time":{"last":"24h"}}'
```
```json
{"ok":true,"data":[{"host":"<主机名>","key":"system.cpu.util","name":"CPU utilization",
  "stats":{"cur":13.1,"avg":12.7,"max":19.6,"min":9.0,"unit":"%","count":1370,"source":"history","missing":false},
  "trend":[null,11.2,12.8,"...最多32桶火花线序列"]}]}
```
> 命中监控项 ≤30 时附带 `trend` 分桶序列（供绘制趋势线）；`source` 标注数据来源 history/trend。

<a id="report"></a>
### 3.6 POST /report — 巡检报表（JSON）

请求体：`{ "group"|"hosts": ..., "time": {...}, "strictness": "loose|standard|strict", "all_items": false, "keys": ["<附加key>"] }`（均可选）。

- `strictness`：评分严格度，宽松(基准90%)/标准(80%)/严格(70%)，缺省 standard；
- `keys`：附加自定义指标（支持通配），结果进每台主机的 `extra` 字段；
- `all_items`：true 时附带每主机全部数值指标统计（`all_items` 字段）。

返回完整 [ReportData 结构](#schema)。

<a id="xlsx"></a>
### 3.7 POST /report — 文件导出（流式 / 服务器端落盘）

**流式下载**（`format=xlsx|csv`，返回二进制）：

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx" \
  -H "Content-Type: application/json" \
  -d '{"group":"<群组名>","time":{"period":"day"}}' -o 巡检报告.xlsx
# csv：?format=csv（UTF-8 BOM，Excel 直开）
```

**在服务器上生成导出文件**（`save=1` + format）：服务端把文件写入其工作目录 `reports/` 并返回路径与摘要——适合由其他系统稍后取用/共享，或调用方不便接收流的场景：

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx&save=1" \
  -H "Content-Type: application/json" \
  -d '{"group":"<群组名>","time":{"period":"day"}}'
```
```json
{"ok":true,"data":{"saved":true,"format":"xlsx",
  "file":"reports/巡检报告_<起>_<止>_<范围>.xlsx",
  "summary":{"host_total":5,"risk_dist":{…}}}}
```
`format=json&save=1`、`format=csv&save=1` 同理（reports/ 下生成 .json / .csv）。

[↑ 返回目录](#toc)

<a id="schema"></a>
## 4. ReportData 数据结构（关键字段）

```jsonc
{
  "version": "1.0.0",
  "generated_at": 1757900000,                  // epoch 秒
  "range": { "from": 1757813600, "till": 1757899999, "tz": "Asia/Shanghai", "human": "… ~ …" },
  "scope_type": "all|group|hosts",
  "scope_names": ["<群组名>"],
  "strictness": "标准（基准 80%）",
  "summary": {
    "host_total": 5, "available": 5, "unavailable": 0, "missing_data": 0,
    "risk_dist": { "healthy": 3, "low": 0, "medium": 1, "high": 1, "critical": 0 },
    "top_risk": [ { "host": "<主机名>", "score": 75, "level": "高危" } ],
    "problem_open": 0
  },
  "hosts": [                                   // HostInspection[]
    {
      "host": { "hostid":"…", "host":"<主机名>", "name":"<可见名>", "ip":"<IP>", "groups":[…], "os_family":"Linux" },
      "os": "<uname 原文>", "os_family": "Linux|Windows|…",
      "available": true,
      "metrics": {
        "cpu":  { "cur":…, "avg":…, "max":…, "min":…, "unit":"%", "count":n, "source":"history|trend", "missing":false },
        "mem":  { …同上… },
        "swap": { … } | null,
        "disk_max":  { "mount":"/data", "total_b":…, "used_b":…, "space":{四值}, "inode":{四值}|null, "forecast_days": 17.8|null },
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
      "risk": { "score": 75, "level": "高危", "points": ["[+30] 内存 使用率 71.4%，达到中危阈值 70%"] },
      "problems": [ { "eventid":"…", "name":"…", "severity":4, "severity_label":"高危",
                      "clock":…, "recovered":false, "acknowledged":false, "hosts":["<主机名>"] } ],
      "spark":  { "cpu":[…48桶], "mem":[…], "disk":[…], "disk_mount":"/data" },   // 趋势火花线序列
      "extra":  { "<附加key>": {四值统计} },                                      // 请求 keys 命中项
      "all_items": { "<key>": {四值统计} },                                      // all_items=true 时
      "missing_data": false
    }
  ],
  "problems": [ …区间内全部问题… ]
}
```

> 字段只增不改（向后兼容承诺）。`四值统计` = `{cur, avg, max, min, unit, count, source, missing}`。

[↑ 返回目录](#toc)

<a id="examples"></a>
## 5. 使用示例

### 5.1 Python：拉取报表并筛选高危主机

```python
import requests

API = "http://127.0.0.1:8787"
H = {"Authorization": "Bearer <TOKEN>"}   # 未设 token 可省

r = requests.post(f"{API}/report", headers=H,
    json={"group": "<群组名>", "time": {"period": "day"},
          "strictness": "strict", "keys": ["net.if*"]}, timeout=300)
data = r.json()["data"]
for h in data["hosts"]:
    if h["risk"]["score"] >= 75:
        print(h["host"]["host"], h["risk"]["score"], h["risk"]["level"], h["risk"]["points"])
```

### 5.2 curl：下载 Excel 报表

```bash
curl -s -X POST "http://127.0.0.1:8787/report?format=xlsx" \
  -H "Authorization: Bearer <TOKEN>" -H "Content-Type: application/json" \
  -d '{"hosts":["<主机名>"],"time":{"from":"2026-09-01","to":"2026-09-15"}}' \
  -o 巡检报告.xlsx
```

### 5.3 集成建议

- 平台侧轮询 `/report` 存库（JSON 全量），或仅存 `summary` + `hosts[].risk`；
- 定时任务直接用 CLI（`zbxpatrol report --quiet`，cron 示例见 `deploy/crontab.example`），API 适合交互式查询；
- 长区间报表（月/年）耗时随主机数增长，建议 `timeout ≥ 300s`。

[↑ 返回目录](#toc) · [返回文档索引](README.md)
