use std::collections::HashMap;

use redis::streams::{StreamId, StreamRangeReply, StreamReadOptions, StreamReadReply};
use redis::{AsyncCommands, Client, FromRedisValue, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::pipeline::InvestigationEvent;

const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETE: &str = "complete";
const STATUS_FAILED: &str = "failed";

#[derive(Debug, Clone)]
pub struct InvestigationReportStore {
    client: Client,
    ttl_secs: u64,
}

impl InvestigationReportStore {
    pub fn new(redis_url: &str, ttl_secs: u64) -> AppResult<Self> {
        Ok(Self {
            client: Client::open(redis_url)?,
            ttl_secs,
        })
    }

    pub async fn initialize_investigation(
        &self,
        investigation_id: &str,
        query: &str,
        query_type: &str,
    ) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let stream_key = stream_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        redis::pipe()
            .cmd("DEL")
            .arg(&stream_key)
            .ignore()
            .cmd("HSET")
            .arg(&meta_key)
            .arg("status")
            .arg(STATUS_RUNNING)
            .arg("query")
            .arg(query)
            .arg("query_type")
            .arg(query_type)
            .ignore()
            .cmd("EXPIRE")
            .arg(&meta_key)
            .arg(self.ttl_secs)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn append_report_chunk(&self, investigation_id: &str, chunk: &str) -> AppResult<()> {
        self.append_event(investigation_id, chunk, false, false)
            .await
    }

    pub async fn finalize_report_stream(&self, investigation_id: &str) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let stream_key = stream_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        redis::pipe()
            .cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("chunk")
            .arg("")
            .arg("done")
            .arg(1)
            .arg("replace")
            .arg(0)
            .ignore()
            .cmd("HSET")
            .arg(&meta_key)
            .arg("status")
            .arg(STATUS_COMPLETE)
            .ignore()
            .cmd("EXPIRE")
            .arg(&meta_key)
            .arg(self.ttl_secs)
            .ignore()
            .cmd("EXPIRE")
            .arg(&stream_key)
            .arg(self.ttl_secs)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn store_replacement_report(
        &self,
        investigation_id: &str,
        markdown: &str,
    ) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let stream_key = stream_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        redis::pipe()
            .cmd("DEL")
            .arg(&stream_key)
            .ignore()
            .cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("chunk")
            .arg(markdown)
            .arg("done")
            .arg(0)
            .arg("replace")
            .arg(1)
            .ignore()
            .cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("chunk")
            .arg("")
            .arg("done")
            .arg(1)
            .arg("replace")
            .arg(0)
            .ignore()
            .cmd("HSET")
            .arg(&meta_key)
            .arg("status")
            .arg(STATUS_COMPLETE)
            .ignore()
            .cmd("EXPIRE")
            .arg(&meta_key)
            .arg(self.ttl_secs)
            .ignore()
            .cmd("EXPIRE")
            .arg(&stream_key)
            .arg(self.ttl_secs)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn mark_failed(&self, investigation_id: &str, message: &str) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        redis::pipe()
            .cmd("HSET")
            .arg(&meta_key)
            .arg("status")
            .arg(STATUS_FAILED)
            .arg("error")
            .arg(message)
            .ignore()
            .cmd("EXPIRE")
            .arg(&meta_key)
            .arg(self.ttl_secs)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn stream_report(
        &self,
        investigation_id: &str,
        tx: &mpsc::Sender<InvestigationEvent>,
        cancel_token: &CancellationToken,
    ) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let stream_key = stream_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        let exists: bool = conn.exists(&meta_key).await?;
        if !exists {
            return Err(AppError::Server(format!(
                "unknown investigation id: {investigation_id}"
            )));
        }

        let backlog: StreamRangeReply = redis::cmd("XRANGE")
            .arg(&stream_key)
            .arg("-")
            .arg("+")
            .query_async(&mut conn)
            .await
            .unwrap_or(StreamRangeReply { ids: Vec::new() });

        let mut last_id = "$".to_string();
        for entry in backlog.ids {
            last_id = entry.id.clone();
            if emit_stream_entry(tx, &entry).await? {
                return Ok(());
            }
        }

        loop {
            if cancel_token.is_cancelled() {
                return Err(AppError::Server("report stream cancelled".to_string()));
            }

            let status: Option<String> = conn.hget(&meta_key, "status").await.ok();
            if matches!(status.as_deref(), Some(STATUS_FAILED)) {
                let error: Option<String> = conn.hget(&meta_key, "error").await.ok();
                return Err(AppError::Server(
                    error.unwrap_or_else(|| "investigation report failed".to_string()),
                ));
            }

            let options = StreamReadOptions::default().block(30_000).count(100);
            let reply: StreamReadReply = conn
                .xread_options(&[stream_key.as_str()], &[last_id.as_str()], &options)
                .await
                .unwrap_or(StreamReadReply { keys: Vec::new() });

            if reply.keys.is_empty() {
                if matches!(status.as_deref(), Some(STATUS_COMPLETE)) {
                    return Ok(());
                }
                continue;
            }

            for key in reply.keys {
                for entry in key.ids {
                    last_id = entry.id.clone();
                    if emit_stream_entry(tx, &entry).await? {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn append_event(
        &self,
        investigation_id: &str,
        chunk: &str,
        done: bool,
        replace: bool,
    ) -> AppResult<()> {
        let meta_key = meta_key(investigation_id);
        let stream_key = stream_key(investigation_id);
        let mut conn = self.client.get_multiplexed_tokio_connection().await?;
        redis::pipe()
            .cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("chunk")
            .arg(chunk)
            .arg("done")
            .arg(if done { 1 } else { 0 })
            .arg("replace")
            .arg(if replace { 1 } else { 0 })
            .ignore()
            .cmd("HSET")
            .arg(&meta_key)
            .arg("status")
            .arg(STATUS_RUNNING)
            .ignore()
            .cmd("EXPIRE")
            .arg(&meta_key)
            .arg(self.ttl_secs)
            .ignore()
            .cmd("EXPIRE")
            .arg(&stream_key)
            .arg(self.ttl_secs)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }
}

async fn emit_stream_entry(
    tx: &mpsc::Sender<InvestigationEvent>,
    entry: &StreamId,
) -> AppResult<bool> {
    let chunk = string_field(&entry.map, "chunk");
    let done = bool_field(&entry.map, "done");
    let replace = bool_field(&entry.map, "replace");
    tx.send(InvestigationEvent::DetectiveStream {
        chunk,
        done,
        replace,
    })
    .await
    .ok();
    Ok(done)
}

fn string_field(map: &HashMap<String, Value>, key: &str) -> String {
    map.get(key)
        .and_then(|value| String::from_redis_value(value).ok())
        .unwrap_or_default()
}

fn bool_field(map: &HashMap<String, Value>, key: &str) -> bool {
    map.get(key)
        .and_then(|value| i64::from_redis_value(value).ok())
        .is_some_and(|value| value != 0)
}

fn meta_key(investigation_id: &str) -> String {
    format!("investigation:{investigation_id}:meta")
}

fn stream_key(investigation_id: &str) -> String {
    format!("investigation:{investigation_id}:report")
}
