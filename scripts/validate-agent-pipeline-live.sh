#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
set -a
source .env.example
set +a

APP_HOST=127.0.0.1
APP_PORT="${VALIDATION_APP_PORT:-3016}"
PG_PORT="${VALIDATION_PG_PORT:-35432}"
QUERY_PHONE="${QUERY_PHONE:-0926408013}"
QUERY_BANK="${QUERY_BANK:-123456789}"
QUERY_URL="${QUERY_URL:-https://evil.example/path?a=1&b=2}"
STAMP="$(date +%y%m%d-%H%M%S)"
REPORT="plans/reports/validation-${STAMP}-agent-pipeline-live.md"
DETECTIVE_ARTIFACT="plans/reports/validation-${STAMP}-phone-detective.md"
APP_LOG_ARTIFACT="plans/reports/validation-${STAMP}-app.log"
TMP_AGENT_DIR="$(mktemp -d)"
TMP_DIR="$(mktemp -d)"
PG_CONTAINER="tracuuluadao-validation-${STAMP}"
APP_LOG="${TMP_DIR}/app.log"
APP_PID=""

cp -R config/agents/. "$TMP_AGENT_DIR/"
DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${PG_PORT}/tracuuluadao"

cleanup() {
  local exit_code=$?
  if [[ -n "${APP_PID}" ]] && kill -0 "${APP_PID}" 2>/dev/null; then
    kill "${APP_PID}" 2>/dev/null || true
    wait "${APP_PID}" 2>/dev/null || true
  fi
  if [[ ${exit_code} -ne 0 ]]; then
    echo "[validate] failed; preserving artifacts"
    echo "[validate] tmp-agent-dir=${TMP_AGENT_DIR}"
    echo "[validate] tmp-dir=${TMP_DIR}"
    echo "[validate] postgres-container=${PG_CONTAINER}"
    return
  fi
  docker rm -f "${PG_CONTAINER}" >/dev/null 2>&1 || true
  rm -rf "${TMP_AGENT_DIR}" "${TMP_DIR}"
}
trap cleanup EXIT

pg() {
  docker exec "${PG_CONTAINER}" psql -U postgres -d tracuuluadao -tAqc "$1"
}

urlencode() {
  python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1"
}

write_report() {
  printf '%s\n' "$*" >> "${REPORT}"
}

require_contains() {
  local file="$1"
  local text="$2"
  grep -q "$text" "$file"
}

assert_direct_json_response() {
  local file="$1"
  local expected_id="${2:-}"
  python3 - "$file" "$expected_id" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
message = payload["choices"][0]["message"]
content = message.get("content") or ""
parsed = json.loads(content)
if parsed.get("ok") is not True:
    raise SystemExit("expected ok=true")
expected_id = sys.argv[2]
if expected_id and str(parsed.get("id")) != expected_id:
    raise SystemExit(f"expected id={expected_id}, got {parsed.get('id')}")
PY
}

extract_openai_stream_text() {
  local file="$1"
  python3 - "$file" <<'PY'
import json
import sys

parts = []
for raw in open(sys.argv[1], encoding="utf-8", errors="ignore"):
    line = raw.strip()
    if not line.startswith("data: "):
        continue
    data = line[6:]
    if data == "[DONE]":
        break
    payload = json.loads(data)
    delta = payload["choices"][0].get("delta", {})
    content = delta.get("content")
    if content:
        parts.append(content)
print("".join(parts))
PY
}

extract_detective_markdown() {
  local sse_file="$1"
  local output_file="$2"
  python3 - "$sse_file" "$output_file" <<'PY'
import json
import sys

markdown = ""
for raw in open(sys.argv[1], encoding="utf-8", errors="ignore"):
    line = raw.strip()
    if not line.startswith("data: "):
        continue
    payload = json.loads(line[6:])
    if payload.get("type") != "detective_stream":
        continue
    chunk = payload.get("chunk", "")
    if payload.get("replace"):
        markdown = chunk
    else:
        markdown += chunk

with open(sys.argv[2], "w", encoding="utf-8") as fh:
    fh.write(markdown)
PY
}

