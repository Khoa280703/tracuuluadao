use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{ArgAction, Parser};
use futures::StreamExt;
use regex::Regex;
use reqwest::StatusCode;
use scraper::{Html, Selector};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::fs;
use uuid::Uuid;

use tracuuluadao::cache::CacheService;
use tracuuluadao::knowledge_base::{EvidenceInput, KnowledgeBase, numeric_to_risk_level};
use tracuuluadao::logging;
use tracuuluadao::scrapers::checkscam::{SIDECAR_BASE_URL, parse_detail_report};
use tracuuluadao::scrapers::http_client::{HttpClient, HttpClientFactory, is_cloudflare_blocked};

const CRAWL_SOURCE: &str = "checkscam_crawl";
const REST_API_BASE: &str = "https://checkscam.vn/wp-json/wp/v2/posts";
const REST_PAGE_SIZE: usize = 100;

static PHONE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:sđt|sdt|điện thoại|số điện thoại)\s*:\s*([0-9 .-]{9,16})")
        .expect("valid phone regex")
});
static TITLE_PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b0\d{8,10}\b").expect("valid title phone regex"));
static TITLE_ACCOUNT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-9]{6,20}\b").expect("valid title account regex"));

#[derive(Parser, Debug, Clone)]
#[command(name = "checkscam-crawler")]
#[command(about = "Bulk-crawl checkscam.vn posts into the knowledge base")]
struct Args {
    #[arg(long, default_value_t = 0)]
    max_pages: usize,

    #[arg(long, default_value_t = 3)]
    concurrency: usize,

    #[arg(long, default_value_t = 200)]
    delay_ms: u64,

    #[arg(long, default_value_t = 0)]
    max_posts: usize,

    #[arg(long, default_value_t = false)]
    dry_run: bool,

    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    resume: bool,

    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    #[arg(long, default_value = "data/media")]
    media_dir: String,

    #[arg(long)]
    url: Option<String>,

