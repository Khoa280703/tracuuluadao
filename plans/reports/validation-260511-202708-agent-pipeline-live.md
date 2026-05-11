# Live Validation 260511-202708

## Direct Models
- 4B direct JSON: PASS
- 27B direct stream: PASS
- 4 parallel 4B calls: PASS

## App Boot
- Health: {"ok":true,"proxies_loaded":true,"uptime_seconds":0,"cache_enabled":true}
## Full Pipeline
- Phone first run complete: PASS
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
- App log: /tmp/tmp.0PwVeV1Iht/app.log