assert_pipeline_quality() {
  local sse_file="$1"
  local markdown_file="$2"
  python3 - "$sse_file" "$markdown_file" <<'PY'
import json
import sys

markdown = open(sys.argv[2], encoding="utf-8").read()
if "RISK_LEVEL:" not in markdown or "CONFIDENCE:" not in markdown:
    raise SystemExit("detective markdown missing footer")

banned_phrases = [
    "chắc chắn là lừa đảo",
    "khẳng định là lừa đảo",
    "là kẻ lừa đảo",
]
lower_markdown = markdown.lower()
for phrase in banned_phrases:
    if phrase in lower_markdown:
        raise SystemExit(f"unsafe conviction phrasing detected: {phrase}")

duration_ms = None
risk_level = None
confidence = None
for raw in open(sys.argv[1], encoding="utf-8", errors="ignore"):
    line = raw.strip()
    if not line.startswith("data: "):
        continue
    payload = json.loads(line[6:])
    if payload.get("type") != "complete":
        continue
    duration_ms = payload.get("duration_ms")
    risk_level = payload.get("risk_level")
    confidence = payload.get("confidence")

if duration_ms is None:
    raise SystemExit("missing complete event")
if duration_ms > 60000:
    raise SystemExit(f"pipeline exceeded 60s: {duration_ms}")

print(json.dumps({
    "duration_ms": duration_ms,
    "risk_level": risk_level,
    "confidence": confidence,
}, ensure_ascii=False))
PY
}

start_postgres() {
  echo "[validate] starting postgres on ${PG_PORT}"
  docker run -d --rm --name "${PG_CONTAINER}" \
    -e POSTGRES_DB=tracuuluadao \
    -e POSTGRES_USER=postgres \
    -e POSTGRES_PASSWORD=postgres \
    -p "127.0.0.1:${PG_PORT}:5432" postgres:16-alpine >/dev/null
  for _ in $(seq 1 30); do
    if docker exec "${PG_CONTAINER}" pg_isready -U postgres -d tracuuluadao >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

start_app() {
  echo "[validate] starting app on ${APP_HOST}:${APP_PORT}"
  APP_PID=""
  APP_HOST="${APP_HOST}" APP_PORT="${APP_PORT}" DATABASE_URL="${DATABASE_URL}" \
    PROXY_DIR="${PROXY_DIR}" AGENT_CONFIG_DIR="${TMP_AGENT_DIR}" \
    QWEN35_ENDPOINT="${QWEN35_ENDPOINT}" QWEN36_ENDPOINT="${QWEN36_ENDPOINT}" \
    cargo run --features tls-impersonation >"${APP_LOG}" 2>&1 &
  APP_PID=$!
  for _ in $(seq 1 60); do
    if curl -fsS "http://${APP_HOST}:${APP_PORT}/health" >"${TMP_DIR}/health.json" 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  tail -n 40 "${APP_LOG}" >&2 || true
  return 1
}

stop_app() {
  if [[ -n "${APP_PID}" ]] && kill -0 "${APP_PID}" 2>/dev/null; then
    kill "${APP_PID}" 2>/dev/null || true
    wait "${APP_PID}" 2>/dev/null || true
  fi
  APP_PID=""
}

run_investigate() {
  local query="$1"
  local query_type="$2"
  local outfile="$3"
  echo "[validate] investigate ${query_type}: ${query}"
  curl -sS -N --max-time 75 \
    "http://${APP_HOST}:${APP_PORT}/api/investigate?q=$(urlencode "$query")&type=${query_type}" \
    >"${outfile}"
}

mkdir -p "$(dirname "${REPORT}")"
write_report "# Live Validation ${STAMP}"
write_report

start_postgres

curl -sS "${QWEN35_ENDPOINT}" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.5-4b","messages":[{"role":"user","content":"Trả JSON duy nhất: {\"ok\":true}"}],"temperature":0,"max_tokens":32,"stream":false,"response_format":{"type":"json_object"},"chat_template_kwargs":{"enable_thinking":false}}' \
  > "${TMP_DIR}/qwen35.json"