    #[arg(long, default_value_t = false)]
    retry_failures: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WpRenderedField {
    rendered: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CrawlPostRecord {
    id: u64,
    link: String,
    title: WpRenderedField,
    content: WpRenderedField,
    date: String,
}

#[derive(Debug, Default)]
struct PaginationResult {
    pages_fetched: usize,
    posts: Vec<CrawlPostRecord>,
    total_pages: Option<usize>,
}

#[derive(Debug, Default)]
struct ExtractedEntities {
    phones: Vec<String>,
    bank_accounts: Vec<String>,
    owner: Option<String>,
    bank: Option<String>,
    warning: Option<String>,
    report_count: Option<u32>,
    image_urls: Vec<String>,
    source_url: String,
    title: String,
    date: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct CrawlStats {
    posts_discovered: usize,
    posts_processed: usize,
    posts_skipped: usize,
    post_errors: usize,
    subjects_upserted: usize,
    evidence_inserted: usize,
    evidence_updated: usize,
    media_attached: usize,
    media_reused: usize,
    media_downloaded: usize,
}

#[derive(Debug, Default)]
struct ProcessOutcome {
    stats: CrawlStats,
}

#[derive(Debug, Default)]
struct LinkStats {
    links_created: usize,
    link_errors: usize,
    subjects_risk_updated: usize,
}

#[derive(Clone)]
struct CrawlerSettings {
    dry_run: bool,
    resume: bool,
    delay: Duration,
    concurrency: usize,
    media_root: PathBuf,
    media_root_relative: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    let _log_guards = logging::init()?;

    let settings = build_settings(&args)?;
    if !settings.dry_run {
        fs::create_dir_all(&settings.media_root)
            .await
            .context("failed to create media root")?;
    }

    let pool = CacheService::connect_pool(&args.database_url)?;
    let kb = Arc::new(KnowledgeBase::new(pool));
    kb.ensure_schema().await?;

    if args.retry_failures {
        let failed_urls = kb.get_unresolved_failure_urls().await?;
        tracing::info!(count = failed_urls.len(), "retrying failed URLs");
        let mut resolved = 0usize;
        let mut still_failed = 0usize;
        for url in &failed_urls {
            match fetch_single_post_record(url).await {
                Ok(post) => {
                    let posts = vec![post];
                    let outcome = process_posts(posts, kb.clone(), settings.clone()).await?;
                    if outcome.stats.post_errors == 0 {
                        kb.mark_failure_resolved(url).await?;
                        resolved += 1;
                    } else {
                        kb.increment_crawl_failure(url, "retry", "post processing failed again")
                            .await?;
                        still_failed += 1;
                    }
                }
                Err(error) => {
                    kb.increment_crawl_failure(url, "detail_fetch", &format!("{error:#}"))
                        .await?;
                    still_failed += 1;
                }
            }
        }
        tracing::info!(resolved, still_failed, "retry pass complete");
        return Ok(());
    }

    let posts = if let Some(ref url) = args.url {
        vec![fetch_single_post_record(url).await?]
    } else {
        let rest_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .context("failed to build rest client")?;
        let mut pagination =
            collect_rest_posts(&rest_client, args.max_pages, settings.delay).await?;
        if args.max_posts > 0 && pagination.posts.len() > args.max_posts {
            pagination.posts.truncate(args.max_posts);
        }
        tracing::info!(
            pages_fetched = pagination.pages_fetched,
            total_pages = pagination.total_pages,
            posts_discovered = pagination.posts.len(),
            dry_run = settings.dry_run,
            "rest pagination complete"
        );
        pagination.posts
    };

    let process_outcome = process_posts(posts, kb.clone(), settings.clone()).await?;
    let link_stats = if settings.dry_run || process_outcome.stats.posts_processed == 0 {
        LinkStats::default()
    } else {
        link_detection_pass(kb.as_ref()).await?
    };

    tracing::info!(
        posts_processed = process_outcome.stats.posts_processed,
        posts_skipped = process_outcome.stats.posts_skipped,
        post_errors = process_outcome.stats.post_errors,
        subjects_upserted = process_outcome.stats.subjects_upserted,
        evidence_inserted = process_outcome.stats.evidence_inserted,
        evidence_updated = process_outcome.stats.evidence_updated,
        media_attached = process_outcome.stats.media_attached,
        media_downloaded = process_outcome.stats.media_downloaded,
        media_reused = process_outcome.stats.media_reused,
        links_created = link_stats.links_created,
        risk_updated = link_stats.subjects_risk_updated,
        "checkscam crawl complete"
    );

    Ok(())
}

fn build_settings(args: &Args) -> Result<CrawlerSettings> {
    let current_dir = std::env::current_dir().context("failed to resolve cwd")?;
    let media_root_relative = PathBuf::from(&args.media_dir);
    let media_root = if media_root_relative.is_absolute() {
        media_root_relative.clone()
    } else {
        current_dir.join(&media_root_relative)
    };

    Ok(CrawlerSettings {
        dry_run: args.dry_run,
        resume: args.resume,
        delay: Duration::from_millis(args.delay_ms),
        concurrency: args.concurrency.max(1),
        media_root,
        media_root_relative,
    })
}

async fn fetch_single_post_record(url: &str) -> Result<CrawlPostRecord> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let response = client
        .post(format!("{SIDECAR_BASE_URL}/checkscam/detail"))
        .json(&serde_json::json!({ "url": url }))
        .send()
        .await
        .context("sidecar request failed for --url mode")?;
    let body: serde_json::Value = response.json().await?;
    anyhow::ensure!(
        body["ok"].as_bool().unwrap_or(false),
        "sidecar returned error for {url}"
    );
    let html = body["html"].as_str().unwrap_or_default();
    let document = Html::parse_document(html);
    let title = document
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|el| el.text().collect::<String>())
        .unwrap_or_default();
    Ok(CrawlPostRecord {
        id: 0,
        link: url.to_string(),
        title: WpRenderedField { rendered: title },
        content: WpRenderedField {
            rendered: html.to_string(),
        },
        date: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    })
}

async fn collect_rest_posts(
    client: &reqwest::Client,
    max_pages: usize,
    delay: Duration,
) -> Result<PaginationResult> {
    let fallback_client = HttpClientFactory::default()
        .standard_client()
        .context("failed to build fallback http client")?;
    let mut page = 1usize;
    let mut result = PaginationResult::default();
    let mut consecutive_errors = 0usize;

    loop {
        if max_pages > 0 && page > max_pages {
            break;
        }

        let mut total_pages = None;
        let page_posts = match client
            .get(REST_API_BASE)
            .query(&[
                ("per_page", REST_PAGE_SIZE.to_string()),
                ("page", page.to_string()),
            ])
            .send()
            .await
        {
            Ok(response) if response.status() == StatusCode::BAD_REQUEST => Vec::new(),
            Ok(response) if response.status() == StatusCode::FORBIDDEN => {
                tracing::warn!(page, "rest api returned 403, falling back to rquest client");
                match fetch_rest_posts_page_via_rquest(&fallback_client, page).await {
                    Ok(posts) => posts,
                    Err(error) => {
                        consecutive_errors += 1;
                        tracing::warn!(page, error = %error, attempt = consecutive_errors, "rquest fallback failed, retrying");
                        if consecutive_errors >= 5 {
                            anyhow::bail!("too many consecutive pagination errors at page {page}");
                        }
                        tokio::time::sleep(Duration::from_secs(5 * consecutive_errors as u64))
                            .await;
                        continue;
                    }
                }
            }
            Ok(response) => {
                total_pages = response
                    .headers()
                    .get("x-wp-totalpages")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<usize>().ok());
                response
                    .error_for_status()?
                    .json::<Vec<CrawlPostRecord>>()
                    .await
                    .with_context(|| format!("failed to parse rest page {page}"))?
            }
            Err(error) => {
                tracing::warn!(page, error = %error, "reqwest rest fetch failed, trying rquest fallback");
                match fetch_rest_posts_page_via_rquest(&fallback_client, page).await {
                    Ok(posts) => posts,
                    Err(error) => {
                        consecutive_errors += 1;
                        tracing::warn!(page, error = %error, attempt = consecutive_errors, "rquest fallback also failed, retrying");
                        if consecutive_errors >= 5 {
                            anyhow::bail!("too many consecutive pagination errors at page {page}");
                        }
                        tokio::time::sleep(Duration::from_secs(5 * consecutive_errors as u64))
                            .await;
                        continue;
                    }
                }
            }
        };
        consecutive_errors = 0;
        if page_posts.is_empty() {
            break;
        }

        result.pages_fetched += 1;
        result.total_pages = total_pages.or(result.total_pages);
        result.posts.extend(page_posts);
        tracing::info!(
            page,
            total_pages = result.total_pages,
            collected = result.posts.len(),
            "fetched rest page"
        );

        page += 1;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    Ok(result)
}

async fn fetch_rest_posts_page_via_rquest(
    client: &tracuuluadao::scrapers::http_client::HttpClient,
    page: usize,
) -> Result<Vec<CrawlPostRecord>> {
    let body = client
        .get_text_with_query(
            REST_API_BASE,
            &[
                ("per_page", REST_PAGE_SIZE.to_string()),
                ("page", page.to_string()),
            ],
        )
        .await
        .with_context(|| format!("fallback rest fetch failed for page {page}"))?;
    if is_cloudflare_blocked(&body) {
        anyhow::bail!("cloudflare blocked fallback rest page {page}");
    }
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if value
            .get("code")
            .and_then(|code| code.as_str())
            .is_some_and(|code| code == "rest_post_invalid_page_number")
        {
            return Ok(Vec::new());
        }
    }
    serde_json::from_str::<Vec<CrawlPostRecord>>(&body)
        .with_context(|| format!("failed to parse fallback rest page {page}"))
}

