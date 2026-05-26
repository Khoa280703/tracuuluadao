use std::time::Instant;

use scraper::{Html, Selector};
use serde::Serialize;
use serde_json::json;
use url::Url;

use crate::scrapers::http_client::HttpClient;
use crate::scrapers::{ScrapedReport, ScrapedResult, SearchResult, SourceName};

const ADMIN_VN_BASE_URL: &str = "https://admin.vn";
const ADMIN_VN_DETAIL_FETCH_LIMIT: usize = 3;

#[derive(Debug, Clone, Serialize)]
struct AdminSearchCard {
    title: String,
    url: String,
    amount: String,
    phone: String,
    account_number: String,
    bank: String,
    views: String,
    date: String,
}

#[derive(Debug, Clone, Serialize)]
struct AdminDetailRecord {
    title: String,
    url: String,
    owner_name: String,
    account_number: String,
    bank: String,
    amount: String,
    category: String,
    complaint: String,
    evidence_urls: Vec<String>,
    approved_votes: u64,
    disapproved_votes: u64,
    published_at: Option<String>,
}

pub async fn scrape(client: Option<HttpClient>, query: &str) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::AdminVN,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, started_at).await {
        Ok(result) => result,
        Err(error) => {
            ScrapedResult::failure(SourceName::AdminVN, query, started_at, error.to_string())
        }
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let html = client.get_text_from_url(build_search_url(query)?).await?;
    let report_count = extract_report_count(&html)?;
    let cards = parse_search_cards(&html)?;

    let mut reports = Vec::new();
    let mut detail_records = Vec::new();
    for card in cards.iter().take(ADMIN_VN_DETAIL_FETCH_LIMIT) {
        let detail_html = match client.get_text(&card.url).await {
            Ok(html) => html,
            Err(error) => {
                tracing::debug!(url = %card.url, error = %error, "admin.vn detail fetch failed");
                continue;
            }
        };
        if let Some(detail) = parse_detail_record(&detail_html, &card.url)? {
            reports.push(ScrapedReport {
                title: detail.title.clone(),
                url: detail.url.clone(),
                content: build_report_content(&detail),
                date: detail
                    .published_at
                    .clone()
                    .or_else(|| (!card.date.is_empty()).then(|| card.date.clone())),
            });
            detail_records.push(detail);
        }
    }

    if reports.is_empty() {
        reports = cards
            .iter()
            .map(|card| ScrapedReport {
                title: card.title.clone(),
                url: card.url.clone(),
                content: format!(
                    "So tien: {}; SDT: {}; STK: {}; Ngan hang: {}; Luot xem: {}",
                    card.amount, card.phone, card.account_number, card.bank, card.views
                ),
                date: (!card.date.is_empty()).then(|| card.date.clone()),
            })
            .collect();
    }

    let mut result = ScrapedResult::success(SourceName::AdminVN, query, started_at);
    result.reports = reports;
    result.search_results = cards
        .iter()
        .map(|card| SearchResult {
            title: card.title.clone(),
            url: card.url.clone(),
            snippet: format!(
                "So tien: {}; SDT: {}; STK: {}; Ngan hang: {}; Ngay: {}",
                card.amount, card.phone, card.account_number, card.bank, card.date
            ),
        })
        .collect();
    result.metadata = json!({
        "report_count": report_count.max(cards.len() as u64),
        "result_cards": cards,
        "detail_reports": detail_records,
    });
    result.raw_html = Some(html.chars().take(2_500).collect());
    Ok(result)
}

fn build_search_url(query: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(&format!("{ADMIN_VN_BASE_URL}/scams"))?;
    url.query_pairs_mut().append_pair("keyword", query);
    Ok(url)
}

fn extract_report_count(html: &str) -> anyhow::Result<u64> {
    let document = Html::parse_document(html);
    let alert_selector = Selector::parse(".alert.alert-danger").expect("valid selector");
    Ok(document
        .select(&alert_selector)
        .next()
        .map(|item| number(&text(item.text().collect::<Vec<_>>().join(" "))))
        .unwrap_or(0))
}