curl -sS -N "${QWEN36_ENDPOINT}" \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.6-27b","messages":[{"role":"user","content":"Viết đúng 1 câu tiếng Việt về kiểm tra lừa đảo."}],"temperature":0,"max_tokens":64,"stream":true,"chat_template_kwargs":{"enable_thinking":false}}' \
  > "${TMP_DIR}/qwen36.stream"
for idx in 1 2 3 4; do
  curl -sS "${QWEN35_ENDPOINT}" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"qwen3.5-4b\",\"messages\":[{\"role\":\"user\",\"content\":\"Trả JSON duy nhất: {\\\"id\\\":${idx},\\\"ok\\\":true}\"}],\"temperature\":0,\"max_tokens\":48,\"stream\":false,\"response_format\":{\"type\":\"json_object\"},\"chat_template_kwargs\":{\"enable_thinking\":false}}" \
    > "${TMP_DIR}/parallel-${idx}.json" &
done
wait

DIRECT_27B_TEXT="$(extract_openai_stream_text "${TMP_DIR}/qwen36.stream")"
DIRECT_27B_TEXT_COMPACT="$(echo "${DIRECT_27B_TEXT}" | tr '\n' ' ' | sed 's/[[:space:]]\\+/ /g' | sed 's/^ //; s/ $//')"

write_report "## Direct Models"
write_report "- 4B direct JSON parseable: $(assert_direct_json_response "${TMP_DIR}/qwen35.json" && echo PASS || echo FAIL)"
write_report "- 27B direct stream non-empty: $([[ -n "${DIRECT_27B_TEXT_COMPACT}" ]] && echo PASS || echo FAIL)"
write_report "- 27B direct stream preview: ${DIRECT_27B_TEXT_COMPACT}"
write_report "- 4 parallel 4B calls parseable: $(for idx in 1 2 3 4; do assert_direct_json_response "${TMP_DIR}/parallel-${idx}.json" "${idx}"; done && echo PASS || echo FAIL)"
write_report

start_app
require_contains "${TMP_DIR}/health.json" '"cache_enabled":true'
write_report "## App Boot"
write_report "- Health: $(cat "${TMP_DIR}/health.json")"

run_investigate "${QUERY_PHONE}" phone "${TMP_DIR}/phone-1.sse"
echo "[validate] phone first run complete"
require_contains "${TMP_DIR}/phone-1.sse" 'event: complete'
require_contains "${TMP_DIR}/phone-1.sse" 'event: detective_stream'
extract_detective_markdown "${TMP_DIR}/phone-1.sse" "${TMP_DIR}/phone-1.detective.md"
cp "${TMP_DIR}/phone-1.detective.md" "${DETECTIVE_ARTIFACT}"
PHONE_QUALITY_JSON="$(assert_pipeline_quality "${TMP_DIR}/phone-1.sse" "${TMP_DIR}/phone-1.detective.md")"

SOURCE_TO_REFRESH="$(pg "select source from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' order by source limit 1")"
SOURCE_BEFORE="$(pg "select coalesce(max(extract(epoch from created_at)),0) from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source='${SOURCE_TO_REFRESH}'")"
OTHER_SOURCE="$(pg "select source from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source <> '${SOURCE_TO_REFRESH}' order by source limit 1")"
OTHER_BEFORE="$(pg "select coalesce(max(extract(epoch from created_at)),0) from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source='${OTHER_SOURCE}'")"
echo "[validate] source refresh candidate=${SOURCE_TO_REFRESH}, other=${OTHER_SOURCE}"

run_investigate "${QUERY_PHONE}" phone "${TMP_DIR}/phone-2.sse"
echo "[validate] phone second run complete"
require_contains "${TMP_DIR}/phone-2.sse" 'Cache hit'

