use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::agents::config::{AgentRegistry, LoadedAgent};
use crate::agents::llm_client::{LlmClient, StreamChunk};
use crate::cache::CacheService;
use crate::error::{AppError, AppResult};
use crate::pipeline::state::{
    AgentExtraction, AgentSummary, Investigation, InvestigationEvent, InvestigationResult,
    QueryType, SelectedUrl, UrlAssessmentResponse, collect_search_results,
};
use crate::pipeline::url_fetcher::fetch_page;
use crate::scrapers::http_client::HttpClientFactory;
use crate::scrapers::{ScrapedResult, SearchResult, run_all_scrapers};

const INVESTIGATION_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn run_investigation(
    investigation: Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    proxy_pool: Option<Arc<crate::scrapers::proxy::ProxyPool>>,
    cache: Option<Arc<CacheService>>,
    cancel_token: CancellationToken,
    tx: mpsc::Sender<InvestigationEvent>,
) -> AppResult<InvestigationResult> {
    match tokio::time::timeout(
        INVESTIGATION_TIMEOUT,
        run_investigation_inner(
            investigation,
            registry,
            llm,
            proxy_pool,
            cache,
            cancel_token.clone(),
            tx,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            cancel_token.cancel();
            Err(AppError::Server("investigation timed out".to_string()))
        }
    }
}

async fn run_investigation_inner(
    investigation: Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    proxy_pool: Option<Arc<crate::scrapers::proxy::ProxyPool>>,
    cache: Option<Arc<CacheService>>,
    cancel_token: CancellationToken,
    tx: mpsc::Sender<InvestigationEvent>,
) -> AppResult<InvestigationResult> {
    let prompt_hash = registry.prompt_hash_all()?;
    if let Some(cache) = cache.as_ref() {
        if let Some(cached) = cache
            .get_full_investigation(investigation.query_type, &investigation.query, &prompt_hash)
            .await
            .unwrap_or(None)
        {
            replay_cached_result(&cached, &tx).await;
            return Ok(cached);
        }
    }

    let started_at = Instant::now();
    emit_phase(&tx, 1, "Thu thập dữ liệu", None).await;
    let scraped_results = tokio::select! {
        _ = cancel_token.cancelled() => return Err(cancelled_error()),
        results = run_all_scrapers(
            &investigation.query,
            investigation.query_type,
            proxy_pool.as_deref(),
            cache.as_deref(),
        ) => results,
    };
    for result in &scraped_results {
        tx.send(InvestigationEvent::SourceStatus {
            source: format!("{:?}", result.source),
            status: if result.success { "done" } else { "error" }.to_string(),
            found: result.reports.len() + result.search_results.len(),
        })
        .await
        .ok();
    }

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 2, "Tóm tắt nguồn", None).await;
    let summaries = summarize_sources(
        &investigation,
        registry.clone(),
        llm.clone(),
        &scraped_results,
        cache.as_ref(),
        &tx,
        &cancel_token,
    )
    .await?;

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 3, "Chọn URL điều tra sâu", None).await;
    let search_results = collect_search_results(&scraped_results);
    let selected_urls = assess_urls(
        &investigation,
        registry.clone(),
        llm.clone(),
        &summaries,
        &search_results,
        cache.as_ref(),
        &cancel_token,
    )
    .await
    .unwrap_or_else(|_| fallback_urls(&search_results));
    tx.send(InvestigationEvent::UrlAssessment {
        selected: selected_urls.len(),
        total: search_results.len(),
        urls: selected_urls.clone(),
    })
    .await
    .ok();

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 4, "Phân tích URL", Some(selected_urls.len())).await;
    let extractions = extract_urls(
        registry.clone(),
        llm.clone(),
        &selected_urls,
        cache.as_ref(),
        &tx,
        &cancel_token,
    )
    .await?;

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 5, "Tổng hợp điều tra", None).await;
    let detective_markdown = match detective_report(
        &investigation,
        registry,
        llm,
        &summaries,
        &extractions,
        cache.as_ref(),
        &tx,
        &cancel_token,
    )
    .await
    {
        Ok(markdown) => markdown,
        Err(error) => {
            tx.send(InvestigationEvent::Error {
                phase: Some(5),
                message: error.to_string(),
                recoverable: true,
            })
            .await
            .ok();
            let fallback = build_detective_fallback(
                &investigation,
                &summaries,
                &extractions,
                &scraped_results,
                &error.to_string(),
            );
            tx.send(InvestigationEvent::DetectiveStream {
                chunk: fallback.clone(),
                done: true,
                replace: true,
            })
            .await
            .ok();
            fallback
        }
    };
    let (risk_level, confidence) = parse_footer(&detective_markdown);

    let result = InvestigationResult {
        query: investigation.query.clone(),
        query_type: investigation.query_type,
        risk_level: risk_level.clone(),
        confidence,
        sources_analyzed: scraped_results.len(),
        duration_ms: started_at.elapsed().as_millis() as u64,
        summaries,
        extractions,
        detective_markdown,
        scraped_results,
    };

    tx.send(InvestigationEvent::Complete {
        risk_level,
        confidence,
        sources_analyzed: result.sources_analyzed,
        duration_ms: result.duration_ms,
    })
    .await
    .ok();

    if let Some(cache) = cache.as_ref() {
        if let Err(error) = cache
            .set_full_investigation(
                investigation.query_type,
                &investigation.query,
                &prompt_hash,
                &result,
                1,
            )
            .await
        {
            tracing::warn!("failed to cache full investigation: {error}");
        }
    }

    Ok(result)
}

