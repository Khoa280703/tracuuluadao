use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::agents::config::{AgentRegistry, LoadedAgent};
use crate::agents::llm_client::{LlmClient, StreamChunk};
use crate::cache::CacheService;
use crate::error::{AppError, AppResult};
use crate::knowledge_base::{KnowledgeBase, SubjectHistory};
use crate::pipeline::knowledge_base::{
    detective_history_payload, load_subject_history, schedule_ingest,
};
use crate::pipeline::state::{
    AgentExtraction, AgentSummary, Investigation, InvestigationEvent, InvestigationResult,
    QueryType, SelectedUrl, UrlAssessmentResponse,
};
use crate::pipeline::url_fetcher::fetch_page;
use crate::report_store::InvestigationReportStore;
use crate::scrapers::http_client::HttpClientFactory;
use crate::scrapers::{ScrapedResult, ScraperProgress, SearchResult, SourceName, run_all_scrapers};

const INVESTIGATION_TIMEOUT: Duration = Duration::from_secs(180);
const INVESTIGATION_CACHE_VERSION: &str = "investigation-v5-buffered-detective-report";
const SEARCH_CANDIDATE_LIMIT: usize = 30;
const SELECTED_URL_LIMIT: usize = 10;
const URL_ASSESSOR_SUMMARY_LIMIT: usize = 6;

struct DetectiveReportOutput {
    markdown: String,
    wrote_stream_chunks: bool,
}

fn investigation_log_fields(investigation: &Investigation) -> (&str, &str, &str) {
    (
        investigation.request_id.as_str(),
        investigation.query.as_str(),
        investigation.query_type.as_str(),
    )
}