pg "delete from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source='${SOURCE_TO_REFRESH}'; delete from investigation_cache where query='${QUERY_PHONE}' and query_type='phone';"
run_investigate "${QUERY_PHONE}" phone "${TMP_DIR}/phone-3.sse"
echo "[validate] phone refresh run complete"
require_contains "${TMP_DIR}/phone-3.sse" 'event: complete'
SOURCE_AFTER="$(pg "select coalesce(max(extract(epoch from created_at)),0) from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source='${SOURCE_TO_REFRESH}'")"
OTHER_AFTER="$(pg "select coalesce(max(extract(epoch from created_at)),0) from scrape_cache where query='${QUERY_PHONE}' and query_type='phone' and source='${OTHER_SOURCE}'")"
echo "[validate] source timestamps before=${SOURCE_BEFORE}/${OTHER_BEFORE} after=${SOURCE_AFTER}/${OTHER_AFTER}"

pg "insert into scrape_cache(query, query_type, source, result, expires_at) values ('expired-phone','phone','ExpiredSource','{}', now() - interval '1 hour');"
pg "insert into analysis_cache(query, agent_name, prompt_hash, input_hash, result, expires_at) values ('expired-query','summarizer','old','old','{}', now() - interval '1 hour');"
pg "insert into investigation_cache(query, query_type, prompt_hash, risk_level, full_result, expires_at) values ('expired-query','phone','old','low','{}', now() - interval '1 hour');"
stop_app
start_app
EXPIRED_LEFT="$(pg "select count(*) from scrape_cache where query='expired-phone' union all select count(*) from analysis_cache where query='expired-query' union all select count(*) from investigation_cache where query='expired-query'")"
EXPIRED_LEFT_COMPACT="$(echo "${EXPIRED_LEFT}" | paste -sd, -)"
echo "[validate] cleanup counts=${EXPIRED_LEFT_COMPACT}"

printf '\n<!-- validation hash change -->\n' >> "${TMP_AGENT_DIR}/detective/prompt.md"
stop_app
start_app
run_investigate "${QUERY_PHONE}" phone "${TMP_DIR}/phone-4.sse"
echo "[validate] phone prompt-hash run complete"
if require_contains "${TMP_DIR}/phone-4.sse" 'Cache hit'; then
  echo "prompt hash invalidation failed" >&2
  exit 1
fi
require_contains "${TMP_DIR}/phone-4.sse" 'event: complete'

run_investigate "${QUERY_BANK}" bank "${TMP_DIR}/bank.sse"
run_investigate "${QUERY_URL}" url "${TMP_DIR}/url.sse"
echo "[validate] bank/url runs complete"
require_contains "${TMP_DIR}/bank.sse" 'event: complete'
require_contains "${TMP_DIR}/url.sse" 'event: complete'
cp "${APP_LOG}" "${APP_LOG_ARTIFACT}"

write_report "## Full Pipeline"
write_report "- Phone first run complete: PASS"
write_report "- Phone first run quality gate: PASS ${PHONE_QUALITY_JSON}"
write_report "- Phone second run full-investigation cache hit: PASS"
write_report "- One-source scrape refresh: $([[ "${SOURCE_AFTER%%.*}" -gt "${SOURCE_BEFORE%%.*}" && "${OTHER_AFTER}" == "${OTHER_BEFORE}" ]] && echo PASS || echo FAIL)"
write_report "- Startup cleanup removed expired rows: $([[ "${EXPIRED_LEFT_COMPACT}" == "0,0,0" ]] && echo PASS || echo FAIL)"
write_report "- Prompt hash invalidation bypassed old investigation cache: PASS"
write_report "- Bank investigation complete: PASS"
write_report "- URL investigation complete: PASS"
write_report
write_report "## Notes"
write_report "- Source refresh candidate: ${SOURCE_TO_REFRESH}"
write_report "- Comparison source: ${OTHER_SOURCE}"
write_report "- Health payload: $(cat "${TMP_DIR}/health.json")"
write_report "- Detective markdown artifact: ${DETECTIVE_ARTIFACT}"
write_report "- App log artifact: ${APP_LOG_ARTIFACT}"

echo "Report written to ${REPORT}"
