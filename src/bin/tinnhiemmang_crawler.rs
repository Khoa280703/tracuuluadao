use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::json;
use tracuuluadao::cache::CacheService;
use tracuuluadao::knowledge_base::{EvidenceInput, KnowledgeBase};
use tracuuluadao::logging;
use url::Url;

const SOURCE: &str = "tinnhiemmang_crawl";
const BASE_URL: &str = "https://tinnhiemmang.vn/filterObj";
const URL_RISK_SCORE: f32 = 0.95;
const URL_RISK_LEVEL: &str = "critical";

#[derive(Parser, Debug)]
#[command(name = "tinnhiemmang-crawler")]
#[command(about = "Bulk-crawl tinnhiemmang.vn blacklist into the knowledge base")]
struct Args {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long, default_value_t = 0)]
    max_pages: usize,
    #[arg(long, default_value_t = 1)]
    from_page: usize,
    #[arg(long, default_value_t = 150)]
    delay_ms: u64,
    #[arg(long, value_enum, default_value_t = CrawlMode::All)]
    mode: CrawlMode,
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum CrawlMode {
    All,
    Web,
    Social,
}

#[derive(Debug, Default)]
struct Stats {
    pages_fetched: usize,
    records_seen: usize,
    records_written: usize,
    evidence_updated: usize,
    links_created: usize,
}

#[derive(Debug)]
struct ListingRecord {
    fake_url: String,
    fake_type: &'static str,
    detected_at: Option<String>,
    status_text: String,
    status_class: Option<String>,
    org_name: Option<String>,
    org_detail_url: Option<String>,
    org_logo_url: Option<String>,
    listing_page: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    let _log_guards = logging::init()?;

    let pool = CacheService::connect_pool(&args.database_url)?;
    let kb = KnowledgeBase::new(pool);
    kb.ensure_schema().await?;

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0")
        .build()
        .context("failed to build http client")?;

    let fake_types = match args.mode {
        CrawlMode::All => vec!["web", "social"],
        CrawlMode::Web => vec!["web"],
        CrawlMode::Social => vec!["social"],
    };

    let mut stats = Stats::default();
    for fake_type in fake_types {
        crawl_type(&client, &kb, &args, fake_type, &mut stats).await?;
    }

    tracing::info!(
        pages_fetched = stats.pages_fetched,
        records_seen = stats.records_seen,
        records_written = stats.records_written,
        evidence_updated = stats.evidence_updated,
        links_created = stats.links_created,
        dry_run = args.dry_run,
        "tinnhiemmang crawl complete"
    );
    Ok(())
}

async fn crawl_type(
    client: &Client,
    kb: &KnowledgeBase,
    args: &Args,
    fake_type: &'static str,
    stats: &mut Stats,
) -> Result<()> {
    let item_sel = Selector::parse("li.item1").unwrap();
    let url_sel = Selector::parse(".meta .webkit-box-2").unwrap();
    let date_sel = Selector::parse(".date").unwrap();
    let org_sel = Selector::parse(".hidden-lg-down.org a, .org a, .code a").unwrap();
    let org_img_sel = Selector::parse(".org img").unwrap();
    let status_sel = Selector::parse(".status > div").unwrap();
    let page_sel = Selector::parse(".pagination a.page-link, .pagination span.page-link").unwrap();
    let start_page = args.from_page.max(1);
    let mut page = start_page;
    let mut last_page = None;

    loop {
        if args.max_pages > 0 && (page - start_page + 1) > args.max_pages {
            break;
        }
        let page_url = format!("{BASE_URL}?type=fake&fakeType[]={fake_type}&page={page}");
        let html = client
            .get(&page_url)
            .send()
            .await
            .with_context(|| format!("failed to fetch {page_url}"))?
            .error_for_status()
            .with_context(|| format!("non-success status for {page_url}"))?
            .text()
            .await
            .with_context(|| format!("failed to read body for {page_url}"))?;
        let document = Html::parse_document(&html);
        let items: Vec<ListingRecord> = document
            .select(&item_sel)
            .filter_map(|item| {
                let fake_url =
                    clean_text(&item.select(&url_sel).next()?.text().collect::<String>());
                if fake_url.is_empty() {
                    return None;
                }
                let status_node = item.select(&status_sel).next();
                let org_node = item.select(&org_sel).next();
                Some(ListingRecord {
                    fake_url,
                    fake_type,
                    detected_at: item.select(&date_sel).next().and_then(|node| {
                        clean_text(&node.text().collect::<String>())
                            .strip_prefix("Đã phát hiện ngày ")
                            .map(str::to_string)
                    }),
                    status_text: status_node
                        .map(|node| clean_text(&node.text().collect::<String>()))
                        .unwrap_or_default(),
                    status_class: status_node
                        .and_then(|node| node.value().attr("class"))
                        .map(str::to_string),
                    org_name: org_node.as_ref().and_then(|node| {
                        normalize_org_name(&clean_text(&node.text().collect::<String>()))
                    }),
                    org_detail_url: org_node
                        .and_then(|node| node.value().attr("href"))
                        .map(to_absolute_url),
                    org_logo_url: item
                        .select(&org_img_sel)
                        .next()
                        .and_then(|node| node.value().attr("src"))
                        .map(to_absolute_url),
                    listing_page: page_url.clone(),
                })
            })
            .collect();
        if items.is_empty() {
            break;
        }
        if last_page.is_none() {
            last_page = document
                .select(&page_sel)
                .filter_map(|node| {
                    clean_text(&node.text().collect::<String>())
                        .parse::<usize>()
                        .ok()
                })
                .max();
        }
        stats.pages_fetched += 1;
        stats.records_seen += items.len();
        for item in items {
            upsert_record(kb, &item, args.dry_run, stats).await?;
        }
        tracing::info!(
            fake_type,
            page,
            last_page,
            records_seen = stats.records_seen,
            "fetched tinnhiemmang page"
        );
        if last_page.is_some_and(|limit| page >= limit) {
            break;
        }
        page += 1;
        if args.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
        }
    }
    Ok(())
}

