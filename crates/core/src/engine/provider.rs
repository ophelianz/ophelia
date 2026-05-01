/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! Starts downloads for the engine controller
//!
//! Keeps HTTP task setup and HTTP pause data out of `controller.rs`

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::EngineConfig;
use crate::engine::http::{DownloadTaskRequest, TaskFinalState, TokenBucket, download_task};
use crate::engine::{
    ChunkSnapshot, DownloadSource, DownloadSpec, HttpResumeData, PersistedDownloadSource,
    ProviderResumeData, TaskRuntimeUpdate, TransferControlAction, TransferControlSupport,
    TransferId,
};

pub(super) struct SpawnedTask {
    pub(super) handle: JoinHandle<TaskFinalState>,
    pub(super) pause_sink: TaskPauseSink,
    pub(super) destination_sink: TaskDestinationSink,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum SchedulerKey {
    Hostname(String),
}

pub(super) struct SharedSchedulerRequirement {
    pub(super) key: SchedulerKey,
    pub(super) limit: usize,
}

pub(super) struct ProviderCapabilities {
    pub(super) shared_scheduler: Option<SharedSchedulerRequirement>,
}

pub(super) struct ProviderRuntimeContext {
    pub(super) shared_scheduler_semaphore: Option<Arc<Semaphore>>,
    pub(super) global_throttle: Arc<TokenBucket>,
    pub(super) runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
}

pub(super) enum TaskPauseSink {
    Http(Arc<Mutex<Option<Vec<ChunkSnapshot>>>>),
}

pub(super) enum TaskDestinationSink {
    Http(Arc<Mutex<Option<PathBuf>>>),
}

pub(super) fn capabilities(spec: &DownloadSpec, config: &EngineConfig) -> ProviderCapabilities {
    match &spec.source {
        DownloadSource::Http { url, .. } => ProviderCapabilities {
            shared_scheduler: Some(SharedSchedulerRequirement {
                key: SchedulerKey::Hostname(host_from_url(url)),
                limit: config.http.max_connections_per_server,
            }),
        },
    }
}

pub(super) fn lifecycle_capabilities(spec: &DownloadSpec) -> TransferControlSupport {
    match &spec.source {
        DownloadSource::Http { .. } => TransferControlSupport::all(),
    }
}

pub(super) fn supports_control_action(spec: &DownloadSpec, action: TransferControlAction) -> bool {
    lifecycle_capabilities(spec).supports(action)
}

pub(super) fn shared_scheduler_limit(key: &SchedulerKey, config: &EngineConfig) -> Option<usize> {
    match key {
        SchedulerKey::Hostname(_) => Some(config.http.max_connections_per_server),
    }
}

pub(super) fn spawn_task(
    id: TransferId,
    spec: &DownloadSpec,
    pause_token: CancellationToken,
    resume_data: Option<ProviderResumeData>,
    runtime: ProviderRuntimeContext,
) -> SpawnedTask {
    let ProviderRuntimeContext {
        shared_scheduler_semaphore,
        global_throttle,
        runtime_update_tx,
    } = runtime;
    match &spec.source {
        DownloadSource::Http { url, config } => {
            let pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>> = Arc::new(Mutex::new(None));
            let destination_sink: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
            let resume_from = resume_data
                .as_ref()
                .and_then(ProviderResumeData::as_http)
                .map(|data| data.chunks.clone());
            let shared_scheduler_semaphore = shared_scheduler_semaphore
                .expect("http downloads require a shared scheduler semaphore");
            let handle = tokio::spawn({
                let url_ = url.clone();
                let dest_ = spec.destination.clone();
                let destination_policy_ = spec.destination_policy().clone();
                let cfg_ = config.clone();
                let pt_ = pause_token.clone();
                let ps_ = Arc::clone(&pause_sink);
                let ds_ = Arc::clone(&destination_sink);
                let gt_ = Arc::clone(&global_throttle);
                let ru_ = runtime_update_tx.clone();
                async move {
                    let final_state = download_task(DownloadTaskRequest {
                        id,
                        url: url_,
                        destination: dest_,
                        destination_policy: destination_policy_,
                        config: cfg_,
                        pause_token: pt_,
                        pause_sink: ps_,
                        destination_sink: ds_,
                        resume_from,
                        server_semaphore: shared_scheduler_semaphore,
                        global_throttle: gt_,
                        runtime_update_tx: ru_.clone(),
                    })
                    .await;
                    // Keep final state behind the task's own runtime updates
                    let _ = ru_
                        .send(TaskRuntimeUpdate::Done {
                            id,
                            status: final_state.status,
                            downloaded_bytes: final_state.downloaded_bytes,
                            total_bytes: final_state.total_bytes,
                        })
                        .await;
                    final_state
                }
            });
            SpawnedTask {
                handle,
                pause_sink: TaskPauseSink::Http(pause_sink),
                destination_sink: TaskDestinationSink::Http(destination_sink),
            }
        }
    }
}

pub(super) fn take_resume_data(pause_sink: TaskPauseSink) -> Option<ProviderResumeData> {
    match pause_sink {
        TaskPauseSink::Http(sink) => sink
            .lock()
            .unwrap()
            .take()
            .map(HttpResumeData::new)
            .map(ProviderResumeData::Http),
    }
}

pub(super) fn current_destination(destination_sink: &TaskDestinationSink) -> Option<PathBuf> {
    match destination_sink {
        TaskDestinationSink::Http(sink) => sink.lock().unwrap().clone(),
    }
}

pub(super) fn persisted_source(spec: &DownloadSpec) -> PersistedDownloadSource {
    match &spec.source {
        DownloadSource::Http { url, .. } => PersistedDownloadSource::Http { url: url.clone() },
    }
}

fn host_from_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let after_auth = after_scheme.rsplit('@').next().unwrap_or(after_scheme);
    after_auth
        .split('/')
        .next()
        .unwrap_or(after_auth)
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DestinationPolicyConfig, EngineConfig, HttpEngineConfig};
    use crate::engine::destination::DestinationPolicy;
    use crate::engine::http::HttpDownloadConfig;
    use std::path::PathBuf;

    #[test]
    fn capabilities_report_http_hostname_scheduler_key() {
        let config = EngineConfig {
            http: HttpEngineConfig {
                max_connections_per_server: 6,
                ..HttpEngineConfig::default()
            },
            ..EngineConfig::default()
        };
        let destination = PathBuf::from("/tmp/archive.zip");
        let destination_config = DestinationPolicyConfig::default();
        let spec = DownloadSpec::http(
            "https://user:pass@EXAMPLE.com:443/archive.zip".to_string(),
            destination.clone(),
            DestinationPolicy::for_resolved_destination(&destination_config, &destination),
            HttpDownloadConfig::default(),
        );

        let capabilities = capabilities(&spec, &config);
        let scheduler = capabilities.shared_scheduler.unwrap();
        assert_eq!(scheduler.limit, 6);
        assert_eq!(
            scheduler.key,
            SchedulerKey::Hostname("example.com:443".to_string())
        );
    }

    #[test]
    fn take_resume_data_wraps_http_pause_snapshots() {
        let pause_sink = TaskPauseSink::Http(Arc::new(Mutex::new(Some(vec![ChunkSnapshot {
            start: 0,
            end: 100,
            downloaded: 40,
        }]))));

        let resume_data = take_resume_data(pause_sink).unwrap();
        assert_eq!(resume_data.downloaded_bytes(), 40);
        assert_eq!(resume_data.total_bytes(), Some(100));
    }
}
