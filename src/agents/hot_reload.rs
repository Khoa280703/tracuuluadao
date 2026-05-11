use std::sync::Arc;
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::agents::config::AgentRegistry;
use crate::error::AppResult;

pub fn start_hot_reload(registry: Arc<AgentRegistry>) -> AppResult<()> {
    let config_dir = registry.config_dir().to_path_buf();
    thread::spawn(move || {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::error!("failed to create watcher: {error}");
                return;
            }
        };

        if let Err(error) = watcher.watch(&config_dir, RecursiveMode::Recursive) {
            tracing::error!("failed to watch config directory: {error}");
            return;
        }

        while let Ok(event) = receiver.recv() {
            if event.is_err() {
                continue;
            }
            thread::sleep(Duration::from_millis(250));
            if let Err(error) = registry.reload() {
                tracing::warn!("agent config reload failed: {error}");
            }
        }
    });

    Ok(())
}