async fn upsert_record(
    kb: &KnowledgeBase,
    item: &ListingRecord,
    dry_run: bool,
    stats: &mut Stats,
) -> Result<()> {
    if dry_run {
        stats.records_written += 1;
        if item.org_name.as_deref().is_some_and(is_known_org) {
            stats.links_created += 1;
        }
        return Ok(());
    }

    let url_subject_id = kb.upsert_subject(&item.fake_url, "url").await?;
    let evidence = EvidenceInput {
        subject_id: url_subject_id,
        investigation_id: None,
        source: SOURCE.to_string(),
        evidence_type: "blacklist_record".to_string(),
        data: json!({
            "source_url": item.fake_url,
            "fake_type": item.fake_type,
            "detected_at": item.detected_at,
            "status_text": item.status_text,
            "status_class": item.status_class,
            "impersonated_org_name": item.org_name,
            "impersonated_org_url": item.org_detail_url,
            "org_logo_url": item.org_logo_url,
            "listing_page": item.listing_page,
        }),
        risk_signals: vec![
            "tinnhiemmang_blacklist".to_string(),
            format!("tinnhiemmang_type:{}", item.fake_type),
            format!("tinnhiemmang_status:{}", slugify(&item.status_text)),
        ],
        mentioned_subjects: Vec::new(),
    };
    let evidence_id = if let Some(id) = kb
        .get_evidence_id_for_subject_source_url_and_fake_type(
            url_subject_id,
            SOURCE,
            &item.fake_url,
            item.fake_type,
        )
        .await?
    {
        kb.update_evidence(id, evidence).await?;
        stats.evidence_updated += 1;
        id
    } else {
        kb.insert_evidence(evidence).await?
    };
    kb.set_crawl_risk(url_subject_id, URL_RISK_SCORE, URL_RISK_LEVEL)
        .await?;
    if let Some(org_name) = item.org_name.as_deref().filter(|name| is_known_org(name)) {
        let org_subject_id = kb.upsert_subject(org_name, "organization").await?;
        kb.upsert_subject_link(
            url_subject_id,
            org_subject_id,
            "impersonates",
            Some(evidence_id),
        )
        .await?;
        stats.links_created += 1;
    }
    stats.records_written += 1;
    Ok(())
}

fn clean_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn is_known_org(value: &str) -> bool {
    let normalized = value.trim();
    !normalized.is_empty() && normalized != "Chưa xác định"
}

fn normalize_org_name(value: &str) -> Option<String> {
    let normalized = clean_text(value)
        .strip_prefix("Mạo danh tổ chức:")
        .unwrap_or(value)
        .trim()
        .to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn to_absolute_url(value: &str) -> String {
    Url::parse(BASE_URL)
        .ok()
        .and_then(|base| base.join(value).ok())
        .map(|url| url.to_string())
        .unwrap_or_else(|| value.to_string())
}

fn slugify(value: &str) -> String {
    let lowered = value.to_lowercase();
    let filtered = lowered
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    filtered.trim_matches('-').replace("--", "-")
}
