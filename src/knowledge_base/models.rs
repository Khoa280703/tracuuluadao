use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Subject {
    pub id: Uuid,
    pub value: String,
    pub subject_type: String,
    pub risk_score: f32,
    pub risk_level: String,
    pub report_count: i32,
    pub investigation_count: i32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct InvestigationRecord {
    pub id: Uuid,
    pub risk_level: String,
    pub risk_score_numeric: f32,
    pub confidence: f32,
    pub sources_analyzed: i32,
    pub sources_success_ratio: f32,
    pub quality_score: f32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LinkedSubject {
    pub id: Uuid,
    pub value: String,
    pub subject_type: String,
    pub risk_score: f32,
    pub risk_level: String,
    pub report_count: i32,
    pub investigation_count: i32,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub link_type: String,
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserReportRecord {
    pub id: Uuid,
    pub description: String,
    pub category: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectHistory {
    pub subject: Subject,
    pub recent_investigations: Vec<InvestigationRecord>,
    pub approved_reports: Vec<UserReportRecord>,
    pub linked_subjects: Vec<LinkedSubject>,
    pub all_risk_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalContextSnapshot {
    pub investigation_count: i32,
    pub last_risk_level: String,
    pub approved_reports: i32,
    pub linked_subjects_count: i32,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceInput {
    pub subject_id: Uuid,
    pub investigation_id: Option<Uuid>,
    pub source: String,
    pub evidence_type: String,
    pub data: serde_json::Value,
    pub risk_signals: Vec<String>,
    pub mentioned_subjects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NetworkNode {
    pub id: Uuid,
    pub value: String,
    pub subject_type: String,
    pub risk_level: String,
    pub risk_score: f32,
    pub depth: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NetworkEdge {
    pub subject_a_id: Uuid,
    pub subject_b_id: Uuid,
    pub link_type: String,
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkGraph {
    pub root_subject_id: Uuid,
    pub nodes: Vec<NetworkNode>,
    pub edges: Vec<NetworkEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserReportWithSubject {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub subject_value: String,
    pub subject_type: String,
    pub subject_risk_level: String,
    pub description: String,
    pub category: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<DateTime<Utc>>,
}