async fn summarize_sources(
    investigation: &Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    scraped_results: &[crate::scrapers::ScrapedResult],
    cache: Option<&Arc<CacheService>>,
    tx: &mpsc::Sender<InvestigationEvent>,
    cancel_token: &CancellationToken,
) -> AppResult<Vec<AgentSummary>> {
    let agent = registry.get("summarizer")?;
    let limit = Arc::new(Semaphore::new(4));
    let mut tasks = futures::stream::FuturesUnordered::new();
    let cache_query = investigation.query.clone();

    for result in scraped_results.iter().filter(|item| item.success) {
        cancel_token_check(cancel_token)?;
        let input = build_summary_input(&investigation.query, result);
        let tx = tx.clone();
        let llm = llm.clone();
        let agent = agent.clone();
        let cache = cache.cloned();
        let cache_query = cache_query.clone();
        let source = format!("{:?}", result.source);
        let scraped_result = result.clone();
        let permit = limit.clone().acquire_owned().await.map_err(|_| {
            AppError::Server("failed to acquire summarizer concurrency slot".to_string())
        })?;
        let cancel_token = cancel_token.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            tx.send(InvestigationEvent::Progress {
                phase: 2,
                message: format!("Đang phân tích {source}..."),
            })
            .await
            .ok();
            let parsed = call_json_agent_cached::<AgentSummary>(
                cache.as_deref(),
                &cache_query,
                &agent,
                &input,
                1,
                &llm,
                &cancel_token,
            )
            .await;
            (source, scraped_result, parsed)
        }));
    }

    let mut summaries = Vec::new();
    while let Some(task) = futures::StreamExt::next(&mut tasks).await {
        let (source, scraped_result, result) =
            task.map_err(|error| AppError::Server(error.to_string()))?;
        match result {
            Ok(mut summary) => {
                summary.source = source.clone();
                tx.send(InvestigationEvent::SummaryResult {
                    source,
                    result: summary.clone(),
                })
                .await
                .ok();
                summaries.push(summary);
            }
            Err(error) => {
                let fallback = build_summary_fallback(&source, &scraped_result, &error.to_string());
                tx.send(InvestigationEvent::Error {
                    phase: Some(2),
                    message: error.to_string(),
                    recoverable: true,
                })
                .await
                .ok();
                tx.send(InvestigationEvent::SummaryResult {
                    source,
                    result: fallback.clone(),
                })
                .await
                .ok();
                summaries.push(fallback);
            }
        }
    }

    Ok(summaries)
}

fn build_summary_input(query: &str, result: &ScrapedResult) -> String {
    json!({
        "query": query,
        "source": format!("{:?}", result.source),
        "reports": result
            .reports
            .iter()
            .take(4)
            .map(|report| json!({
                "title": trim_text(&report.title, 120),
                "url": trim_text(&report.url, 200),
                "content": trim_text(&report.content, 700),
                "date": report.date,
            }))
            .collect::<Vec<_>>(),
        "search_results": result
            .search_results
            .iter()
            .take(6)
            .map(|item| json!({
                "title": trim_text(&item.title, 140),
                "url": trim_text(&item.url, 220),
                "snippet": trim_text(&item.snippet, 240),
            }))
            .collect::<Vec<_>>(),
        "metadata": compact_json(&result.metadata, 180, 4, 2),
        "raw_html_preview": result.raw_html.as_ref().map(|html| trim_text(html, 500)),
    })
    .to_string()
}

