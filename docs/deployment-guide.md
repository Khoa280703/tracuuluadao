# Deployment Guide

> Last updated: 2026-05-25

## Production Architecture

- `Cloudflare -> Traefik/Coolify -> frontend`
- `Cloudflare -> Traefik/Coolify -> backend`
- `frontend` và `backend` chạy trong Docker qua Coolify
- `postgres` lưu cả cache TTL lẫn persistent knowledge base
- `redis` dùng để buffer/replay báo cáo điều tra theo `investigation_id`
- model nhỏ chạy host-side ở port `8001` bằng `llama.cpp`
- model lớn chạy host-side ở port `8101` bằng `llama.cpp`
- agent configs trong repo gọi thẳng model host-side qua `host.docker.internal`

## Included Files

- `docker-compose.yml` — stack để import vào Coolify
- `Dockerfile.backend` — build backend Rust
- `frontend/Dockerfile` — build frontend SvelteKit adapter-node
- `.env.coolify.example` — biến môi trường mẫu cho stack Coolify
- `deploy/systemd/*.service` — service mẫu để chạy backend local + 2 lane model lúc boot
- `deploy/systemd/tracuuluadao-models.env.example` — file mẫu để override path, model, GPU, port theo server thật

## Runtime Requirements

| Dependency | Required | Purpose |
|------------|----------|---------|
| PostgreSQL | Yes | cache + persistent knowledge base |
| Redis | No | buffered report replay |
| `ADMIN_API_KEY` | No | admin moderation APIs |
| Writable `data/media/` path | No for web runtime, yes for crawler runs | downloaded evidence files referenced by `media.file_path` |
| Small GGUF endpoint (`8001`) | Yes | structured agent calls |
| Large GGUF endpoint (`8101`) | Yes | streamed detective report |

Nếu `DATABASE_URL` không có hoặc database init lỗi, app vẫn boot nhưng sẽ tắt cả cache và knowledge base.

## Coolify Setup

### Preferred: frontend + backend public riêng

1. Tạo app mới từ repo này bằng `Docker Compose`.
2. Chọn file compose: `docker-compose.yml`.
3. Public `frontend` bằng domain web chính.
4. Public `backend` bằng domain API riêng, ví dụ `api.example.com`.
5. Set các env sau trong Coolify:

```env
POSTGRES_PASSWORD=change-me
REDIS_URL=redis://redis:6379/
INVESTIGATION_REPORT_TTL_SECS=3600
RUST_LOG=info
VITE_API_BASE_URL=https://api.example.com
```

`VITE_API_BASE_URL` là biến build-time của frontend. Khi set biến này, web sẽ gọi trực tiếp backend public.

`docker-compose.yml` hiện tự build:
- `DATABASE_URL=postgres://postgres:${POSTGRES_PASSWORD}@postgres:5432/tracuuluadao`
- `REDIS_URL=${REDIS_URL:-redis://redis:6379/}`
- `ADMIN_API_KEY=${ADMIN_API_KEY:-}`
- `VITE_API_BASE_URL=${VITE_API_BASE_URL:-}`

Hiện tại `docker-compose.yml` chưa mount volume riêng cho `data/media/`. Nếu chạy `checkscam-crawler` bên trong container `backend`, file media tải về sẽ nằm trong filesystem của container trừ khi bạn thêm bind mount hoặc named volume override.

## Bulk Crawler Operation

`checkscam-crawler` là binary vận hành riêng để seed knowledge base, không phải background task mặc định của web server.

Yêu cầu tối thiểu:
- `DATABASE_URL` hợp lệ
- process có quyền ghi vào `data/media/` hoặc thư mục truyền qua `--media-dir`
- outbound HTTP tới `checkscam.vn`

Ví dụ dry run:

```bash
cargo run --bin checkscam-crawler -- \
  --database-url "$DATABASE_URL" \
  --dry-run \
  --max-pages 2
```

Ví dụ ingest thật:

```bash
cargo run --bin checkscam-crawler -- \
  --database-url "$DATABASE_URL" \
  --concurrency 3 \
  --delay-ms 200 \
  --media-dir data/media
```

Hành vi runtime đáng chú ý:
- crawler đọc WordPress REST API `https://checkscam.vn/wp-json/wp/v2/posts`
- nếu sidecar detail endpoint reachable, crawler ưu tiên HTML detail; nếu không, nó fallback sang `content.rendered`
- media chỉ lấy từ ảnh upload của `checkscam.vn` và ghi dưới `data/media/evidence/{evidence_id}/`
- `--resume` bật mặc định để skip post đã có đủ evidence/media

## Admin API Deployment Note

Knowledge base moderation endpoints dùng env `ADMIN_API_KEY` và Compose stack hiện đã forward biến này vào service `backend`.

Lưu ý:
- `POST /api/reports` chỉ cần PostgreSQL để hoạt động
- `/api/admin/reports*` chỉ hoạt động khi `ADMIN_API_KEY` được set khác rỗng trong môi trường deploy

## Host Model Services

Hai file service mẫu trong `deploy/systemd/` dùng tên generic để sau này đổi model không cần đổi service name:

- small model: `Qwen3.5-9B-UD-Q8_K_XL.gguf`
- large model: `Qwopus3.6-35B-A3B-v1-Q3_K_L.gguf`

Path repo và binary không còn hardcode vào unit nữa. Cách làm:

1. copy file env mẫu
2. sửa path theo server thật
3. enable 2 service

