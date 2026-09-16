#!/bin/bash
# zbxpatrol API 自测 / API self-test
# 对运行中的 serve 实例逐接口断言（健康/目录/查询/报表/鉴权/错误码/xlsx）。
# Asserts every endpoint of a running serve instance (health/catalog/query/report/auth/errors/xlsx).
#
# Usage:
#   ./zbxpatrol serve --listen 127.0.0.1:8787 [--token mytoken]   # terminal 1
#   ./deploy/api-selftest.sh [BASE_URL] [TOKEN]                    # terminal 2
# Env overrides: ZBP_HOST (default <first host>)
set -u
BASE="${1:-http://127.0.0.1:8787}"
TOKEN="${2:-}"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

PASS=0; FAIL=0
say() { printf '%s\n' "$*"; }
ok()  { PASS=$((PASS+1)); say "PASS  $*"; }
bad() { FAIL=$((FAIL+1)); say "FAIL  $*"; }

# 请求封装：响应落文件，状态码单独取 / request helper: body -> file, status -> stdout
req() { # req <outfile> <statusfile> <method> <url> [curl-extra...]
  local out=$1 st=$2 m=$3 u=$4; shift 4
  if [ -n "$TOKEN" ]; then
    curl -s -m 300 -o "$out" -w "%{http_code}" -X "$m" -H "Authorization: Bearer $TOKEN" "$@" "$u" >"$st"
  else
    curl -s -m 300 -o "$out" -w "%{http_code}" -X "$m" "$@" "$u" >"$st"
  fi
}
noauth() { # 不带鉴权的请求（401 测试）
  local out=$1 st=$2 u=$3
  curl -s -m 15 -o "$out" -w "%{http_code}" "$u" >"$st"
}
jq_() { # 从文件读 JSON 并取路径 / read JSON file, walk path
  python3 "$T/walk.py" "$1" "${@:2}"
}
cat > "$T/walk.py" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for p in sys.argv[2:]:
    d = d[int(p)] if isinstance(d, list) else d.get(p)
print(json.dumps(d, ensure_ascii=False) if isinstance(d, (dict, list)) else d)
PY
isnum() { python3 -c "import sys;float(sys.argv[1])" "$1" 2>/dev/null; }

# ---------- 1. /health ----------
req "$T/h.json" "$T/h.st" GET "$BASE/health"
[ "$(jq_ "$T/h.json" ok)" = "True" ] && ok "/health ok=true ($(jq_ "$T/h.json" data zabbix_version))" || bad "/health: $(head -c 120 "$T/h.json")"

# ---------- 2. /groups ----------
req "$T/g.json" "$T/g.st" GET "$BASE/groups"
G=$(jq_ "$T/g.json" data 0 name)
[ -n "$G" ] && [ "$G" != "None" ] && ok "/groups first group: $G" || bad "/groups empty"

# ---------- 3. /hosts ----------
req "$T/hs.json" "$T/hs.st" GET "$BASE/hosts"
H="${ZBP_HOST:-$(jq_ "$T/hs.json" data 0 host)}"
OS=$(jq_ "$T/hs.json" data 0 os_family)
[ -n "$H" ] && [ "$H" != "None" ] && ok "/hosts first host: $H (os_family=$OS)" || bad "/hosts empty"
[ -n "$(jq_ "$T/hs.json" data 0 ip)" ] && ok "/hosts ip present" || bad "/hosts ip missing"

# ---------- 4. /items ----------
HU=$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1]))" "$H")
req "$T/it.json" "$T/it.st" GET "$BASE/items?host=$HU&search=cpu"
[ "$(jq_ "$T/it.json" ok)" = "True" ] && ok "/items?host&search=cpu ok ($(python3 -c "import json;print(len(json.load(open('$T/it.json'))['data']))" ) keys)" || bad "/items failed"

# ---------- 5. /query ----------
printf '{"keys":["system.cpu.util"],"hosts":["%s"],"time":{"last":"24h"}}' "$H" > "$T/q.body"
req "$T/q.json" "$T/q.st" POST "$BASE/query" -H "Content-Type: application/json" --data-binary "@$T/q.body"
AVG=$(jq_ "$T/q.json" data 0 stats avg)
isnum "$AVG" && ok "/query stats.avg numeric: $AVG" || bad "/query avg bad: $(head -c 120 "$T/q.json")"
SRC=$(jq_ "$T/q.json" data 0 stats source)
{ [ "$SRC" = "history" ] || [ "$SRC" = "trend" ]; } && ok "/query source=$SRC" || bad "/query source=$SRC"

