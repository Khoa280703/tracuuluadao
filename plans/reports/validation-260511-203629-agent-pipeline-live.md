# Live Validation 260511-203629

## Direct Models
- 4B direct JSON parseable: PASS
- 27B direct stream non-empty: PASS
- 27B direct stream preview: Việc kiểm tra kỹ lưỡng thông tin nguồn gốc và tính xác thực của các giao dịch trực tuyến là biện pháp thiết yếu để phòng ngừa và phát hiện sớm các hành vi lừa đảo.
- 4 parallel 4B calls parseable: PASS

## App Boot
- Health: {"ok":true,"proxies_loaded":true,"uptime_seconds":0,"cache_enabled":true}
## Full Pipeline
- Phone first run complete: PASS
- Phone first run quality gate: PASS {"duration_ms": 42292, "risk_level": "high", "confidence": 0.85}
- Phone second run full-investigation cache hit: PASS
- One-source scrape refresh: PASS
- Startup cleanup removed expired rows: PASS
- Prompt hash invalidation bypassed old investigation cache: PASS
- Bank investigation complete: PASS
- URL investigation complete: PASS

## Notes
- Source refresh candidate: ChongLuaDao
- Comparison source: DuckDuckGo
- Health payload: {"ok":true,"proxies_loaded":true,"uptime_seconds":0,"cache_enabled":true}
- Detective markdown artifact: plans/reports/validation-260511-203629-phone-detective.md
- App log artifact: plans/reports/validation-260511-203629-app.log