async fn process_posts(
    posts: Vec<CrawlPostRecord>,
    kb: Arc<KnowledgeBase>,
    settings: CrawlerSettings,
) -> Result<ProcessOutcome> {
    let sidecar_client = if check_sidecar_available().await {
        Some(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .connect_timeout(Duration::from_secs(5))
                .build()?,
        )
    } else {
        None
    };
    tracing::info!(
        sidecar_available = sidecar_client.is_some(),
        concurrency = settings.concurrency,
        "starting post processing"
    );

    let image_client = HttpClientFactory.standard_client()?;
    let total = posts.len();
    let results = futures::stream::iter(posts.into_iter().map(|post| {
        let kb = kb.clone();
        let settings = settings.clone();
        let sidecar_client = sidecar_client.clone();
        let image_client = image_client.clone();
        let url = post.link.clone();
        async move {
            let result =
                process_single_post(post, kb, &settings, sidecar_client.as_ref(), &image_client)
                    .await;
            (url, result)
        }
    }))
    .buffer_unordered(settings.concurrency)
    .collect::<Vec<_>>()
    .await;

    let mut outcome = ProcessOutcome::default();
    for (index, (url, result)) in results.into_iter().enumerate() {
        match result {
            Ok(post_stats) => {
                if post_stats.posts_processed > 0 {
                    let _ = kb.mark_failure_resolved(&url).await;
                }
                merge_stats(&mut outcome.stats, &post_stats);
            }
            Err(error) => {
                tracing::warn!(url = %url, error = %error, "post processing failed");
                outcome.stats.post_errors += 1;
                let _ = kb
                    .increment_crawl_failure(&url, "post_processing", &format!("{error:#}"))
                    .await;
            }
        }
        let done = index + 1;
        if done % 50 == 0 || done == total {
            tracing::info!(
                progress = format!("{done}/{total}"),
                processed = outcome.stats.posts_processed,
                errors = outcome.stats.post_errors,
                media = outcome.stats.media_downloaded,
                "batch progress"
            );
        }
    }

    Ok(outcome)
}

