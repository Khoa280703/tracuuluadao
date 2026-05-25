use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::error::AppResult;
use crate::knowledge_base::{
    EvidenceInput, HistoricalContextSnapshot, KnowledgeBase, SubjectHistory, compute_quality_score,
    risk_level_to_numeric,
};
use crate::pipeline::state::{Investigation, InvestigationEvent, InvestigationResult};

pub async fn load_subject_history(
    knowledge_base: Option<&Arc<KnowledgeBase>>,
    investigation: &Investigation,
    tx: &mpsc::Sender<InvestigationEvent>,
) -> Option<SubjectHistory> {
    let knowledge_base = knowledge_base?;
    let subject = match knowledge_base
        .get_subject_by_value(&investigation.query, investigation.query_type.as_str())
        .await
    {
        Ok(subject) => subject,
        Err(error) => {
            tracing::warn!("knowledge base subject lookup failed: {error}");
            return None;
        }
    }?;
    let history = match knowledge_base.get_subject_history(subject.id, 10).await {
        Ok(history) => history,
        Err(error) => {
            tracing::warn!("knowledge base history lookup failed: {error}");
            return None;
        }
    };
    let snapshot = KnowledgeBase::build_historical_snapshot(&history);
    tx.send(InvestigationEvent::HistoricalContext {
        investigation_count: snapshot.investigation_count,
        last_risk_level: snapshot.last_risk_level,
        approved_reports: snapshot.approved_reports,
        linked_subjects_count: snapshot.linked_subjects_count,
        first_seen: snapshot.first_seen,
        last_seen: snapshot.last_seen,
    })
    .await
    .ok();

    Some(history)
}

pub fn detective_history_payload(history: Option<&SubjectHistory>) -> Option<serde_json::Value> {
    history.map(|history| {
        let snapshot = KnowledgeBase::build_historical_snapshot(history);
        serde_json::json!({
            "summary": HistoricalContextSnapshot {
                investigation_count: snapshot.investigation_count,
                last_risk_level: snapshot.last_risk_level,
                approved_reports: snapshot.approved_reports,
                linked_subjects_count: snapshot.linked_subjects_count,
                first_seen: snapshot.first_seen,
                last_seen: snapshot.last_seen,
            },
            "approved_reports": history.approved_reports.iter().take(5).map(|report| {
                serde_json::json!({
                    "description": report.description,
                    "category": report.category,
                    "created_at": report.created_at,
                })
            }).collect::<Vec<_>>(),
            "linked_subjects": history.linked_subjects.iter().take(8).map(|subject| {
                serde_json::json!({
                    "value": subject.value,
                    "subject_type": subject.subject_type,
                    "risk_level": subject.risk_level,
                    "strength": subject.strength,
                })
            }).collect::<Vec<_>>(),
            "risk_signals": history.all_risk_signals.iter().take(12).cloned().collect::<Vec<_>>(),
        })
    })
}

pub fn schedule_ingest(knowledge_base: Option<&Arc<KnowledgeBase>>, result: &InvestigationResult) {
    let Some(knowledge_base) = knowledge_base else {
        return;
    };
    let knowledge_base = knowledge_base.clone();
    let result = result.clone();
    tokio::spawn(async move {
        if let Err(error) = ingest_investigation(knowledge_base, result).await {
            tracing::warn!("knowledge base ingest failed: {error}");
        }
    });
}