pub async fn run_investigation(
    investigation: Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    proxy_pool: Option<Arc<crate::scrapers::proxy::ProxyPool>>,
    cache: Option<Arc<CacheService>>,
    knowledge_base: Option<Arc<KnowledgeBase>>,
    report_store: Option<Arc<InvestigationReportStore>>,
    cancel_token: CancellationToken,
    tx: mpsc::Sender<InvestigationEvent>,
) -> AppResult<InvestigationResult> {
    let request_id = investigation.request_id.clone();
    let query = investigation.query.clone();
    let query_type = investigation.query_type;
    tracing::info!(request_id = %request_id, query = %query, query_type = query_type.as_str(), "investigation pipeline started");
    match tokio::time::timeout(
        INVESTIGATION_TIMEOUT,
        run_investigation_inner(
            investigation,
            registry,
            llm,
            proxy_pool,
            cache,
            knowledge_base,
            report_store,
            cancel_token.clone(),
            tx,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            cancel_token.cancel();
            tracing::error!(request_id = %request_id, query = %query, query_type = query_type.as_str(), timeout_secs = INVESTIGATION_TIMEOUT.as_secs(), "investigation timed out");
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
    knowledge_base: Option<Arc<KnowledgeBase>>,
    report_store: Option<Arc<InvestigationReportStore>>,
    cancel_token: CancellationToken,
    tx: mpsc::Sender<InvestigationEvent>,
) -> AppResult<InvestigationResult> {
    let (request_id, query, query_type) = investigation_log_fields(&investigation);
    let prompt_hash = format!(
        "{}:{INVESTIGATION_CACHE_VERSION}",
        registry.prompt_hash_all()?
    );
    let subject_history = load_subject_history(knowledge_base.as_ref(), &investigation, &tx).await;
    if let Some(cache) = cache.as_ref() {
        if let Some(cached) = cache
            .get_full_investigation(investigation.query_type, &investigation.query, &prompt_hash)
            .await
            .unwrap_or(None)
        {
            tracing::info!(
                request_id = %request_id,
                query = %query,
                query_type = query_type,
                sources = cached.sources_analyzed,
                "investigation cache hit"
            );
            if let Some(store) = report_store.as_ref() {
                store
                    .store_replacement_report(&investigation.request_id, &cached.detective_markdown)
                    .await?;
            }
            replay_cached_result(
                &cached,
                &tx,
                &investigation.request_id,
                report_store.is_none(),
            )
            .await;
            return Ok(cached);
        }
    }

    let started_at = Instant::now();
    emit_phase(&tx, 1, "Thu thập dữ liệu", None).await;
    tx.send(InvestigationEvent::Progress {
        phase: 1,
        message: format!(
            "Đang kết nối tới các nguồn để rà soát {}...",
            investigation.query
        ),
    })
    .await
    .ok();
    let (scrape_progress_tx, mut scrape_progress_rx) = mpsc::unbounded_channel::<ScraperProgress>();
    let scrape_event_tx = tx.clone();
    let request_id_for_scrape = investigation.request_id.clone();
    let query_for_scrape = investigation.query.clone();
    let query_type_for_scrape = investigation.query_type;
    let scrape_progress_forwarder = tokio::spawn(async move {
        while let Some(progress) = scrape_progress_rx.recv().await {
            match progress {
                ScraperProgress::Starting(source) => {
                    let message = if source.is_search_engine() {
                        "Tiến hành rà soát trên internet...".to_string()
                    } else {
                        format!("Đang tìm kiếm ở {}...", source.display_name())
                    };
                    scrape_event_tx
                        .send(InvestigationEvent::SourceStatus {
                            source: format!("{:?}", source),
                            status: "searching".to_string(),
                            found: 0,
                            message,
                        })
                        .await
                        .ok();
                }
                ScraperProgress::Completed(result) => {
                    let found_count = effective_source_found_count(&result);
                    tracing::info!(
                        request_id = %request_id_for_scrape,
                        query = %query_for_scrape,
                        query_type = query_type_for_scrape.as_str(),
                        source = ?result.source,
                        success = result.success,
                        found = found_count,
                        "scraper source finished"
                    );
                    let message = if !result.success {
                        format!(
                            "Không lấy được dữ liệu từ {}.",
                            result.source.display_name()
                        )
                    } else if found_count > 0 {
                        format!("Tìm thấy {} kết quả liên quan.", found_count)
                    } else {
                        "Đã kiểm tra nhưng chưa thấy kết quả cụ thể.".to_string()
                    };
                    scrape_event_tx
                        .send(InvestigationEvent::SourceStatus {
                            source: format!("{:?}", result.source),
                            status: if result.success { "done" } else { "error" }.to_string(),
                            found: found_count,
                            message,
                        })
                        .await
                        .ok();
                }
            }
        }
    });
    let scraped_results = tokio::select! {
        _ = cancel_token.cancelled() => return Err(cancelled_error()),
        results = run_all_scrapers(
            &investigation.query,
            investigation.query_type,
            proxy_pool.as_deref(),
            Some(scrape_progress_tx),
        ) => results,
    };
    let _ = scrape_progress_forwarder.await;
    let successful_sources = scraped_results.iter().filter(|item| item.success).count();
    tracing::info!(
        request_id = %request_id,
        query = %query,
        query_type = query_type,
        total_sources = scraped_results.len(),
        successful_sources,
        "scraping phase finished"
    );

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
    tracing::info!(
        request_id = %request_id,
        query = %query,
        query_type = query_type,
        summaries = summaries.len(),
        "source summarization finished"
    );

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 3, "Chọn URL điều tra sâu", None).await;
    let candidate_search_results =
        select_candidate_search_results(&investigation, &scraped_results, SEARCH_CANDIDATE_LIMIT);
    let selected_urls = select_investigation_urls(
        &investigation,
        registry.clone(),
        llm.clone(),
        &summaries,
        &candidate_search_results,
        cache.as_ref(),
        &cancel_token,
    )
    .await;
    tx.send(InvestigationEvent::UrlAssessment {
        selected: selected_urls.len(),
        total: candidate_search_results.len(),
        urls: selected_urls.clone(),
    })
    .await
    .ok();
    tracing::info!(
        request_id = %request_id,
        query = %query,
        query_type = query_type,
        candidate_urls = candidate_search_results.len(),
        selected_urls = selected_urls.len(),
        "deep investigation targets selected"
    );

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 4, "Phân tích URL", Some(selected_urls.len())).await;
    let extractions = extract_urls(
        &investigation,
        registry.clone(),
        llm.clone(),
        &selected_urls,
        cache.as_ref(),
        &tx,
        &cancel_token,
    )
    .await?;
    tracing::info!(
        request_id = %request_id,
        query = %query,
        query_type = query_type,
        extracted_urls = extractions.len(),
        "deep extraction phase finished"
    );

    cancel_token_check(&cancel_token)?;
    emit_phase(&tx, 5, "Tổng hợp điều tra", None).await;
    let detective_output = match detective_report(
        &investigation,
        registry,
        llm,
        &summaries,
        &extractions,
        subject_history.as_ref(),
        cache.as_ref(),
        report_store.as_deref(),
        &cancel_token,
    )
    .await
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                request_id = %request_id,
                query = %query,
                query_type = query_type,
                error = %error,
                "detective report failed; using fallback"
            );
            tx.send(InvestigationEvent::Error {
                phase: Some(5),
                message: user_facing_phase_error(Some(5), &error.to_string()),
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
            if let Some(store) = report_store.as_ref() {
                store
                    .store_replacement_report(&investigation.request_id, &fallback)
                    .await?;
            }
            DetectiveReportOutput {
                markdown: fallback,
                wrote_stream_chunks: report_store.is_some(),
            }
        }
    };
    let detective_markdown = detective_output.markdown;
    if let Some(store) = report_store.as_ref() {
        if !detective_output.wrote_stream_chunks {
            store
                .store_replacement_report(&investigation.request_id, &detective_markdown)
                .await?;
        }
    }
    if report_store.is_none() {
        emit_final_detective_report(&tx, &detective_markdown).await;
    }
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
    schedule_ingest(knowledge_base.as_ref(), &result);

    tx.send(InvestigationEvent::Complete {
        investigation_id: investigation.request_id.clone(),
        risk_level,
        confidence,
        sources_analyzed: result.sources_analyzed,
        duration_ms: result.duration_ms,
    })
    .await
    .ok();
    tracing::info!(
        request_id = %request_id,
        query = %query,
        query_type = query_type,
        risk_level = %result.risk_level,
        confidence = result.confidence,
        sources = result.sources_analyzed,
        duration_ms = result.duration_ms,
        "investigation pipeline completed"
    );

    let sources_success_ratio = if result.scraped_results.is_empty() {
        0.0
    } else {
        result.scraped_results.iter().filter(|r| r.success).count() as f64
            / result.scraped_results.len() as f64
    };
    if let Some(cache) = cache.as_ref() {
        if sources_success_ratio >= 0.5 {
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
        } else {
            tracing::info!(
                request_id = %request_id,
                query = %query,
                sources_success_ratio = %sources_success_ratio,
                "skipping investigation cache: low source success ratio"
            );
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
    let (request_id, query, query_type) = investigation_log_fields(investigation);
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
            Ok(summary) => {
                let summary = sanitize_summary(&source, &scraped_result, summary);
                tx.send(InvestigationEvent::SummaryResult {
                    source,
                    result: summary.clone(),
                })
                .await
                .ok();
                tracing::info!(
                    request_id = %request_id,
                    query = %query,
                    query_type = query_type,
                    source = %summary.source,
                    risk_signals = summary.risk_signals.len(),
                    evidence_urls = summary.evidence_urls.len(),
                    "source summary generated"
                );
                summaries.push(summary);
            }
            Err(error) => {
                tracing::warn!(
                    request_id = %request_id,
                    query = %query,
                    query_type = query_type,
                    source = %source,
                    error = %error,
                    "source summarization failed; using fallback summary"
                );
                let fallback = build_summary_fallback(&source, &scraped_result, &error.to_string());
                tx.send(InvestigationEvent::Error {
                    phase: Some(2),
                    message: user_facing_phase_error(Some(2), &error.to_string()),
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

fn build_url_assessment_input(
    investigation: &Investigation,
    summaries: &[AgentSummary],
    search_results: &[SearchResult],
) -> String {
    json!({
        "query": investigation.query,
        "query_type": investigation.query_type.as_str(),
        "selection_target": SELECTED_URL_LIMIT,
        "candidate_pool_size": search_results.len(),
        "dedicated_scraper_domains": dedicated_scraper_domains(),
        "summaries": summaries
            .iter()
            .take(URL_ASSESSOR_SUMMARY_LIMIT)
            .map(|summary| json!({
                "source": trim_text(&summary.source, 40),
                "summary": trim_text(&summary.summary, 220),
                "key_facts": summary.key_facts.iter().take(3).map(|item| trim_text(item, 120)).collect::<Vec<_>>(),
                "risk_signals": summary.risk_signals.iter().take(3).map(|item| trim_text(item, 120)).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "search_results": search_results
            .iter()
            .enumerate()
            .map(|(index, item)| json!({
                "rank": index + 1,
                "title": trim_text(&item.title, 140),
                "url": trim_text(&item.url, 220),
                "snippet": trim_text(&item.snippet, 260),
                "domain": extract_host(&item.url),
                "flags": {
                    "mentions_query": is_query_specific_result(investigation, item),
                    "article_like": is_article_like_target(&item.url),
                    "direct_scam_signal": has_direct_scam_signal(investigation, item),
                    "generic_or_low_signal": is_generic_or_low_signal_result(item),
                    "dedicated_scraper_domain": is_dedicated_scraper_domain(&item.url),
                }
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn build_summary_fallback(
    source: &str,
    result: &ScrapedResult,
    _error_message: &str,
) -> AgentSummary {
    let found_count = effective_source_found_count(result);
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
        if found_count > 0 {
            risk_signals.push(format!(
                "Nguồn {source} có dữ liệu liên quan và nên được kiểm tra thêm."
            ));
        }
    }

    let raw_summary = summarize_raw_source(result);
    AgentSummary {
        source: source.to_string(),
        summary: if raw_summary.is_empty() {
            format!("Nguồn {source} có phản hồi nhưng chưa đủ dữ liệu rõ ràng để kết luận thêm.")
        } else {
            trim_text(
                &format!(
                    "Nguồn {source} có dữ liệu liên quan, nhưng nội dung chưa đủ rõ để tóm tắt trọn vẹn. Manh mối chính ghi nhận được: {raw_summary}"
                ),
                320,
            )
        },
        key_facts,
        phone_mentions: Vec::new(),
        risk_signals,
        evidence_urls: collect_evidence_urls(result),
    }
}

fn sanitize_summary(
    source: &str,
    result: &ScrapedResult,
    mut summary: AgentSummary,
) -> AgentSummary {
    let mut baseline = build_summary_baseline(source, result);
    baseline.summary = sanitize_summary_text(&baseline.summary);
    baseline.key_facts = sanitize_summary_list(baseline.key_facts, 140);
    baseline.phone_mentions = sanitize_summary_list(baseline.phone_mentions, 40);
    baseline.risk_signals = sanitize_summary_list(baseline.risk_signals, 180);

    summary.source = source.to_string();
    summary.summary = sanitize_summary_text(&summary.summary);
    summary.key_facts = sanitize_summary_list(summary.key_facts, 140);
    summary.phone_mentions = sanitize_summary_list(summary.phone_mentions, 40);
    summary.risk_signals = sanitize_summary_list(summary.risk_signals, 180);

    if summary.summary.is_empty() {
        summary.summary = baseline.summary;
    }
    if summary.key_facts.is_empty() {
        summary.key_facts = baseline.key_facts;
    }
    if summary.phone_mentions.is_empty() {
        summary.phone_mentions = baseline.phone_mentions;
    }
    if summary.risk_signals.is_empty() {
        summary.risk_signals = baseline.risk_signals;
    }

    summary.evidence_urls = collect_evidence_urls(result);
    summary
}

fn build_summary_baseline(source: &str, result: &ScrapedResult) -> AgentSummary {
    let found_count = effective_source_found_count(result);
    let key_facts = result
        .reports
        .iter()
        .take(3)
        .map(|report| trim_text(&report.title, 140))
        .chain(
            result
                .search_results
                .iter()
                .take(3)
                .map(|item| trim_text(&item.title, 140)),
        )
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    let risk_signals = result
        .search_results
        .iter()
        .filter_map(|item| {
            let snippet = trim_text(&item.snippet, 180);
            (!snippet.is_empty()).then_some(snippet)
        })
        .take(3)
        .collect::<Vec<_>>();

    let phone_mentions = query_phone_mention(result);
    let summary = if let Some(report) = result.reports.first() {
        trim_text(
            &format!(
                "Nguồn {source} ghi nhận {found_count} mục liên quan. Nội dung nổi bật từ bài report đầu tiên: {}",
                report.content
            ),
            320,
        )
    } else if let Some(item) = result.search_results.first() {
        trim_text(
            &format!(
                "Nguồn {source} ghi nhận {found_count} kết quả liên quan. Manh mối nổi bật: {} {}",
                item.title, item.snippet
            ),
            320,
        )
    } else {
        format!("Nguồn {source} không trả về dữ liệu chi tiết để tóm tắt thêm.")
    };

    AgentSummary {
        source: source.to_string(),
        summary,
        key_facts,
        phone_mentions,
        risk_signals: if risk_signals.is_empty() && found_count > 0 {
            vec![format!(
                "Nguồn {source} có {found_count} mục liên quan cần điều tra sâu hơn."
            )]
        } else {
            risk_signals
        },
        evidence_urls: collect_evidence_urls(result),
    }
}

fn sanitize_summary_text(value: &str) -> String {
    let text = trim_text(value, 320);
    if is_schema_leak(&text) || contains_user_unfriendly_technical_detail(&text) {
        String::new()
    } else {
        text
    }
}

fn sanitize_summary_list(items: Vec<String>, max_len: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut sanitized = Vec::new();

    for item in items {
        let text = trim_text(&item, max_len);
        if text.is_empty()
            || is_schema_leak(&text)
            || contains_user_unfriendly_technical_detail(&text)
            || !seen.insert(text.clone())
        {
            continue;
        }
        sanitized.push(text);
    }

    sanitized
}

fn is_schema_leak(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized.contains("summary:")
        || normalized.contains("key_facts")
        || normalized.contains("phone_mentions")
        || normalized.contains("risk_signals")
        || normalized.contains("evidence_urls")
}

fn contains_user_unfriendly_technical_detail(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    [
        "page expired",
        "unauthorized",
        "invalid api key",
        "authentication_error",
        "llm upstream error",
        "server error",
        "moved permanently",
        "http status",
        "status code",
        "redirect",
        "parser",
        "parse json",
        "raw html",
        "timeout",
        "timed out",
        "proxy",
        "transport error",
        "cache hit",
        "sidecar",
        "scraper",
        "json error",
        "http client",
        "cloudflare",
        "nginx",
        "javascript",
        "cookies",
        "dns-prefetch",
        "<link",
        "<script",
        "<meta",
        "<div",
        "<html",
        "nội dung scrape",
        "challenge",
        "captcha",
        "403 forbidden",
        "404 not found",
        "500 internal",
        "502 bad gateway",
        "503 service",
        "connection refused",
        "connection reset",
        "ssl",
        "tls",
        "certificate",
        "enablejs",
        "recaptcha",
        "verify you are human",
        "access denied",
        "bot detection",
        "rate limit",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

fn user_facing_phase_error(phase: Option<u8>, error_message: &str) -> String {
    if matches!(phase, Some(5)) {
        if is_ai_service_issue(error_message) {
            return "Hệ thống AI đang gặp sự cố nên tôi sẽ tự tổng hợp báo cáo tạm thời từ các dấu vết đã thu thập.".to_string();
        }
        return "Phần nhận định cuối cùng đang gặp trục trặc, tôi sẽ tự tổng hợp báo cáo tạm thời từ dữ liệu hiện có.".to_string();
    }

    if matches!(phase, Some(4)) {
        return "Một vài trang chưa phân tích được trọn vẹn, nhưng hệ thống vẫn giữ lại các dấu vết đã đọc được.".to_string();
    }

    if matches!(phase, Some(2)) {
        return "Một vài nguồn chưa tóm tắt được đầy đủ, nhưng hệ thống vẫn tiếp tục ghép các dữ kiện đã có.".to_string();
    }

    "Quá trình điều tra đang gặp trục trặc tạm thời. Hệ thống sẽ cố gắng giữ lại các dữ kiện đã thu thập.".to_string()
}

fn is_ai_service_issue(error_message: &str) -> bool {
    let normalized = error_message.to_ascii_lowercase();
    [
        "invalid api key",
        "authentication_error",
        "unauthorized",
        "llm upstream error",
        "chat/completions",
        "model",
        "api key",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

fn query_phone_mention(result: &ScrapedResult) -> Vec<String> {
    let query = result.query.trim();
    let digits = query.chars().filter(|ch| ch.is_ascii_digit()).count();
    if digits >= 8 && digits == query.chars().count() {
        vec![query.to_string()]
    } else {
        Vec::new()
    }
}

fn sanitize_extraction(mut extraction: AgentExtraction) -> AgentExtraction {
    extraction.summary = sanitize_summary_text(&extraction.summary);
    extraction.entities = sanitize_summary_list(extraction.entities, 120);
    extraction.risk_signals = sanitize_summary_list(extraction.risk_signals, 180);
    extraction.related_numbers = sanitize_summary_list(extraction.related_numbers, 40);

    if extraction.summary.is_empty() {
        extraction.summary = "Không có thông tin liên quan".to_string();
    }

    extraction
}

fn collect_evidence_urls(result: &ScrapedResult) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for report in &result.reports {
        let url = report.url.trim();
        if !url.is_empty() && seen.insert(url.to_owned()) {
            urls.push(url.to_owned());
        }
    }

    for item in &result.search_results {
        let url = item.url.trim();
        if !url.is_empty() && seen.insert(url.to_owned()) {
            urls.push(url.to_owned());
        }
    }

    urls
}

fn summarize_raw_source(result: &ScrapedResult) -> String {
    if let Some(report) = result.reports.first() {
        let content = trim_text(&report.content, 220);
        if !contains_user_unfriendly_technical_detail(&content) {
            return content;
        }
    }
    if let Some(item) = result.search_results.first() {
        let text = trim_text(&format!("{} {}", item.title, item.snippet), 220);
        if !contains_user_unfriendly_technical_detail(&text) {
            return text;
        }
    }
    String::new()
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
    let payload = build_url_assessment_input(investigation, summaries, search_results);
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
    Ok(response.urls.into_iter().take(SELECTED_URL_LIMIT).collect())
}

async fn select_investigation_urls(
    investigation: &Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    summaries: &[AgentSummary],
    search_results: &[SearchResult],
    cache: Option<&Arc<CacheService>>,
    cancel_token: &CancellationToken,
) -> Vec<SelectedUrl> {
    assess_urls(
        investigation,
        registry,
        llm,
        summaries,
        search_results,
        cache,
        cancel_token,
    )
    .await
    .map(|urls| sanitize_selected_urls(urls, search_results, SELECTED_URL_LIMIT))
    .ok()
    .filter(|urls| !urls.is_empty())
    .unwrap_or_else(|| fallback_urls(search_results))
}

fn select_candidate_search_results(
    investigation: &Investigation,
    scraped_results: &[ScrapedResult],
    limit: usize,
) -> Vec<SearchResult> {
    let google_results =
        collect_engine_search_results(investigation, scraped_results, SourceName::Google, limit);
    if google_results.len() >= limit {
        return google_results;
    }

    let ddg_results = collect_engine_search_results(
        investigation,
        scraped_results,
        SourceName::DuckDuckGo,
        limit,
    );
    merge_search_results(google_results, ddg_results, limit)
}

fn collect_engine_search_results(
    investigation: &Investigation,
    scraped_results: &[ScrapedResult],
    source: SourceName,
    limit: usize,
) -> Vec<SearchResult> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();

    for result in scraped_results {
        if result.source != source {
            continue;
        }

        for item in &result.search_results {
            let dedupe_key = canonicalize_investigation_url(&item.url);
            if is_low_value_investigation_target(&item.url)
                || is_search_engine_results_page(&item.url)
                || !is_query_specific_result(investigation, item)
                || !seen.insert(dedupe_key)
            {
                continue;
            }
            selected.push(item.clone());
            if selected.len() >= limit {
                return selected;
            }
        }
    }

    selected
}

fn merge_search_results(
    primary: Vec<SearchResult>,
    secondary: Vec<SearchResult>,
    limit: usize,
) -> Vec<SearchResult> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for item in primary.into_iter().chain(secondary) {
        let dedupe_key = canonicalize_investigation_url(&item.url);
        if !seen.insert(dedupe_key) {
            continue;
        }
        merged.push(item);
        if merged.len() >= limit {
            break;
        }
    }

    merged
}

fn is_query_specific_result(investigation: &Investigation, item: &SearchResult) -> bool {
    let haystack = format!("{} {} {}", item.title, item.snippet, item.url).to_ascii_lowercase();
    let query = investigation.query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return false;
    }

    match investigation.query_type {
        QueryType::Phone | QueryType::Bank => haystack.contains(&query),
        QueryType::Url => {
            haystack.contains(&query)
                || Url::parse(&item.url)
                    .ok()
                    .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
                    .is_some_and(|host| query.contains(&host) || host.contains(&query))
        }
    }
}

fn has_direct_scam_signal(investigation: &Investigation, item: &SearchResult) -> bool {
    let haystack = format!("{} {} {}", item.title, item.snippet, item.url).to_ascii_lowercase();
    let query = investigation.query.trim().to_ascii_lowercase();
    let risk_keyword_hit = [
        "lừa đảo",
        "lua dao",
        "scam",
        "tố cáo",
        "to cao",
        "cảnh báo",
        "canh bao",
        "giả mạo",
        "gia mao",
        "bùng",
        "quỵt",
        "chiếm đoạt",
        "chiem doat",
        "chuyển tiền",
        "không trả",
        "mat tien",
        "mất tiền",
    ]
    .iter()
    .any(|keyword| haystack.contains(keyword));

    if !risk_keyword_hit {
        return false;
    }

    match investigation.query_type {
        QueryType::Phone | QueryType::Bank => haystack.contains(&query),
        QueryType::Url => haystack.contains(&query),
    }
}

fn is_generic_or_low_signal_result(item: &SearchResult) -> bool {
    let haystack = format!("{} {} {}", item.title, item.snippet, item.url).to_ascii_lowercase();
    [
        "hướng dẫn",
        "huong dan",
        "cách",
        "cach",
        "kinh nghiệm",
        "kinh nghiem",
        "mẹo",
        "meo",
        "nhận biết",
        "nhan biet",
        "phòng tránh",
        "phong tranh",
        "tra cứu",
        "tra cuu",
        "là gì",
        "la gi",
        "quy trình",
        "quy trinh",
        "tổng hợp",
        "tong hop",
    ]
    .iter()
    .any(|keyword| haystack.contains(keyword))
}

fn is_article_like_target(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let path = parsed.path().to_ascii_lowercase();
    if path == "/" || path.is_empty() {
        return false;
    }

    [
        "/posts/",
        "/post/",
        "/threads/",
        "/thread/",
        "/groups/",
        "/warning/",
        "/report/",
        "/article/",
        "/articles/",
        "/news/",
    ]
    .iter()
    .any(|needle| path.contains(needle))
        || path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .nth(1)
            .is_some()
}

fn dedicated_scraper_domains() -> Vec<&'static str> {
    vec![
        "checkscam.vn",
        "tinnhiemmang.vn",
        "chongluadao.vn",
        "feeds.chongluadao.vn",
        "trangtrang.com",
    ]
}

fn is_dedicated_scraper_domain(url: &str) -> bool {
    let Some(host) = extract_host(url) else {
        return false;
    };
    dedicated_scraper_domains()
        .into_iter()
        .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
}

fn extract_host(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

fn sanitize_selected_urls(
    urls: Vec<SelectedUrl>,
    search_results: &[SearchResult],
    limit: usize,
) -> Vec<SelectedUrl> {
    let allowed_urls = search_results
        .iter()
        .map(|item| canonicalize_investigation_url(&item.url))
        .collect::<HashSet<_>>();
    let result_by_url = search_results
        .iter()
        .map(|item| (canonicalize_investigation_url(&item.url), item))
        .collect::<HashMap<_, _>>();
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_signatures = HashSet::new();

    for url in urls {
        let dedupe_key = canonicalize_investigation_url(&url.url);
        let Some(source_item) = result_by_url.get(&dedupe_key) else {
            continue;
        };
        if is_low_value_investigation_target(&url.url)
            || is_search_engine_results_page(&url.url)
            || !allowed_urls.contains(&dedupe_key)
            || !seen.insert(dedupe_key)
            || is_generic_or_low_signal_result(source_item)
            || !seen_signatures.insert(canonicalize_search_result_signature(source_item))
        {
            continue;
        }
        selected.push(SelectedUrl {
            priority: selected.len() as u8 + 1,
            ..url
        });
        if selected.len() >= limit {
            break;
        }
    }

    selected
}

async fn extract_urls(
    investigation: &Investigation,
    registry: Arc<AgentRegistry>,
    llm: Arc<LlmClient>,
    urls: &[SelectedUrl],
    cache: Option<&Arc<CacheService>>,
    tx: &mpsc::Sender<InvestigationEvent>,
    cancel_token: &CancellationToken,
) -> AppResult<Vec<AgentExtraction>> {
    let (request_id, query, query_type) = investigation_log_fields(investigation);
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
            extraction = sanitize_extraction(extraction);
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
                tracing::info!(
                    request_id = %request_id,
                    query = %query,
                    query_type = query_type,
                    url = %extraction.url,
                    risk_signals = extraction.risk_signals.len(),
                    related_numbers = extraction.related_numbers.len(),
                    "url extraction generated"
                );
                extractions.push(extraction);
            }
            Err(error) => {
                tracing::warn!(
                    request_id = %request_id,
                    query = %query,
                    query_type = query_type,
                    error = %error,
                    "url extraction failed"
                );
                tx.send(InvestigationEvent::Error {
                    phase: Some(4),
                    message: user_facing_phase_error(Some(4), &error.to_string()),
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
    subject_history: Option<&SubjectHistory>,
    cache: Option<&Arc<CacheService>>,
    report_store: Option<&InvestigationReportStore>,
    cancel_token: &CancellationToken,
) -> AppResult<DetectiveReportOutput> {
    let agent = registry.get("detective")?;
    let payload = json!({
        "query": investigation.query,
        "query_type": investigation.query_type.as_str(),
        "available_sources": summaries
            .iter()
            .map(|item| item.source.as_str())
            .collect::<Vec<_>>(),
        "historical_context": detective_history_payload(subject_history),
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
            tracing::info!(
                request_id = %investigation.request_id,
                query = %investigation.query,
                query_type = investigation.query_type.as_str(),
                "detective report cache hit"
            );
            return Ok(DetectiveReportOutput {
                markdown: cached,
                wrote_stream_chunks: false,
            });
        }
    }
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<StreamChunk>(64);
    let stream_agent = agent.clone();
    let stream_payload = payload.clone();
    let llm_task = tokio::spawn(async move {
        llm.stream_text(&stream_agent, &stream_payload, chunk_tx)
            .await
    });

    let mut repetition_aborted = false;
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
                if !chunk.done {
                    if let Some(store) = report_store {
                        store
                            .append_report_chunk(&investigation.request_id, &chunk.content)
                            .await?;
                    }
                }
                if chunk.done {
                    if let Some(store) = report_store {
                        store.finalize_report_stream(&investigation.request_id).await?;
                    }
                    break;
                }
            }
        }
    }

    let markdown = llm_task
        .await
        .map_err(|error| AppError::Server(error.to_string()))??;

    if detect_repetition_loop(&markdown) {
        repetition_aborted = true;
        tracing::warn!(
            request_id = %investigation.request_id,
            "detective report detected as repetition loop; using fallback"
        );
    }

    if repetition_aborted {
        return Err(AppError::Server(
            "detective report was repetitive and aborted".to_string(),
        ));
    }
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
    Ok(DetectiveReportOutput {
        markdown,
        wrote_stream_chunks: report_store.is_some(),
    })
}

async fn emit_final_detective_report(tx: &mpsc::Sender<InvestigationEvent>, markdown: &str) {
    tx.send(InvestigationEvent::DetectiveStream {
        chunk: markdown.to_string(),
        done: true,
        replace: true,
    })
    .await
    .ok();
}

fn fallback_urls(results: &[SearchResult]) -> Vec<SelectedUrl> {
    let mut seen = HashSet::new();
    let mut seen_signatures = HashSet::new();
    results
        .iter()
        .filter(|item| {
            let dedupe_key = canonicalize_investigation_url(&item.url);
            !is_low_value_investigation_target(&item.url)
                && !is_search_engine_results_page(&item.url)
                && !is_generic_or_low_signal_result(item)
                && seen.insert(dedupe_key)
                && seen_signatures.insert(canonicalize_search_result_signature(item))
        })
        .take(SELECTED_URL_LIMIT)
        .enumerate()
        .map(|(index, item)| SelectedUrl {
            url: item.url.clone(),
            reason: "fallback".to_string(),
            priority: (index + 1) as u8,
        })
        .collect()
}

fn is_search_engine_results_page(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };

    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = parsed.path();

    if host.contains("duckduckgo.com") {
        return path == "/" || path == "/html/" || path == "/html";
    }

    if host == "google.com" || host.starts_with("google.") || host.starts_with("www.google.") {
        return path == "/search" || path == "/url";
    }

    false
}

fn is_low_value_investigation_target(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let path = parsed.path().to_ascii_lowercase();
    [
        ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn canonicalize_investigation_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };

    parsed.set_fragment(None);
    let filtered_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| !is_tracking_query_param(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(filtered_pairs);

    if parsed.query().is_none_or(str::is_empty) {
        parsed.set_query(None);
    }

    parsed.to_string()
}

fn canonicalize_search_result_signature(item: &SearchResult) -> String {
    normalize_signature_text(&format!("{} {}", item.title, item.snippet))
}

fn normalize_signature_text(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(24)
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_tracking_query_param(key: &str) -> bool {
    key.starts_with("utm_")
        || matches!(
            key,
            "fbclid"
                | "gclid"
                | "dclid"
                | "gbraid"
                | "wbraid"
                | "ved"
                | "ei"
                | "sei"
                | "sa"
                | "usg"
                | "aqs"
                | "oq"
                | "source"
                | "sxsrf"
                | "sca_esv"
                | "rlz"
                | "iflsig"
                | "sclient"
                | "qh_ss"
                | "srsltid"
        )
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

fn detect_repetition_loop(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| line.len() >= 20)
        .collect();
    if lines.len() < 6 {
        return false;
    }
    let mut seen = HashMap::new();
    let mut max_repeats = 0usize;
    for line in &lines {
        let normalized = normalize_signature_text(line);
        let count = seen.entry(normalized).or_insert(0usize);
        *count += 1;
        max_repeats = max_repeats.max(*count);
    }
    max_repeats >= 4
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

async fn replay_cached_result(
    cached: &InvestigationResult,
    tx: &mpsc::Sender<InvestigationEvent>,
    investigation_id: &str,
    include_report_event: bool,
) {
    emit_phase(tx, 1, "Cache hit", Some(cached.scraped_results.len())).await;
    for result in &cached.scraped_results {
        let found_count = effective_source_found_count(result);
        let message = if !result.success {
            format!(
                "Không lấy được dữ liệu từ {}.",
                result.source.display_name()
            )
        } else if found_count > 0 {
            format!("Tìm thấy {} kết quả liên quan.", found_count)
        } else {
            "Đã kiểm tra nhưng chưa thấy kết quả cụ thể.".to_string()
        };
        tx.send(InvestigationEvent::SourceStatus {
            source: format!("{:?}", result.source),
            status: if result.success { "done" } else { "error" }.to_string(),
            found: found_count,
            message,
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

    let candidate_search_results = select_candidate_search_results(
        &Investigation {
            request_id: "cached-replay".to_string(),
            query: cached.query.clone(),
            query_type: cached.query_type,
        },
        &cached.scraped_results,
        SEARCH_CANDIDATE_LIMIT,
    );
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
        total: candidate_search_results.len(),
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
    if include_report_event {
        emit_final_detective_report(tx, &cached.detective_markdown).await;
    }
    tx.send(InvestigationEvent::Complete {
        investigation_id: investigation_id.to_string(),
        risk_level: cached.risk_level.clone(),
        confidence: cached.confidence,
        sources_analyzed: cached.sources_analyzed,
        duration_ms: cached.duration_ms,
    })
    .await
    .ok();
}

fn effective_source_found_count(result: &ScrapedResult) -> usize {
    let scraped_count = result.reports.len() + result.search_results.len();
    let metadata_count = result
        .metadata
        .get("report_count")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);

    metadata_count.unwrap_or(scraped_count).max(scraped_count)
}

fn build_detective_fallback(
    investigation: &Investigation,
    summaries: &[AgentSummary],
    extractions: &[AgentExtraction],
    scraped_results: &[ScrapedResult],
    _error_message: &str,
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
    let total_reports: usize = scraped_results
        .iter()
        .map(|item| effective_source_found_count(item))
        .sum();
    let has_dedicated_scam_reports = scraped_results.iter().any(|item| {
        matches!(
            item.source,
            SourceName::AdminVN | SourceName::CheckScam | SourceName::ChongLuaDao
        ) && effective_source_found_count(item) > 0
    });
    let risk_level = if total_reports >= 8 || (has_dedicated_scam_reports && total_reports >= 3) {
        "high"
    } else if risk_signal_count >= 4 || total_reports >= 5 {
        "high"
    } else if risk_signal_count >= 2 || total_reports >= 2 || has_dedicated_scam_reports {
        "medium"
    } else if successful_sources == 0 {
        "unknown"
    } else {
        "low"
    };
    let confidence = if successful_sources == 0 {
        0.1
    } else if total_reports >= 8 {
        0.85
    } else if total_reports >= 5 || (has_dedicated_scam_reports && total_reports >= 3) {
        0.75
    } else if risk_signal_count >= 4 || total_reports >= 3 {
        0.6
    } else if risk_signal_count >= 2 || total_reports >= 2 {
        0.5
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
        "## Báo cáo tạm thời\n\n\
Hệ thống AI đang gặp sự cố nên tôi đang tự tổng hợp báo cáo tạm thời từ những dấu vết đã thu thập được.\n\n\
### Thông tin đã ghi nhận\n\n\
- {}: `{}`\n\
- Đã nhận phản hồi từ: {}/{} nguồn\n\n\
### Dấu hiệu nổi bật\n\n\
- Tín hiệu rủi ro từ các nguồn đã rà soát: {}\n\
- Tín hiệu rủi ro từ các trang được xem kỹ hơn: {}\n\
- Chi tiết nổi bật đã ghi nhận: {}\n\
- Thực thể liên quan đã bóc tách: {}\n\n\
### Kết luận tạm thời\n\n\
Mức rủi ro hiện được đánh giá là {}. Nếu bạn đang cân nhắc giao dịch hoặc chia sẻ thông tin nhạy cảm, nên dừng lại và kiểm tra thêm trước khi tiếp tục.\n\n\
RISK_LEVEL: {}\n\
CONFIDENCE: {:.2}",
        query_subject_label(investigation.query_type),
        investigation.query,
        successful_sources,
        scraped_results.len(),
        join_or_none(&summary_signals),
        join_or_none(&extraction_signals),
        join_or_none(&key_facts),
        join_or_none(&related_entities),
        fallback_risk_level_label(risk_level),
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

fn fallback_risk_level_label(value: &str) -> &'static str {
    match value {
        "critical" => "rất cao",
        "high" => "cao",
        "medium" => "trung bình",
        "low" => "thấp",
        _ => "chưa rõ",
    }
}

fn query_subject_label(query_type: QueryType) -> &'static str {
    match query_type {
        QueryType::Phone => "Số điện thoại đang tra cứu",
        QueryType::Bank => "Tài khoản đang tra cứu",
        QueryType::Url => "Địa chỉ đang tra cứu",
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
    use tokio::sync::mpsc;

    #[test]
    fn fallback_detective_report_sets_high_risk_when_many_signals_exist() {
        let investigation = Investigation {
            request_id: "test-request".to_string(),
            query: "0926408013".to_string(),
            query_type: QueryType::Phone,
        };
        let summaries = vec![AgentSummary {
            source: "CheckScam".to_string(),
            summary: "summary".to_string(),
            key_facts: vec!["co canh bao".to_string()],
            phone_mentions: vec![],
            risk_signals: vec!["signal-1".to_string(), "signal-2".to_string()],
            evidence_urls: vec![],
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
        assert!(markdown.contains("Mức rủi ro hiện được đánh giá là cao"));
        assert!(!markdown.contains("upstream failed"));
    }

    #[test]
    fn fallback_urls_dedupes_tracking_variants_and_skips_pdf_targets() {
        let urls = vec![
            SearchResult {
                title: "A".to_string(),
                url: "https://example.com/post?srsltid=aaa".to_string(),
                snippet: String::new(),
            },
            SearchResult {
                title: "B".to_string(),
                url: "https://example.com/post?srsltid=bbb&utm_source=x".to_string(),
                snippet: String::new(),
            },
            SearchResult {
                title: "C".to_string(),
                url: "https://example.com/file.pdf?srsltid=ccc".to_string(),
                snippet: String::new(),
            },
        ];

        let selected = fallback_urls(&urls);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].url, "https://example.com/post?srsltid=aaa");
    }

    #[test]
    fn candidate_search_results_prefer_google_and_drop_generic_articles() {
        let investigation = Investigation {
            request_id: "test-request".to_string(),
            query: "0562015037".to_string(),
            query_type: QueryType::Phone,
        };
        let google = ScrapedResult {
            source: SourceName::Google,
            query: investigation.query.clone(),
            success: true,
            reports: vec![],
            search_results: vec![
                SearchResult {
                    title: "Nguyen Dinh Thang 0562015037".to_string(),
                    url: "https://checkscam.vn/nguyen-dinh-thang-24/".to_string(),
                    snippet: "Canh bao so 0562015037".to_string(),
                },
                SearchResult {
                    title: "Hướng dẫn kiểm tra số điện thoại lừa đảo".to_string(),
                    url: "https://vietnamnet.vn/huong-dan-kiem-tra-so-dien-thoai-lua-dao-2455380.html"
                        .to_string(),
                    snippet: "Bài hướng dẫn chung, không nhắc số cụ thể".to_string(),
                },
            ],
            metadata: Value::Null,
            raw_html: None,
            duration_ms: 1,
            error: None,
        };
        let ddg = ScrapedResult {
            source: SourceName::DuckDuckGo,
            query: investigation.query.clone(),
            success: true,
            reports: vec![],
            search_results: vec![SearchResult {
                title: "Facebook post 0562015037".to_string(),
                url: "https://www.facebook.com/groups/871890707767827/posts/1236268011330093/"
                    .to_string(),
                snippet: "Canh bao ve 0562015037".to_string(),
            }],
            metadata: Value::Null,
            raw_html: None,
            duration_ms: 1,
            error: None,
        };

        let selected =
            select_candidate_search_results(&investigation, &[google, ddg], SEARCH_CANDIDATE_LIMIT);
        let urls = selected
            .iter()
            .map(|item| item.url.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                "https://checkscam.vn/nguyen-dinh-thang-24/",
                "https://www.facebook.com/groups/871890707767827/posts/1236268011330093/",
            ]
        );
    }

    #[test]
    fn sanitize_selected_urls_drops_near_duplicate_story_variants() {
        let search_results = vec![
            SearchResult {
                title: "Canh bao 0562015037".to_string(),
                url: "https://www.facebook.com/groups/a/posts/1/".to_string(),
                snippet: "Nguoi nay scam 300k khi giao dich acc".to_string(),
            },
            SearchResult {
                title: "Canh bao 0562015037".to_string(),
                url: "https://forum.example/thread-1".to_string(),
                snippet: "Nguoi nay scam 300k khi giao dich acc".to_string(),
            },
        ];

        let selected = sanitize_selected_urls(
            vec![
                SelectedUrl {
                    url: "https://www.facebook.com/groups/a/posts/1/".to_string(),
                    reason: "direct".to_string(),
                    priority: 1,
                },
                SelectedUrl {
                    url: "https://forum.example/thread-1".to_string(),
                    reason: "mirror".to_string(),
                    priority: 2,
                },
            ],
            &search_results,
            SELECTED_URL_LIMIT,
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].url,
            "https://www.facebook.com/groups/a/posts/1/"
        );
    }

    #[tokio::test]
    async fn emit_final_detective_report_sends_single_replacement_event() {
        let (tx, mut rx) = mpsc::channel(1);

        emit_final_detective_report(&tx, "# Bao cao cuoi cung").await;

        match rx.recv().await {
            Some(InvestigationEvent::DetectiveStream {
                chunk,
                done,
                replace,
            }) => {
                assert_eq!(chunk, "# Bao cao cuoi cung");
                assert!(done);
                assert!(replace);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