# ---------- 6. /report JSON 结构 ----------
printf '{"hosts":["%s"],"time":{"last":"24h"},"strictness":"standard","keys":["system.cpu.util*"]}' "$H" > "$T/r.body"
req "$T/r.json" "$T/r.st" POST "$BASE/report" -H "Content-Type: application/json" --data-binary "@$T/r.body"
[ "$(jq_ "$T/r.json" ok)" = "True" ] && ok "/report ok=true" || bad "/report: $(head -c 200 "$T/r.json")"
for f in version generated_at range strictness summary hosts; do
  [ "$(jq_ "$T/r.json" data "$f")" != "None" ] && ok "/report field $f present" || bad "/report field $f missing"
done
CPU_AVG=$(jq_ "$T/r.json" data hosts 0 metrics cpu avg)
isnum "$CPU_AVG" && ok "/report cpu.avg=$CPU_AVG" || bad "/report cpu.avg bad"
SCORE=$(jq_ "$T/r.json" data hosts 0 risk score)
python3 -c "import sys;0<=int(sys.argv[1])<=100" "$SCORE" 2>/dev/null && ok "/report risk.score=$SCORE (0-100)" || bad "/report risk.score=$SCORE"
case "$(jq_ "$T/r.json" data hosts 0 risk level)" in
  健康|低危|中危|高危|严重) ok "/report risk.level=$(jq_ "$T/r.json" data hosts 0 risk level)";;
  *) bad "/report level invalid";;
esac
python3 - "$T/r.json" <<'PY' && ok "/report spark.cpu & extra attached" || bad "/report spark/extra missing"
import json, sys
h = json.load(open(sys.argv[1]))["data"]["hosts"][0]
assert len(h.get("spark", {}).get("cpu", [])) > 0, "spark empty"
assert len(h.get("extra") or {}) > 0, "extra empty"
PY

# ---------- 7. /report xlsx 魔数 ----------
req "$T/r.xlsx" "$T/rx.st" POST "$BASE/report?format=xlsx" -H "Content-Type: application/json" --data-binary "@$T/q.body"
M=$(head -c 2 "$T/r.xlsx")
[ "$M" = "PK" ] && ok "/report?format=xlsx magic=PK ($(wc -c <"$T/r.xlsx" | tr -d ' ') bytes)" || bad "xlsx not zip (got '$M')"

# ---------- 7b. /report?save=1 服务器端落盘 ----------
printf '{"hosts":["%s"],"time":{"last":"1h"}}' "$H" > "$T/s.body"
req "$T/s.json" "$T/s.st" POST "$BASE/report?format=xlsx&save=1" -H "Content-Type: application/json" --data-binary "@$T/s.body"
SV=$(jq_ "$T/s.json" data file)
[ "$(jq_ "$T/s.json" data saved)" = "True" ] && [ -n "$SV" ] && [ "$SV" != "None" ] && ok "save=1 → file: $SV" || bad "save=1: $(head -c 150 "$T/s.json")"

# ---------- 8. 错误码：未知群组 → 400 ----------
printf '{"group":"__no_such_group__","time":{"period":"day"}}' > "$T/e.body"
req "$T/e.json" "$T/e.st" POST "$BASE/report" -H "Content-Type: application/json" --data-binary "@$T/e.body"
[ "$(cat "$T/e.st")" = "400" ] && ok "unknown group → HTTP 400" || bad "unknown group → HTTP $(cat "$T/e.st") (expect 400)"

# ---------- 9. 鉴权（仅设置 TOKEN 时） ----------
if [ -n "$TOKEN" ]; then
  noauth "$T/a.json" "$T/a.st" "$BASE/groups"
  [ "$(cat "$T/a.st")" = "401" ] && ok "no token → 401" || bad "no token → $(cat "$T/a.st")"
  curl -s -m 15 -o "$T/a2.json" -w "%{http_code}" -H "Authorization: Bearer wrong" "$BASE/groups" >"$T/a2.st"
  [ "$(cat "$T/a2.st")" = "401" ] && ok "wrong token → 401" || bad "wrong token → $(cat "$T/a2.st")"
else
  say "SKIP  auth tests (no TOKEN provided)"
fi

say ""
say "======== 结果 / RESULT: PASS=$PASS FAIL=$FAIL ========"
[ "$FAIL" -eq 0 ]