```bash
sudo cp deploy/systemd/tracuuluadao-models.env.example /etc/default/tracuuluadao-models
sudo nano /etc/default/tracuuluadao-models
```

Host prerequisites:

```bash
source /etc/default/tracuuluadao-models
test -x "$LLAMA_SERVER_BIN"
test -f "$TRACUULUADAO_SMALL_MODEL_PATH"
test -f "$TRACUULUADAO_LARGE_MODEL_PATH"
```

Copy service files:

```bash
sudo cp deploy/systemd/tracuuluadao-small-llm.service /etc/systemd/system/
sudo cp deploy/systemd/tracuuluadao-large-llm.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now tracuuluadao-small-llm tracuuluadao-large-llm
```

Các unit này chạy:

- `tracuuluadao-small-llm.service` -> `${LOCAL_MODELS_ROOT}/scripts/gguf/run-gguf-host-llama-server.sh`
- `tracuuluadao-large-llm.service` -> `${LOCAL_MODELS_ROOT}/scripts/qwopus36/run-qwopus36-35b-a3b-gguf.sh`

Với runtime chính:

- small model -> `GPU 0`, `port 8001`, `ctx 262144`, `parallel 8`
- large model -> `GPU 1`, `port 8101`, `ctx 262144`, `parallel 1`

Check health:

```bash
curl http://127.0.0.1:8001/v1/models
curl http://127.0.0.1:8101/v1/models
```

Nếu service restart fail ngay sau khi enable, check port conflict trước:

```bash
sudo ss -ltnp '( sport = :8001 or sport = :8101 )'
sudo systemctl status tracuuluadao-small-llm --no-pager
sudo systemctl status tracuuluadao-large-llm --no-pager
```

Nếu thấy một `llama-server` cũ đang chiếm port mà không thuộc `systemd`, stop process đó rồi restart service tương ứng.

Smoke test từng lane:

```bash
GGUF_HOST_PORT=8001 GGUF_HOST_MODEL_NAME=Qwen3.5-9B-UD-Q8_K_XL.gguf \
  "${LOCAL_MODELS_ROOT}/scripts/gguf/smoke-test-gguf-host-llama-server.sh"

GGUF_HOST_PORT=8101 GGUF_HOST_MODEL_NAME=Qwopus3.6-35B-A3B-v1-Q3_K_L.gguf \
  "${LOCAL_MODELS_ROOT}/scripts/qwopus36/smoke-test-qwopus36-35b-a3b-gguf.sh"
```

## Host Backend Service

Build binary release:

```bash
cargo build --release
```

Copy backend service:

```bash
sudo cp deploy/systemd/tracuuluadao-backend.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now tracuuluadao-backend
```

Check service:

```bash
systemctl status tracuuluadao-backend --no-pager
journalctl -u tracuuluadao-backend -f
```

## Database Bootstrap Behavior

Khi `DATABASE_URL` hợp lệ:
- cache schema được tạo tự động qua `CacheService::ensure_schema()`
- knowledge base schema được tạo tự động qua `KnowledgeBase::ensure_schema()`
- materialized view `subject_risk_overview` được refresh lúc startup và mỗi 15 phút

Không có migration CLI riêng cho feature knowledge base ở thời điểm hiện tại.

## Logs & Operational Signals

Backend ghi file log xoay vòng theo ngày vào:

```bash
ls -lah /var/log/tracuuluadao
tail -f /var/log/tracuuluadao/backend.log.$(date +%F)
```

Biến môi trường log hữu ích:

```env
RUST_LOG=info
APP_LOG_DIR=/var/log/tracuuluadao
APP_LOG_FILE_PREFIX=backend.log
```

Log backend mới nên có các mốc:
- request nhận vào với `request_id`
- knowledge base enabled/disabled
- historical subject lookup thành công/thất bại
- cache hit
- scraper nào xong, thành công/thất bại, tìm được bao nhiêu kết quả
- report replay buffer init thành công/thất bại
- knowledge base ingest thành công/thất bại
- hoàn tất với `risk_level`, `confidence`, `duration_ms`

## Notes

- Không cần `QWEN35_ENDPOINT` hay `QWEN36_ENDPOINT` nữa; agent configs đã trỏ sẵn tới `host.docker.internal:8001` và `host.docker.internal:8101`.
- Nếu đổi port model host-side, cập nhật trực tiếp các file `config/agents/*/config.toml`.
- Nếu đổi path repo hoặc path `llama-server`, chỉ cần sửa `/etc/default/tracuuluadao-models`, không cần sửa unit file.
- Hai model GGUF hiện deploy qua repo này là:
  - `models/qwen35-9b-mtp-gguf/Qwen3.5-9B-UD-Q8_K_XL.gguf`
  - `models/qwopus3.6-35b-a3b-v1-gguf/Qwopus3.6-35B-A3B-v1-Q3_K_L.gguf`
- Không expose `8001` hoặc `8101` qua Traefik/Coolify/Cloudflare. Chỉ expose frontend/backend public; backend container sẽ gọi model qua `host.docker.internal`.
- Backend container cần model service bind trên host `0.0.0.0`, không chỉ `127.0.0.1`, để `host.docker.internal` truy cập được.
- Nếu firewall đang mở LAN, chỉ allow truy cập `8001` và `8101` từ host/container nội bộ.
- Frontend production nên luôn set `VITE_API_BASE_URL` để gọi trực tiếp backend public.
- `POST /api/reports` phụ thuộc PostgreSQL vì user reports nằm trong persistent knowledge base.
