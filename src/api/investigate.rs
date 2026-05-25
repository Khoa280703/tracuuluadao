use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::pipeline::{Investigation, InvestigationEvent, QueryType, run_investigation};

#[derive(Debug, Deserialize)]
pub struct InvestigateParams {
    pub q: String,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReportStreamParams {
    pub investigation_id: String,
}

pub async fn handler(
    Query(params): Query<InvestigateParams>,
    State(state): State<AppState>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<InvestigationEvent>(128);
    let query_type = match params.r#type.as_deref() {
        None => QueryType::Phone,
        Some(value) => value.parse::<QueryType>().map_err(|_| {
            AppError::Config(format!(
                "invalid query type: {value}; supported: phone|bank|url"
            ))
        })?,
    };

    let app_state = Arc::new(state);
    let investigation = Investigation {
        request_id: request_id.clone(),
        query: params.q,
        query_type,
    };
    let report_store = initialize_report_store(&app_state, &investigation).await;
    let cancel_token = CancellationToken::new();

    tracing::info!(
        request_id = %request_id,
        query = %investigation.query,
        query_type = investigation.query_type.as_str(),
        "investigation request accepted"
    );

    tx.send(InvestigationEvent::InvestigationStarted {
        investigation_id: request_id.clone(),
    })
    .await
    .ok();
    let investigation_tx = tx.clone();
    let error_tx = tx.clone();

    tokio::spawn({
        let app_state = app_state.clone();
        let cancel_token = cancel_token.clone();
        let request_id_for_log = investigation.request_id.clone();
        let query_for_log = investigation.query.clone();
        let query_type_for_log = investigation.query_type;
        async move {
            if let Err(error) = run_investigation(
                investigation,
                app_state.registry.clone(),
                app_state.llm.clone(),
                Some(app_state.proxy_pool.clone()),
                app_state.cache.clone(),
                app_state.knowledge_base.clone(),
                report_store.clone(),
                cancel_token,
                investigation_tx.clone(),
            )
            .await
            {
                tracing::error!(
                    request_id = %request_id_for_log,
                    query = %query_for_log,
                    query_type = query_type_for_log.as_str(),
                    error = %error,
                    "investigation request failed"
                );
                if let Some(store) = report_store.as_ref() {
                    let _ = store
                        .mark_failed(
                            &request_id_for_log,
                            &user_facing_investigation_failure_message(&error),
                        )
                        .await;
                }
                let _ = error_tx
                    .send(InvestigationEvent::Error {
                        phase: None,
                        message: user_facing_investigation_failure_message(&error),
                        recoverable: false,
                    })
                    .await;
            }
        }
    });
    tokio::spawn({
        let cancel_token = cancel_token.clone();
        let tx = tx.clone();
        async move {
            tx.closed().await;
            cancel_token.cancel();
        }
    });

    Ok(Sse::new(event_stream(rx)).keep_alive(KeepAlive::default()))
}

pub async fn report_handler(
    Query(params): Query<ReportStreamParams>,
    State(state): State<AppState>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let Some(report_store) = state.report_store.clone() else {
        return Err(AppError::Config(
            "report replay requires REDIS_URL to be configured".to_string(),
        ));
    };

    let (tx, rx) = mpsc::channel::<InvestigationEvent>(128);
    let investigation_id = params.investigation_id;
    tokio::spawn(async move {
        if let Err(error) = report_store
            .stream_report(&investigation_id, &tx, &CancellationToken::new())
            .await
        {
            tracing::warn!(investigation_id = %investigation_id, error = %error, "report stream failed");
            let _ = tx
                .send(InvestigationEvent::Error {
                    phase: Some(5),
                    message: user_facing_report_stream_failure_message(&error),
                    recoverable: false,
                })
                .await;
        }
    });

    Ok(Sse::new(event_stream(rx)).keep_alive(KeepAlive::default()))
}

async fn initialize_report_store(
    state: &Arc<AppState>,
    investigation: &Investigation,
) -> Option<Arc<crate::report_store::InvestigationReportStore>> {
    let store = state.report_store.clone()?;
    if let Err(error) = store
        .initialize_investigation(
            &investigation.request_id,
            &investigation.query,
            investigation.query_type.as_str(),
        )
        .await
    {
        tracing::warn!(
            request_id = %investigation.request_id,
            error = %error,
            "report store init failed; falling back to direct SSE report delivery"
        );
        None
    } else {
        Some(store)
    }
}

fn event_stream(
    rx: mpsc::Receiver<InvestigationEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut rx = ReceiverStream::new(rx);
        while let Some(event) = futures::StreamExt::next(&mut rx).await {
            if matches!(event, InvestigationEvent::Error { recoverable: false, .. }) {
                yield Ok(Event::default()
                    .event(event.event_name())
                    .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string())));
                break;
            }

            yield Ok(Event::default()
                .event(event.event_name())
                .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string())));

            if matches!(event, InvestigationEvent::Complete { .. }) {
                break;
            }
        }
    }
}

fn user_facing_investigation_failure_message(error: &AppError) -> String {
    let raw = error.to_string().to_ascii_lowercase();
    if raw.contains("invalid api key")
        || raw.contains("authentication_error")
        || raw.contains("unauthorized")
        || raw.contains("llm upstream error")
    {
        return "Hệ thống AI đang gặp sự cố nên chưa thể hoàn tất bản nhận định lần này."
            .to_string();
    }

    "Quá trình điều tra đã bị gián đoạn trước khi hoàn tất. Vui lòng thử lại sau ít phút."
        .to_string()
}

fn user_facing_report_stream_failure_message(error: &AppError) -> String {
    let raw = error.to_string().to_ascii_lowercase();
    if raw.contains("unknown investigation id") {
        return "Báo cáo đang được chuẩn bị lại. Vui lòng chờ thêm ít giây rồi mở lại.".to_string();
    }
    if raw.contains("report stream cancelled") {
        return "Báo cáo đang tạm dừng cập nhật. Vui lòng mở lại sau ít giây.".to_string();
    }
    if raw.contains("invalid api key")
        || raw.contains("authentication_error")
        || raw.contains("unauthorized")
        || raw.contains("llm upstream error")
    {
        return "Hệ thống AI đang gặp sự cố nên báo cáo tạm thời chưa thể mở ngay lúc này."
            .to_string();
    }

    "Chưa thể mở báo cáo lúc này. Hệ thống đang thử chuẩn bị lại nội dung.".to_string()
}
