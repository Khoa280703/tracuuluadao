use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::AppState;
use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize)]
pub struct SubmitReportRequest {
    pub value: String,
    pub subject_type: String,
    pub description: String,
    pub category: Option<String>,
}

pub async fn submit_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<SubmitReportRequest>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    validate_request(&body)?;

    let ip_hash = sha256_hex(&extract_client_ip(&headers, addr));
    if !knowledge_base.check_report_rate_limit(&ip_hash, 5).await? {
        return Err(AppError::Config(
            "rate limit exceeded (max 5 reports per day)".to_string(),
        ));
    }

    let subject_id = knowledge_base
        .upsert_subject(&body.value, body.subject_type.as_str())
        .await?;
    if !knowledge_base
        .check_duplicate_report(&ip_hash, subject_id)
        .await?
    {
        return Err(AppError::Config(
            "duplicate report for this subject within 24 hours".to_string(),
        ));
    }

    let report_id = knowledge_base
        .insert_user_report(
            subject_id,
            &ip_hash,
            body.description.trim(),
            body.category.as_deref().map(str::trim),
        )
        .await?;

    Ok((StatusCode::CREATED, Json(json!({ "id": report_id }))))
}

fn validate_request(body: &SubmitReportRequest) -> AppResult<()> {
    if !matches!(body.subject_type.as_str(), "phone" | "bank" | "url") {
        return Err(AppError::Config(
            "subject_type must be one of phone|bank|url".to_string(),
        ));
    }
    if normalized_subject_value(body.value.as_str(), body.subject_type.as_str()).is_empty() {
        return Err(AppError::Config(
            "value is invalid for the selected subject_type".to_string(),
        ));
    }
    if body.description.trim().is_empty() {
        return Err(AppError::Config("description is required".to_string()));
    }
    if body.description.len() > 2000 {
        return Err(AppError::Config(
            "description too long (max 2000 chars)".to_string(),
        ));
    }
    Ok(())
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn extract_client_ip(headers: &HeaderMap, addr: SocketAddr) -> String {
    if is_trusted_proxy(addr.ip()) {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return forwarded.to_string();
        }
    }
    addr.ip().to_string()
}

fn normalized_subject_value(value: &str, subject_type: &str) -> String {
    match subject_type {
        "phone" => value.chars().filter(|ch| ch.is_ascii_digit()).collect(),
        "bank" => value.split_whitespace().collect::<String>(),
        "url" => value.trim().trim_end_matches('/').to_ascii_lowercase(),
        _ => value.trim().to_string(),
    }
}

fn is_trusted_proxy(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}
