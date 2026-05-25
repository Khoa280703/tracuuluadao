use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::pipeline::QueryType;

#[derive(Debug, Deserialize)]
pub struct SubjectParams {
    #[serde(rename = "type")]
    pub subject_type: Option<String>,
    pub depth: Option<i32>,
}

pub async fn get_subject(
    Path(value): Path<String>,
    Query(params): Query<SubjectParams>,
    State(state): State<AppState>,
) -> AppResult<Json<crate::knowledge_base::SubjectHistory>> {
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    let subject_type = parse_query_type(params.subject_type.as_deref())?;
    let subject = knowledge_base
        .get_subject_by_value(&value, subject_type.as_str())
        .await?
        .ok_or_else(|| AppError::NotFound(format!("subject not found: {value}")))?;
    let history = knowledge_base.get_subject_history(subject.id, 10).await?;
    Ok(Json(history))
}

pub async fn get_network(
    Path(value): Path<String>,
    Query(params): Query<SubjectParams>,
    State(state): State<AppState>,
) -> AppResult<Json<crate::knowledge_base::NetworkGraph>> {
    let knowledge_base = state
        .knowledge_base
        .as_ref()
        .ok_or_else(|| AppError::Config("knowledge base not available".to_string()))?;
    let subject_type = parse_query_type(params.subject_type.as_deref())?;
    let subject = knowledge_base
        .get_subject_by_value(&value, subject_type.as_str())
        .await?
        .ok_or_else(|| AppError::NotFound(format!("subject not found: {value}")))?;
    let graph = knowledge_base
        .get_network(subject.id, params.depth.unwrap_or(2))
        .await?;
    Ok(Json(graph))
}

fn parse_query_type(value: Option<&str>) -> AppResult<QueryType> {
    match value.unwrap_or("phone").parse::<QueryType>() {
        Ok(query_type) => Ok(query_type),
        Err(_) => Err(AppError::Config(
            "invalid query type: supported phone|bank|url".to_string(),
        )),
    }
}
