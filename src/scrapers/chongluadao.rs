use std::time::Instant;

use serde_json::{Value, json};

use crate::scrapers::http_client::HttpClient;
use crate::scrapers::{ScrapedReport, ScrapedResult, SourceName};

pub async fn scrape(client: Option<HttpClient>, query: &str) -> ScrapedResult {
    let started_at = Instant::now();
    let Some(client) = client else {
        return ScrapedResult::failure(
            SourceName::ChongLuaDao,
            query,
            started_at,
            "http client unavailable",
        );
    };

    match scrape_impl(&client, query, started_at).await {
        Ok(result) => result,
        Err(error) => ScrapedResult::failure(
            SourceName::ChongLuaDao,
            query,
            started_at,
            error.to_string(),
        ),
    }
}

async fn scrape_impl(
    client: &HttpClient,
    query: &str,
    started_at: Instant,
) -> anyhow::Result<ScrapedResult> {
    let phone_url = format!("https://feeds.chongluadao.vn/checkphone?q={query}");
    let exists_url = format!("https://feeds.chongluadao.vn/reports/check-exists?q={query}");
    let phone_json: Value = client.get_json_value(&phone_url).await?;
    let exists_json: Value = client.get_json_value(&exists_url).await?;
    let mut result = ScrapedResult::success(SourceName::ChongLuaDao, query, started_at);
    result.reports = collect_reports(&phone_json);
    result.metadata = json!({
        "phone_lookup": phone_json,
        "check_exists": exists_json,
    });
    Ok(result)
}

fn collect_reports(payload: &Value) -> Vec<ScrapedReport> {
    let Some(items) = payload.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let data = item.get("data")?;
            if data.is_null() {
                return None;
            }
            let title = item
                .get("name")
                .or_else(|| item.get("source"))
                .and_then(Value::as_str)
                .unwrap_or("chongluadao")
                .to_owned();
            Some(ScrapedReport {
                title,
                url: String::new(),
                content: serde_json::to_string(data).unwrap_or_default(),
                date: None,
            })
        })
        .collect()
}