fn parse_search_cards(html: &str) -> anyhow::Result<Vec<AdminSearchCard>> {
    let document = Html::parse_document(html);
    let card_selector = Selector::parse(".scam-card").expect("valid selector");
    let link_selector = Selector::parse("a.stretched-link").expect("valid selector");
    let title_selector =
        Selector::parse(".scam-title .limit, .scam-title").expect("valid selector");
    let column_selector = Selector::parse(".scam-column").expect("valid selector");
    let mut cards = Vec::new();

    for card in document.select(&card_selector) {
        let Some(link) = card.select(&link_selector).next() else {
            continue;
        };
        let Some(url) = absolute_url(link.value().attr("href")) else {
            continue;
        };
        let columns = card
            .select(&column_selector)
            .map(|item| text(item.text().collect::<Vec<_>>().join(" ")))
            .collect::<Vec<_>>();
        cards.push(AdminSearchCard {
            title: card
                .select(&title_selector)
                .next()
                .map(|item| text(item.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default(),
            url,
            amount: columns.get(1).cloned().unwrap_or_default(),
            phone: columns.get(2).cloned().unwrap_or_default(),
            account_number: columns.get(3).cloned().unwrap_or_default(),
            bank: columns.get(4).cloned().unwrap_or_default(),
            views: columns.get(5).cloned().unwrap_or_default(),
            date: columns.get(6).cloned().unwrap_or_default(),
        });
    }

    Ok(cards)
}

fn parse_detail_record(html: &str, url: &str) -> anyhow::Result<Option<AdminDetailRecord>> {
    let document = Html::parse_document(html);
    let item_selector = Selector::parse(".information-item").expect("valid selector");
    let value_selector = Selector::parse(".information-item_value").expect("valid selector");
    let icon_selector = Selector::parse(".information-item_title img").expect("valid selector");
    let title_selector = Selector::parse("title").expect("valid selector");
    let evidence_selector =
        Selector::parse(r#"a[data-fancybox="scammer-images"]"#).expect("valid selector");
    let meta_selector =
        Selector::parse(r#"meta[property="article:published_time"]"#).expect("valid selector");
    let approved_selector =
        Selector::parse(".information-item_vote__approved").expect("valid selector");
    let disapproved_selector =
        Selector::parse(".information-item_vote__disapprove").expect("valid selector");

    let mut detail = AdminDetailRecord {
        title: document
            .select(&title_selector)
            .next()
            .map(|item| text(item.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_else(|| "admin.vn".to_owned()),
        url: url.to_owned(),
        owner_name: String::new(),
        account_number: String::new(),
        bank: String::new(),
        amount: String::new(),
        category: String::new(),
        complaint: String::new(),
        evidence_urls: document
            .select(&evidence_selector)
            .filter_map(|item| absolute_url(item.value().attr("href")))
            .collect(),
        approved_votes: 0,
        disapproved_votes: 0,
        published_at: document
            .select(&meta_selector)
            .next()
            .and_then(|item| item.value().attr("content"))
            .map(str::to_owned),
    };

    for item in document.select(&item_selector) {
        let value = item
            .select(&value_selector)
            .next()
            .map(|value| text(value.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        let icon = item
            .select(&icon_selector)
            .next()
            .and_then(|value| value.value().attr("src"))
            .unwrap_or_default();
        if icon.contains("/user.png") {
            detail.owner_name = value;
        } else if icon.contains("/credit.png") {
            detail.account_number = value;
        } else if icon.contains("/bank.png") {
            detail.bank = value;
        } else if icon.contains("/price.png") {
            detail.amount = value;
        } else if icon.contains("/list.png") {
            detail.category = value;
        } else if icon.contains("/content.png") {
            detail.complaint = value;
        }
    }

    detail.approved_votes = document
        .select(&approved_selector)
        .next()
        .map(|item| number(&text(item.text().collect::<Vec<_>>().join(" "))))
        .unwrap_or(0);
    detail.disapproved_votes = document
        .select(&disapproved_selector)
        .next()
        .map(|item| number(&text(item.text().collect::<Vec<_>>().join(" "))))
        .unwrap_or(0);
    if detail.owner_name.is_empty()
        && detail.account_number.is_empty()
        && detail.complaint.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(detail))
}

fn build_report_content(detail: &AdminDetailRecord) -> String {
    format!(
        "Chu TK: {}; STK: {}; Ngan hang: {}; So tien: {}; Hang muc: {}; Noi dung: {}; Bang chung: {}; Tan thanh: {}; Khong tan thanh: {}",
        detail.owner_name,
        detail.account_number,
        detail.bank,
        detail.amount,
        detail.category,
        detail.complaint,
        detail.evidence_urls.len(),
        detail.approved_votes,
        detail.disapproved_votes
    )
}

fn absolute_url(href: Option<&str>) -> Option<String> {
    let href = href?.trim();
    if href.is_empty() {
        return None;
    }
    Url::parse(ADMIN_VN_BASE_URL)
        .ok()?
        .join(href)
        .ok()
        .map(Into::into)
}

fn text(value: String) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(':', "")
        .trim()
        .to_owned()
}

fn number(value: &str) -> u64 {
    value
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}