async fn process_single_post(
    post: CrawlPostRecord,
    kb: Arc<KnowledgeBase>,
    settings: &CrawlerSettings,
    sidecar_client: Option<&reqwest::Client>,
    image_client: &HttpClient,
) -> Result<CrawlStats> {
    if !settings.delay.is_zero() {
        tokio::time::sleep(settings.delay).await;
    }

    let mut stats = CrawlStats {
        posts_discovered: 1,
        ..Default::default()
    };

    let detail_html = fetch_detail_html(sidecar_client, &post).await?;
    let entities = extract_entities(&post, &detail_html);
    let subjects = build_subjects(&entities);
    if subjects.is_empty() {
        stats.posts_skipped = 1;
        return Ok(stats);
    }

    if settings.resume
        && is_post_fully_ingested(kb.as_ref(), &post.link, &subjects, &entities.image_urls).await?
    {
        stats.posts_skipped = 1;
        return Ok(stats);
    }

    if settings.dry_run {
        stats.posts_processed = 1;
        stats.subjects_upserted = subjects.len();
        stats.evidence_inserted = subjects.len();
        stats.media_attached = entities.image_urls.len() * subjects.len();
        return Ok(stats);
    }

    let risk_score = entities.report_count.map(crawl_risk_score).unwrap_or(0.2);
    let risk_level = numeric_to_risk_level(risk_score);

    for (value, subject_type) in subjects {
        let subject_id = kb.upsert_subject(&value, subject_type).await?;
        stats.subjects_upserted += 1;
        let mentioned_subjects = build_mentions_for_subject(&entities, &value, subject_type);
        let evidence_input = build_evidence_input(subject_id, &post, &entities, mentioned_subjects);
        let evidence_id = if let Some(existing_id) = kb
            .get_evidence_id_for_subject_source_url(subject_id, CRAWL_SOURCE, &post.link)
            .await?
        {
            kb.update_evidence(existing_id, evidence_input).await?;
            stats.evidence_updated += 1;
            existing_id
        } else {
            stats.evidence_inserted += 1;
            kb.insert_evidence(evidence_input).await?
        };

        let media_stats = attach_images_to_evidence(
            kb.as_ref(),
            image_client,
            settings,
            evidence_id,
            &entities.image_urls,
        )
        .await?;
        stats.media_attached += media_stats.media_attached;
        stats.media_downloaded += media_stats.media_downloaded;
        stats.media_reused += media_stats.media_reused;

        kb.set_crawl_risk(subject_id, risk_score, risk_level)
            .await?;
    }

    stats.posts_processed = 1;
    Ok(stats)
}