fn build_summary_fallback(
    source: &str,
    result: &ScrapedResult,
    error_message: &str,
) -> AgentSummary {
    let mut key_facts = result
        .reports
        .iter()
        .take(3)
        .map(|report| trim_text(&report.title, 120))
        .collect::<Vec<_>>();
    key_facts.extend(
        result
            .search_results
            .iter()
            .take(3)
            .map(|item| trim_text(&item.title, 120)),
    );

    let mut risk_signals = result
        .search_results
        .iter()
        .filter_map(|item| {
            let snippet = trim_text(&item.snippet, 180);
            (!snippet.is_empty()).then_some(snippet)
        })
        .take(3)
        .collect::<Vec<_>>();
    if risk_signals.is_empty() {
        risk_signals.push(format!("fallback_summary:{error_message}"));
    }

    AgentSummary {
        source: source.to_string(),
        summary: trim_text(
            &format!(
                "Không thể parse JSON summary cho nguồn {source}. Dùng dữ liệu scrape thô để tiếp tục tổng hợp: {}",
                summarize_raw_source(result)
            ),
            320,
        ),
        key_facts,
        phone_mentions: Vec::new(),
        risk_signals,
    }
}

fn summarize_raw_source(result: &ScrapedResult) -> String {
    if let Some(report) = result.reports.first() {
        return trim_text(&report.content, 220);
    }
    if let Some(item) = result.search_results.first() {
        return trim_text(&format!("{} {}", item.title, item.snippet), 220);
    }
    result
        .raw_html
        .as_deref()
        .map(|html| trim_text(html, 220))
        .unwrap_or_else(|| "không có nội dung scrape".to_string())
}

