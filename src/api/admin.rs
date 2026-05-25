use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::AppState;
use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize)]
pub struct AdminListParams {
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

pub async fn list_reports(
    headers: HeaderMap,
    Query(params): Query<AdminListParams>,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<crate::knowledge_base::UserReportWithSubject>>> {
    check_admin_key(&headers, &state)?;
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    let reports = knowledge_base
        .list_reports_by_status(
            params.status.as_deref().unwrap_or("pending"),
            params.limit.unwrap_or(50).clamp(1, 100),
            params.offset.unwrap_or(0).max(0),
        )
        .await?;
    Ok(Json(reports))
}

pub async fn approve_report(
    headers: HeaderMap,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let reviewed_by = check_admin_key(&headers, &state)?;
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    let report_id = parse_report_id(&id)?;
    let subject_id = knowledge_base
        .update_report_status(report_id, "approved", &reviewed_by)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("report not found: {id}")))?;
    knowledge_base.recalculate_risk(subject_id).await?;
    Ok(Json(json!({ "ok": true, "status": "approved" })))
}

pub async fn reject_report(
    headers: HeaderMap,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let reviewed_by = check_admin_key(&headers, &state)?;
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    let report_id = parse_report_id(&id)?;
    let subject_id = knowledge_base
        .update_report_status(report_id, "rejected", &reviewed_by)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("report not found: {id}")))?;
    knowledge_base.recalculate_risk(subject_id).await?;
    Ok(Json(json!({ "ok": true, "status": "rejected" })))
}

fn check_admin_key(headers: &HeaderMap, state: &AppState) -> AppResult<String> {
    let expected = state
        .config
        .admin_api_key
        .as_deref()
        .ok_or_else(|| AppError::Config("admin API not configured".to_string()))?;
    let provided = headers
        .get("x-admin-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("missing X-Admin-Key header".to_string()))?;
    if provided != expected {
        return Err(AppError::Unauthorized("invalid admin key".to_string()));
    }
    Ok("admin_api_key".to_string())
}

fn parse_report_id(id: &str) -> AppResult<Uuid> {
    Uuid::parse_str(id).map_err(|_| AppError::Config(format!("invalid report id: {id}")))
}