async fn fetch_detail_html(
    sidecar_client: Option<&reqwest::Client>,
    post: &CrawlPostRecord,
) -> Result<String> {
    if let Some(sidecar) = sidecar_client {
        let response = sidecar
            .post(format!("{SIDECAR_BASE_URL}/checkscam/detail"))
            .json(&serde_json::json!({ "url": post.link }))
            .send()
            .await;
        if let Ok(response) = response {
            let body = response
                .json::<serde_json::Value>()
                .await
                .unwrap_or_default();
            if body["ok"].as_bool().unwrap_or(false) {
                if let Some(html) = body["html"].as_str() {
                    if !html.trim().is_empty() {
                        return Ok(html.to_string());
                    }
                }
            }
        }
    }

    if !post.content.rendered.trim().is_empty() {
        return Ok(post.content.rendered.clone());
    }

    anyhow::bail!("detail html unavailable for {}", post.link)
}

async fn check_sidecar_available() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{SIDECAR_BASE_URL}/health"))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn extract_entities(post: &CrawlPostRecord, html: &str) -> ExtractedEntities {
    let parsed = parse_detail_report(&post.link, &post.title.rendered, html);
    let document = Html::parse_document(html);

    let mut entities = ExtractedEntities {
        source_url: post.link.clone(),
        title: parsed.title.clone(),
        date: parsed.date.or_else(|| Some(post.date.clone())),
        ..Default::default()
    };

    for line in parsed
        .content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(value) = line.strip_prefix("Chủ tài khoản: ") {
            entities.owner = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Số điện thoại: ") {
            let normalized = normalize_subject_value(value, "phone");
            if !normalized.is_empty() {
                entities.phones.push(normalized);
            }
        } else if let Some(value) = line.strip_prefix("Số tài khoản: ") {
            let normalized = normalize_subject_value(value, "bank");
            if !normalized.is_empty() {
                entities.bank_accounts.push(normalized);
            }
        } else if let Some(value) = line.strip_prefix("Ngân hàng: ") {
            entities.bank = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Nội dung cảnh báo: ") {
            entities.warning = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Số lần bị cảnh báo trên CheckScam: ")
        {
            entities.report_count = value.trim().parse::<u32>().ok();
        }
    }

    if entities.phones.is_empty() {
        for captures in PHONE_REGEX.captures_iter(&parsed.title) {
            if let Some(value) = captures.get(1) {
                let normalized = normalize_subject_value(value.as_str(), "phone");
                if !normalized.is_empty() {
                    entities.phones.push(normalized);
                }
            }
        }
    }
    if entities.phones.is_empty() {
        for captures in PHONE_REGEX.captures_iter(&post.title.rendered) {
            if let Some(value) = captures.get(1) {
                let normalized = normalize_subject_value(value.as_str(), "phone");
                if !normalized.is_empty() {
                    entities.phones.push(normalized);
                }
            }
        }
    }
    if entities.bank_accounts.is_empty() {
        let title_lower = parsed.title.to_ascii_lowercase();
        if title_lower.contains("stk") || title_lower.contains("tài khoản") {
            for captures in TITLE_ACCOUNT_REGEX.captures_iter(&parsed.title) {
                if let Some(value) = captures.get(0) {
                    let normalized = normalize_subject_value(value.as_str(), "bank");
                    if !normalized.is_empty() {
                        entities.bank_accounts.push(normalized);
                    }
                }
            }
        }
    }

    let title_lower = parsed.title.to_ascii_lowercase();
    if title_lower.contains("sđt")
        || title_lower.contains("sdt")
        || title_lower.contains("điện thoại")
        || title_lower.contains("số điện thoại")
    {
        for captures in TITLE_PHONE_REGEX.captures_iter(&parsed.title) {
            if let Some(value) = captures.get(0) {
                let normalized = normalize_subject_value(value.as_str(), "phone");
                if !normalized.is_empty() {
                    entities.phones.push(normalized);
                }
            }
        }
    }

    entities.phones = dedup_preserve_order(entities.phones);
    entities.bank_accounts = dedup_preserve_order(entities.bank_accounts);
    entities.image_urls = extract_image_urls(&document);
    entities
}