async fn ingest_investigation(
    knowledge_base: Arc<KnowledgeBase>,
    result: InvestigationResult,
) -> AppResult<()> {
    let successful_sources = result
        .scraped_results
        .iter()
        .filter(|item| item.success)
        .count();
    let total_sources = result.scraped_results.len();
    let sources_success_ratio = if total_sources == 0 {
        0.0
    } else {
        successful_sources as f32 / total_sources as f32
    };
    let risk_signals_count = result
        .summaries
        .iter()
        .map(|item| item.risk_signals.len())
        .sum::<usize>()
        + result
            .extractions
            .iter()
            .map(|item| item.risk_signals.len())
            .sum::<usize>();
    let evidence_urls_count = result
        .summaries
        .iter()
        .map(|item| item.evidence_urls.len())
        .sum::<usize>();
    let quality_score = compute_quality_score(
        sources_success_ratio,
        result.confidence,
        risk_signals_count,
        evidence_urls_count,
    );
    if quality_score < 0.3 {
        return Ok(());
    }

    let subject_id = knowledge_base
        .upsert_subject(&result.query, result.query_type.as_str())
        .await?;
    let investigation_id = knowledge_base
        .insert_investigation(
            subject_id,
            &result.query,
            result.query_type.as_str(),
            &result.risk_level,
            risk_level_to_numeric(&result.risk_level),
            result.confidence,
            result.sources_analyzed as i32,
            sources_success_ratio,
            quality_score,
            result.duration_ms as i64,
            Some(&result.detective_markdown),
        )
        .await?;

    let evidence_inputs = build_evidence_inputs(subject_id, investigation_id, &result)?;
    knowledge_base
        .insert_evidence_batch(evidence_inputs)
        .await?;
    detect_and_create_links(&knowledge_base, subject_id, &result).await?;
    knowledge_base.recalculate_risk(subject_id).await?;
    Ok(())
}

fn build_evidence_inputs(
    subject_id: uuid::Uuid,
    investigation_id: uuid::Uuid,
    result: &InvestigationResult,
) -> AppResult<Vec<EvidenceInput>> {
    let mut items = Vec::new();
    for summary in &result.summaries {
        items.push(EvidenceInput {
            subject_id,
            investigation_id: Some(investigation_id),
            source: summary.source.clone(),
            evidence_type: "agent_summary".to_string(),
            data: serde_json::to_value(summary)?,
            risk_signals: summary.risk_signals.clone(),
            mentioned_subjects: summary.phone_mentions.clone(),
        });
    }
    for extraction in &result.extractions {
        items.push(EvidenceInput {
            subject_id,
            investigation_id: Some(investigation_id),
            source: extraction.url.clone(),
            evidence_type: "agent_extraction".to_string(),
            data: serde_json::to_value(extraction)?,
            risk_signals: extraction.risk_signals.clone(),
            mentioned_subjects: extraction.related_numbers.clone(),
        });
    }
    Ok(items)
}

async fn detect_and_create_links(
    knowledge_base: &KnowledgeBase,
    source_subject_id: uuid::Uuid,
    result: &InvestigationResult,
) -> AppResult<()> {
    let source_value = canonicalize_subject_value(&result.query, result.query_type.as_str());
    let mut mentioned_values = HashSet::<(String, String)>::new();
    for summary in &result.summaries {
        for value in &summary.phone_mentions {
            push_link_candidate(&mut mentioned_values, value, &source_value);
        }
    }
    for extraction in &result.extractions {
        for value in &extraction.related_numbers {
            push_link_candidate(&mut mentioned_values, value, &source_value);
        }
    }

    for (value, subject_type) in mentioned_values {
        let linked_id = knowledge_base.upsert_subject(&value, &subject_type).await?;
        if linked_id != source_subject_id {
            knowledge_base
                .upsert_subject_link(source_subject_id, linked_id, "mentioned_together", None)
                .await?;
        }
    }
    Ok(())
}

fn push_link_candidate(
    mentioned_values: &mut HashSet<(String, String)>,
    value: &str,
    source_value: &str,
) {
    let Some(subject_type) = classify_value(value) else {
        return;
    };
    let normalized = canonicalize_subject_value(value, subject_type);
    if normalized.is_empty() || normalized == source_value {
        return;
    }
    mentioned_values.insert((normalized, subject_type.to_string()));
}

fn classify_value(value: &str) -> Option<&'static str> {
    if value.starts_with("http://") || value.starts_with("https://") {
        return Some("url");
    }
    let digits = value
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if matches!(digits.len(), 10 | 11 | 12) && (digits.starts_with('0') || digits.starts_with("84"))
    {
        return Some("phone");
    }
    if (8..=20).contains(&digits.len()) {
        return Some("bank");
    }
    None
}

fn canonicalize_subject_value(value: &str, subject_type: &str) -> String {
    match subject_type {
        "phone" => value.chars().filter(|ch| ch.is_ascii_digit()).collect(),
        "bank" => value.split_whitespace().collect::<String>(),
        "url" => value.trim().trim_end_matches('/').to_ascii_lowercase(),
        _ => value.trim().to_string(),
    }
}
