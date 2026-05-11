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

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::pipeline::{Investigation, InvestigationEvent, QueryType, run_investigation};

#[derive(Debug, Deserialize)]
pub struct InvestigateParams {
    pub q: String,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub async fn handler(
    Query(params): Query<InvestigateParams>,
    State(state): State<AppState>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let (tx, rx) = mpsc::channel::<InvestigationEvent>(128);
    let cancel_token = CancellationToken::new();
    let cancel_for_task = cancel_token.clone();
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
        query: params.q,
        query_type,
    };

    tokio::spawn({
        let app_state = app_state.clone();
        async move {
            if let Err(error) = run_investigation(
                investigation,
                app_state.registry.clone(),
                app_state.llm.clone(),
                Some(app_state.proxy_pool.clone()),
                app_state.cache.clone(),
                cancel_for_task.clone(),
                tx.clone(),
            )
            .await
            {
                let _ = tx
                    .send(InvestigationEvent::Error {
                        phase: None,
                        message: error.to_string(),
                        recoverable: false,
                    })
                    .await;
            }
        }
    });

    let stream_guard = CancelOnDrop(cancel_token);
    let stream = async_stream::stream! {
        let _guard = stream_guard;
        let mut rx = ReceiverStream::new(rx);
        while let Some(event) = futures::StreamExt::next(&mut rx).await {
            yield Ok(Event::default()
                .event(event.event_name())
                .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string())));
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