fn build_subjects(entities: &ExtractedEntities) -> Vec<(String, &'static str)> {
    let mut subjects = Vec::new();
    subjects.extend(
        entities
            .phones
            .iter()
            .cloned()
            .map(|value| (value, "phone")),
    );
    subjects.extend(
        entities
            .bank_accounts
            .iter()
            .cloned()
            .map(|value| (value, "bank")),
    );
    subjects
}

fn build_mentions_for_subject(
    entities: &ExtractedEntities,
    current_value: &str,
    current_type: &str,
) -> Vec<String> {
    build_subjects(entities)
        .into_iter()
        .filter_map(|(value, subject_type)| {
            ((value != current_value) || (subject_type != current_type))
                .then_some(format!("{subject_type}:{value}"))
        })
        .collect()
}

fn build_evidence_input(
    subject_id: Uuid,
    post: &CrawlPostRecord,
    entities: &ExtractedEntities,
    mentioned_subjects: Vec<String>,
) -> EvidenceInput {
    let mut risk_signals = vec!["checkscam_report".to_string()];
    if entities.warning.is_some() {
        risk_signals.push("checkscam_warning_present".to_string());
    }
    if let Some(report_count) = entities.report_count {
        risk_signals.push(format!("checkscam_report_count:{report_count}"));
    }
    if !entities.image_urls.is_empty() {
        risk_signals.push("checkscam_images_present".to_string());
    }

    EvidenceInput {
        subject_id,
        investigation_id: None,
        source: CRAWL_SOURCE.to_string(),
        evidence_type: "external_report".to_string(),
        data: serde_json::json!({
            "source_url": entities.source_url,
            "title": entities.title,
            "owner": entities.owner,
            "bank": entities.bank,
            "warning": entities.warning,
            "report_count": entities.report_count,
            "date": entities.date,
            "wp_post_id": post.id,
            "image_urls": entities.image_urls,
        }),
        risk_signals,
        mentioned_subjects,
    }
}

async fn attach_images_to_evidence(
    kb: &KnowledgeBase,
    client: &HttpClient,
    settings: &CrawlerSettings,
    evidence_id: Uuid,
    image_urls: &[String],
) -> Result<CrawlStats> {
    let mut stats = CrawlStats::default();

    for image_url in image_urls {
        if kb
            .media_exists_for_entity_url("evidence", evidence_id, image_url)
            .await?
        {
            continue;
        }
        if let Some(existing) = kb.get_media_by_original_url(image_url).await? {
            kb.insert_media(
                "evidence",
                evidence_id,
                &existing.file_path,
                existing.original_url.as_deref(),
                existing.content_type.as_deref(),
                existing.file_size_bytes,
            )
            .await?;
            stats.media_attached += 1;
            stats.media_reused += 1;
            continue;
        }

        let (bytes, content_type) = match client.get_bytes(image_url).await {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(url = %image_url, error = %error, "image download failed");
                let _ = kb
                    .increment_crawl_failure(image_url, "image_download", &format!("{error:#}"))
                    .await;
                continue;
            }
        };

        let hash = Sha256::digest(&bytes);
        let hash_hex = hex::encode(hash);
        let extension = infer_extension(image_url, content_type.as_deref());
        let relative_path = settings
            .media_root_relative
            .join("evidence")
            .join(evidence_id.to_string())
            .join(format!("{hash_hex}.{extension}"));
        let absolute_path = settings
            .media_root
            .join("evidence")
            .join(evidence_id.to_string())
            .join(format!("{hash_hex}.{extension}"));

        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        if fs::metadata(&absolute_path).await.is_err() {
            fs::write(&absolute_path, &bytes).await?;
        }

        kb.insert_media(
            "evidence",
            evidence_id,
            &path_to_string(&relative_path),
            Some(image_url),
            content_type.as_deref(),
            Some(bytes.len() as i64),
        )
        .await?;
        stats.media_attached += 1;
        stats.media_downloaded += 1;
    }

    Ok(stats)
}

