use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::scrapers::{ScrapedResult, SearchResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryType {
    Phone,
    Bank,
    Url,
}

impl QueryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Bank => "bank",
            Self::Url => "url",
        }
    }
}

impl FromStr for QueryType {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "phone" => Ok(Self::Phone),
            "bank" => Ok(Self::Bank),
            "url" => Ok(Self::Url),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Investigation {
    pub query: String,
    pub query_type: QueryType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub key_facts: Vec<String>,
    #[serde(default)]
    pub phone_mentions: Vec<String>,
    #[serde(default)]
    pub risk_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedUrl {
    pub url: String,
    pub reason: String,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtraction {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub risk_signals: Vec<String>,
    #[serde(default)]
    pub related_numbers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationResult {
    pub query: String,
    pub query_type: QueryType,
    pub risk_level: String,
    pub confidence: f32,
    pub sources_analyzed: usize,
    pub duration_ms: u64,
    pub summaries: Vec<AgentSummary>,
    pub extractions: Vec<AgentExtraction>,
    pub detective_markdown: String,
    pub scraped_results: Vec<ScrapedResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvestigationEvent {
    PhaseStart {
        phase: u8,
        label: String,
        total_sources: Option<usize>,
    },
    SourceStatus {
        source: String,
        status: String,
        found: usize,
    },
    Progress {
        phase: u8,
        message: String,
    },
    SummaryResult {
        source: String,
        result: AgentSummary,
    },
    UrlAssessment {
        selected: usize,
        total: usize,
        urls: Vec<SelectedUrl>,
    },
    ExtractionResult {
        url: String,
        result: AgentExtraction,
    },
    DetectiveStream {
        chunk: String,
        done: bool,
        replace: bool,
    },
    Complete {
        risk_level: String,
        confidence: f32,
        sources_analyzed: usize,
        duration_ms: u64,
    },
    Error {
        phase: Option<u8>,
        message: String,
        recoverable: bool,
    },
}

impl InvestigationEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::PhaseStart { .. } => "phase_start",
            Self::SourceStatus { .. } => "source_status",
            Self::Progress { .. } => "progress",
            Self::SummaryResult { .. } => "summary_result",
            Self::UrlAssessment { .. } => "url_assessment",
            Self::ExtractionResult { .. } => "extraction_result",
            Self::DetectiveStream { .. } => "detective_stream",
            Self::Complete { .. } => "complete",
            Self::Error { .. } => "error",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UrlAssessmentResponse {
    #[serde(default)]
    pub urls: Vec<SelectedUrl>,
}

pub fn collect_search_results(results: &[ScrapedResult]) -> Vec<SearchResult> {
    results
        .iter()
        .flat_map(|item| item.search_results.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::QueryType;
    use std::str::FromStr;

    #[test]
    fn query_type_parser_accepts_all_supported_values() {
        assert!(matches!(QueryType::from_str("phone"), Ok(QueryType::Phone)));
        assert!(matches!(QueryType::from_str("bank"), Ok(QueryType::Bank)));
        assert!(matches!(QueryType::from_str("url"), Ok(QueryType::Url)));
    }

    #[test]
    fn query_type_parser_rejects_invalid_values() {
        assert!(QueryType::from_str("email").is_err());
        assert!(QueryType::from_str("").is_err());
    }
}