fn compact_json(value: &Value, max_str: usize, max_items: usize, depth: usize) -> Value {
    if depth == 0 {
        return Value::String(trim_text(&value.to_string(), max_str));
    }

    match value {
        Value::String(text) => Value::String(trim_text(text, max_str)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(max_items)
                .map(|item| compact_json(item, max_str, max_items, depth - 1))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .take(max_items)
                .map(|(key, item)| {
                    (
                        key.clone(),
                        compact_json(item, max_str, max_items, depth - 1),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn trim_text(value: &str, max_len: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let trimmed = chars.by_ref().take(max_len).collect::<String>();
    if chars.next().is_some() {
        format!("{trimmed}...")
    } else {
        trimmed
    }
}

async fn assess_urls(
    investigation: &Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    summaries: &[AgentSummary],
    search_results: &[SearchResult],
    cache: Option<&Arc<CacheService>>,
    cancel_token: &CancellationToken,
) -> AppResult<Vec<SelectedUrl>> {
    let agent = registry.get("url-assessor")?;
    let payload = json!({
        "query": investigation.query,
        "query_type": investigation.query_type.as_str(),
        "summaries": summaries,
        "search_results": search_results,
    })
    .to_string();
    let response = call_json_agent_cached::<UrlAssessmentResponse>(
        cache.map(Arc::as_ref),
        &investigation.query,
        &agent,
        &payload,
        1,
        &llm,
        cancel_token,
    )
    .await?;
    if response.urls.is_empty() {
        return Err(AppError::Server(
            "url assessor returned no investigation targets".to_string(),
        ));
    }
    Ok(response.urls.into_iter().take(5).collect())
}

async fn extract_urls(
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    urls: &[SelectedUrl],
    cache: Option<&Arc<CacheService>>,
    tx: &mpsc::Sender<InvestigationEvent>,
    cancel_token: &CancellationToken,
) -> AppResult<Vec<AgentExtraction>> {
    let extractor = registry.get("extractor")?;
    let http = HttpClientFactory::default()
        .standard_client()
        .map_err(|error| AppError::Server(error.to_string()))?;
    let mut tasks = futures::stream::FuturesUnordered::new();

    for selected in urls.iter().cloned() {
        cancel_token_check(cancel_token)?;
        let tx = tx.clone();
        let extractor = extractor.clone();
        let http = http.clone();
        let llm = llm.clone();
        let cache = cache.cloned();
        let cancel_token = cancel_token.clone();
        tasks.push(tokio::spawn(async move {
            tx.send(InvestigationEvent::Progress {
                phase: 4,
                message: format!("Đang phân tích {}...", selected.url),
            })
            .await
            .ok();
            let page = tokio::select! {
                _ = cancel_token.cancelled() => return Err(cancelled_error()),
                page = fetch_page(&http, &selected.url) => page,
            }?;
            let payload = json!({ "url": page.url, "content": page.content }).to_string();
            let mut extraction = call_json_agent_cached::<AgentExtraction>(
                cache.as_deref(),
                &selected.url,
                &extractor,
                &payload,
                1,
                &llm,
                &cancel_token,
            )
            .await?;
            extraction.url = selected.url.clone();
            Ok::<_, AppError>(extraction)
        }));
    }

    let mut extractions = Vec::new();
    while let Some(task) = futures::StreamExt::next(&mut tasks).await {
        match task.map_err(|error| AppError::Server(error.to_string()))? {
            Ok(extraction) => {
                tx.send(InvestigationEvent::ExtractionResult {
                    url: extraction.url.clone(),
                    result: extraction.clone(),
                })
                .await
                .ok();
                extractions.push(extraction);
            }
            Err(error) => {
                tx.send(InvestigationEvent::Error {
                    phase: Some(4),
                    message: error.to_string(),
                    recoverable: true,
                })
                .await
                .ok();
            }
        }
    }

    Ok(extractions)
}

async fn detective_report(
    investigation: &Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    summaries: &[AgentSummary],
    extractions: &[AgentExtraction],
    cache: Option<&Arc<CacheService>>,
    tx: &mpsc::Sender<InvestigationEvent>,
    cancel_token: &CancellationToken,
) -> AppResult<String> {
    let agent = registry.get("detective")?;
    let payload = json!({
        "query": investigation.query,
        "query_type": investigation.query_type.as_str(),
        "summaries": summaries,
        "extractions": extractions,
    })
    .to_string();
    let input_hash = hash_text(&payload);
    if let Some(cache) = cache {
        if let Some(cached) = cache
            .get_analysis(
                investigation.query.as_str(),
                &agent.key,
                &agent.prompt_hash,
                &input_hash,
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            return Ok(cached);
        }
    }
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<StreamChunk>(64);
    let stream_agent = agent.clone();
    let stream_payload = payload.clone();
    let llm_task = tokio::spawn(async move {
        llm.stream_text(&stream_agent, &stream_payload, chunk_tx)
            .await
    });

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                llm_task.abort();
                return Err(cancelled_error());
            }
            chunk = chunk_rx.recv() => {
                let Some(chunk) = chunk else {
                    break;
                };
                tx.send(InvestigationEvent::DetectiveStream {
                    chunk: chunk.content,
                    done: chunk.done,
                    replace: false,
                })
                .await
                .ok();
            }
        }
    }

    let markdown = llm_task
        .await
        .map_err(|error| AppError::Server(error.to_string()))??;
    if let Some(cache) = cache {
        if let Err(error) = cache
            .set_analysis(
                investigation.query.as_str(),
                &agent.key,
                &agent.prompt_hash,
                &input_hash,
                &serde_json::Value::String(markdown.clone()),
                1,
            )
            .await
        {
            tracing::warn!("failed to cache detective report: {error}");
        }
    }
    Ok(markdown)
}

fn fallback_urls(results: &[SearchResult]) -> Vec<SelectedUrl> {
    results
        .iter()
        .take(5)
        .enumerate()
        .map(|(index, item)| SelectedUrl {
            url: item.url.clone(),
            reason: "fallback".to_string(),
            priority: (index + 1) as u8,
        })
        .collect()
}

fn parse_footer(markdown: &str) -> (String, f32) {
    let risk = markdown
        .lines()
        .find_map(|line| line.strip_prefix("RISK_LEVEL: "))
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let confidence = markdown
        .lines()
        .find_map(|line| line.strip_prefix("CONFIDENCE: "))
        .and_then(|value| value.trim().parse::<f32>().ok())
        .unwrap_or(0.5);
    (risk, confidence)
}

fn cancel_token_check(cancel_token: &CancellationToken) -> AppResult<()> {
    if cancel_token.is_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn cancelled_error() -> AppError {
    AppError::Server("request cancelled".to_string())
}

async fn emit_phase(
    tx: &mpsc::Sender<InvestigationEvent>,
    phase: u8,
    label: &str,
    total_sources: Option<usize>,
) {
    tx.send(InvestigationEvent::PhaseStart {
        phase,
        label: label.to_string(),
        total_sources,
    })
    .await
    .ok();
}

#[allow(dead_code)]
fn _query_type_label(query_type: QueryType) -> &'static str {
    match query_type {
        QueryType::Phone => "Số điện thoại",
        QueryType::Bank => "Tài khoản ngân hàng",
        QueryType::Url => "URL",
    }
}

async fn replay_cached_result(cached: &InvestigationResult, tx: &mpsc::Sender<InvestigationEvent>) {
    emit_phase(tx, 1, "Cache hit", Some(cached.scraped_results.len())).await;
    for result in &cached.scraped_results {
        tx.send(InvestigationEvent::SourceStatus {
            source: format!("{:?}", result.source),
            status: if result.success { "done" } else { "error" }.to_string(),
            found: result.reports.len() + result.search_results.len(),
        })
        .await
        .ok();
    }

    emit_phase(tx, 2, "Tóm tắt nguồn", Some(cached.summaries.len())).await;
    for summary in &cached.summaries {
        tx.send(InvestigationEvent::SummaryResult {
            source: summary.source.clone(),
            result: summary.clone(),
        })
        .await
        .ok();
    }

    let search_results = collect_search_results(&cached.scraped_results);
    let selected_urls = cached
        .extractions
        .iter()
        .enumerate()
        .map(|(index, extraction)| SelectedUrl {
            url: extraction.url.clone(),
            reason: "cached".to_string(),
            priority: (index + 1) as u8,
        })
        .collect::<Vec<_>>();
    emit_phase(tx, 3, "Chọn URL điều tra sâu", None).await;
    tx.send(InvestigationEvent::UrlAssessment {
        selected: selected_urls.len(),
        total: search_results.len(),
        urls: selected_urls,
    })
    .await
    .ok();

    emit_phase(tx, 4, "Phân tích URL", Some(cached.extractions.len())).await;
    for extraction in &cached.extractions {
        tx.send(InvestigationEvent::ExtractionResult {
            url: extraction.url.clone(),
            result: extraction.clone(),
        })
        .await
        .ok();
    }

    emit_phase(tx, 5, "Tổng hợp điều tra", None).await;
    tx.send(InvestigationEvent::DetectiveStream {
        chunk: cached.detective_markdown.clone(),
        done: true,
        replace: true,
    })
    .await
    .ok();
    tx.send(InvestigationEvent::Complete {
        risk_level: cached.risk_level.clone(),
        confidence: cached.confidence,
        sources_analyzed: cached.sources_analyzed,
        duration_ms: cached.duration_ms,
    })
    .await
    .ok();
}

fn build_detective_fallback(
    investigation: &Investigation,
    summaries: &[AgentSummary],
    extractions: &[AgentExtraction],
    scraped_results: &[ScrapedResult],
    error_message: &str,
) -> String {
    let summary_signals = summaries
        .iter()
        .flat_map(|item| item.risk_signals.iter())
        .cloned()
        .collect::<Vec<_>>();
    let extraction_signals = extractions
        .iter()
        .flat_map(|item| item.risk_signals.iter())
        .cloned()
        .collect::<Vec<_>>();
    let risk_signal_count = summary_signals.len() + extraction_signals.len();
    let successful_sources = scraped_results.iter().filter(|item| item.success).count();
    let risk_level = if risk_signal_count >= 4 {
        "high"
    } else if risk_signal_count >= 2 {
        "medium"
    } else if successful_sources == 0 {
        "unknown"
    } else {
        "low"
    };
    let confidence = if successful_sources == 0 {
        0.1
    } else if risk_signal_count >= 4 {
        0.6
    } else if risk_signal_count >= 2 {
        0.45
    } else {
        0.3
    };

    let key_facts = summaries
        .iter()
        .flat_map(|item| item.key_facts.iter())
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    let related_entities = extractions
        .iter()
        .flat_map(|item| item.entities.iter())
        .take(5)
        .cloned()
        .collect::<Vec<_>>();

    format!(
        "## Kết quả điều tra tạm thời\n\n\
Không thể hoàn tất bước tổng hợp bằng model lớn, nên báo cáo này được dựng từ dữ liệu đã scrape và các kết quả agent trung gian.\n\n\
### Truy vấn\n\n\
- Query: `{}`\n\
- Loại: `{}`\n\
- Nguồn scrape thành công: {}/{}\n\n\
### Tín hiệu đã thu thập\n\n\
- Risk signals từ summary: {}\n\
- Risk signals từ extraction: {}\n\
- Key facts mẫu: {}\n\
- Entities mẫu: {}\n\n\
### Ghi chú degrade\n\n\
- Lý do fallback: {}\n\
- Báo cáo này chỉ là kết quả degrade để không làm mất dữ liệu đã thu thập.\n\n\
RISK_LEVEL: {}\n\
CONFIDENCE: {:.2}",
        investigation.query,
        investigation.query_type.as_str(),
        successful_sources,
        scraped_results.len(),
        join_or_none(&summary_signals),
        join_or_none(&extraction_signals),
        join_or_none(&key_facts),
        join_or_none(&related_entities),
        error_message,
        risk_level,
        confidence
    )
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "không có".to_string()
    } else {
        values.join("; ")
    }
}

async fn call_json_agent_cached<T: serde::de::DeserializeOwned + serde::Serialize>(
    cache: Option<&CacheService>,
    cache_query: &str,
    agent: &LoadedAgent,
    input: &str,
    ttl_hours: i64,
    llm: &LlmClient,
    cancel_token: &CancellationToken,
) -> AppResult<T> {
    let input_hash = hash_text(input);
    if let Some(cache) = cache {
        if let Some(cached) = cache
            .get_analysis(cache_query, &agent.key, &agent.prompt_hash, &input_hash)
            .await
            .ok()
            .flatten()
        {
            return serde_json::from_value(cached).map_err(Into::into);
        }
    }

    let parsed = tokio::select! {
        _ = cancel_token.cancelled() => Err(cancelled_error()),
        result = llm.complete_json::<T>(agent, input) => result,
    }?;

    if let Some(cache) = cache {
        let value = serde_json::to_value(&parsed)?;
        if let Err(error) = cache
            .set_analysis(
                cache_query,
                &agent.key,
                &agent.prompt_hash,
                &input_hash,
                &value,
                ttl_hours,
            )
            .await
        {
            tracing::warn!("failed to cache agent result for {}: {error}", agent.key);
        }
    }

    Ok(parsed)
}

fn hash_text(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_detective_report_sets_high_risk_when_many_signals_exist() {
        let investigation = Investigation {
            query: "0926408013".to_string(),
            query_type: QueryType::Phone,
        };
        let summaries = vec![AgentSummary {
            source: "CheckScam".to_string(),
            summary: "summary".to_string(),
            key_facts: vec!["co canh bao".to_string()],
            phone_mentions: vec![],
            risk_signals: vec!["signal-1".to_string(), "signal-2".to_string()],
        }];
        let extractions = vec![AgentExtraction {
            url: "https://example.com".to_string(),
            summary: "summary".to_string(),
            entities: vec!["Nguyen Van A".to_string()],
            risk_signals: vec!["signal-3".to_string(), "signal-4".to_string()],
            related_numbers: vec![],
        }];
        let scraped_results = vec![ScrapedResult {
            source: crate::scrapers::SourceName::Google,
            query: investigation.query.clone(),
            success: true,
            reports: vec![],
            search_results: vec![],
            metadata: Value::Null,
            raw_html: None,
            duration_ms: 1,
            error: None,
        }];

        let markdown = build_detective_fallback(
            &investigation,
            &summaries,
            &extractions,
            &scraped_results,
            "upstream failed",
        );

        assert!(markdown.contains("RISK_LEVEL: high"));
        assert!(markdown.contains("CONFIDENCE: 0.60"));
        assert!(markdown.contains("upstream failed"));
    }
}