async fn is_post_fully_ingested(
    kb: &KnowledgeBase,
    source_url: &str,
    subjects: &[(String, &'static str)],
    image_urls: &[String],
) -> Result<bool> {
    for (value, subject_type) in subjects {
        let Some(subject) = kb.get_subject_by_value(value, subject_type).await? else {
            return Ok(false);
        };
        let Some(evidence_id) = kb
            .get_evidence_id_for_subject_source_url(subject.id, CRAWL_SOURCE, source_url)
            .await?
        else {
            return Ok(false);
        };
        for image_url in image_urls {
            if !kb
                .media_exists_for_entity_url("evidence", evidence_id, image_url)
                .await?
            {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

async fn link_detection_pass(kb: &KnowledgeBase) -> Result<LinkStats> {
    let evidence_records = kb.get_crawl_evidence_with_mentions(CRAWL_SOURCE).await?;
    let mut stats = LinkStats::default();
    let mut touched_subjects = HashSet::new();

    for record in evidence_records {
        touched_subjects.insert(record.subject_id);
        for mentioned_value in record.mentioned_subjects {
            let (subject_type, subject_value) = parse_mentioned_subject(&mentioned_value);
            let Some(linked_subject) = kb
                .get_subject_by_value(&subject_value, subject_type)
                .await?
            else {
                continue;
            };
            if linked_subject.id == record.subject_id {
                continue;
            }
            match kb
                .upsert_subject_link(
                    record.subject_id,
                    linked_subject.id,
                    "co_mentioned",
                    Some(record.id),
                )
                .await
            {
                Ok(()) => {
                    touched_subjects.insert(linked_subject.id);
                    stats.links_created += 1;
                }
                Err(error) => {
                    tracing::debug!(error = %error, "subject link upsert failed");
                    stats.link_errors += 1;
                }
            }
        }
    }

    for subject_id in &touched_subjects {
        kb.recalculate_risk(*subject_id).await?;
        if let Some(risk_score) = kb.get_crawl_risk_floor(*subject_id, CRAWL_SOURCE).await? {
            kb.set_crawl_risk(*subject_id, risk_score, numeric_to_risk_level(risk_score))
                .await?;
        }
    }
    stats.subjects_risk_updated = touched_subjects.len();

    Ok(stats)
}

fn merge_stats(target: &mut CrawlStats, source: &CrawlStats) {
    target.posts_discovered += source.posts_discovered;
    target.posts_processed += source.posts_processed;
    target.posts_skipped += source.posts_skipped;
    target.post_errors += source.post_errors;
    target.subjects_upserted += source.subjects_upserted;
    target.evidence_inserted += source.evidence_inserted;
    target.evidence_updated += source.evidence_updated;
    target.media_attached += source.media_attached;
    target.media_reused += source.media_reused;
    target.media_downloaded += source.media_downloaded;
}

fn extract_image_urls(document: &Html) -> Vec<String> {
    let gallery_selector = Selector::parse("a[href][class*='gallery'], a[href][data-fancybox]")
        .expect("valid selector");
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for element in document.select(&gallery_selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        if !href.starts_with("http") {
            continue;
        }
        let Ok(url) = url::Url::parse(href) else {
            continue;
        };
        let Some(host) = url.host_str() else {
            continue;
        };
        if !host.ends_with("checkscam.vn") {
            continue;
        }
        let path = url.path().to_ascii_lowercase();
        if !path.contains("/wp-content/uploads/") {
            continue;
        }
        let clean = href.split('?').next().unwrap_or(href);
        if !matches!(
            Path::new(&clean.to_ascii_lowercase())
                .extension()
                .and_then(|v| v.to_str()),
            Some("jpg" | "jpeg" | "png" | "webp" | "gif")
        ) {
            continue;
        }
        if seen.insert(clean.to_string()) {
            urls.push(clean.to_string());
        }
    }

    urls
}

fn dedup_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_subject_value(value: &str, subject_type: &str) -> String {
    let trimmed = value.trim();
    match subject_type {
        "phone" => trimmed
            .chars()
            .filter(|ch| ch.is_ascii_digit())
            .collect::<String>(),
        "bank" => trimmed
            .chars()
            .filter(|ch| ch.is_ascii_digit())
            .collect::<String>(),
        _ => trimmed.to_string(),
    }
}

fn crawl_risk_score(report_count: u32) -> f32 {
    match report_count {
        10.. => 0.9,
        5..=9 => 0.75,
        2..=4 => 0.55,
        1 => 0.35,
        _ => 0.2,
    }
}

fn infer_subject_type(value: &str) -> &'static str {
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() >= 9 && digits.starts_with('0') {
        "phone"
    } else {
        "bank"
    }
}

fn parse_mentioned_subject(value: &str) -> (&str, String) {
    if let Some((subject_type, raw_value)) = value.split_once(':') {
        if matches!(subject_type, "phone" | "bank") {
            return (subject_type, raw_value.to_string());
        }
    }
    (infer_subject_type(value), value.to_string())
}

fn infer_extension(image_url: &str, content_type: Option<&str>) -> &'static str {
    if let Some(content_type) = content_type {
        return match content_type.split(';').next().unwrap_or("").trim() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/svg+xml" => "svg",
            _ => infer_extension_from_url(image_url),
        };
    }
    infer_extension_from_url(image_url)
}

fn infer_extension_from_url(image_url: &str) -> &'static str {
    let Some(path) = url::Url::parse(image_url)
        .ok()
        .map(|url| url.path().to_string())
    else {
        return "img";
    };
    match Path::new(&path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "jpg",
        Some("png") => "png",
        Some("webp") => "webp",
        Some("gif") => "gif",
        Some("svg") => "svg",
        _ => "img",
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{
        Html, crawl_risk_score, extract_image_urls, infer_subject_type, normalize_subject_value,
    };

    #[test]
    fn extract_image_urls_keeps_only_checkscam_upload_images() {
        let html = Html::parse_document(
            r#"
            <html>
                <body>
                    <a class="gallery" href="https://checkscam.vn/wp-content/uploads/2026/05/evidence-1.jpg?v=123"><img src="thumb.jpg" /></a>
                    <a class="gallery" href="https://checkscam.vn/wp-content/themes/site/logo.png"><img src="thumb.jpg" /></a>
                    <a class="gallery" href="https://cdn.example.com/wp-content/uploads/external.jpg"><img src="thumb.jpg" /></a>
                    <a class="gallery" href="https://checkscam.vn/wp-content/uploads/2026/05/evidence-2.webp"><img src="thumb.jpg" /></a>
                    <img src="https://checkscam.vn/wp-content/uploads/2025/10/Shopee.png" />
                </body>
            </html>
            "#,
        );

        let urls = extract_image_urls(&html);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().all(|url| url.contains("/wp-content/uploads/")));
        assert!(!urls.iter().any(|url| url.contains("?v=")));
    }

    #[test]
    fn infer_subject_type_prefers_phone_for_zero_prefixed_numbers() {
        assert_eq!(infer_subject_type("0912345678"), "phone");
        assert_eq!(infer_subject_type("123456789012"), "bank");
    }

    #[test]
    fn normalize_subject_value_strips_non_digits_for_phone_and_bank() {
        assert_eq!(
            normalize_subject_value("0912 345 678", "phone"),
            "0912345678"
        );
        assert_eq!(
            normalize_subject_value("1234 5678 90", "bank"),
            "1234567890"
        );
    }

    #[test]
    fn crawl_risk_score_scales_with_report_count() {
        assert_eq!(crawl_risk_score(0), 0.2);
        assert_eq!(crawl_risk_score(1), 0.35);
        assert_eq!(crawl_risk_score(6), 0.75);
        assert_eq!(crawl_risk_score(12), 0.9);
    }
}
