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

//! HTTP/HTTPS download task
//!
//! `download_task` probes the server, picks range or single-stream download,
//! prepares the part file, then hands range downloads to the runner
//!
//! Single-stream fallback has no resume data or chunk map yet

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::disk::{DiskHandle, DiskLease};
use crate::engine::chunk;
use crate::engine::destination::{DestinationPolicy, ResolvedDestination, part_path_for};
use crate::engine::http::throttle::{Throttle, TokenBucket};
use crate::engine::http::{HttpDownloadConfig, HttpRangeStrategyConfig};
use crate::engine::types::{
    ChunkSnapshot, ProgressUpdate, TaskRuntimeUpdate, TransferControlSupport, TransferId,
    TransferStatus,
};

use super::config::RangeOrdering;
use super::probe::probe;
use super::range_runner::{ChunkMapSupport, RangeDownloadConfig, run_range_download};
use super::resume::RangeResumeSnapshot;
use super::single::{SingleTransferRequest, single_download};

const RANGE_WORK_UNIT_SIZE: u64 = 2 * 1024 * 1024;
const STEAL_ALIGN: u64 = 4096;

#[derive(Debug, Clone, Copy)]
pub struct TaskFinalState {
    pub status: TransferStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

pub struct DownloadTaskRequest {
    pub id: TransferId,
    pub url: String,
    pub destination: PathBuf,
    pub destination_policy: DestinationPolicy,
    pub config: HttpDownloadConfig,
    pub pause_token: CancellationToken,
    pub pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>>,
    pub destination_sink: Arc<Mutex<Option<PathBuf>>>,
    pub resume_from: Option<Vec<ChunkSnapshot>>,
    pub server_semaphore: Arc<Semaphore>,
    pub global_throttle: Arc<TokenBucket>,
    pub(crate) disk: DiskHandle,
    pub runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
}

impl DownloadTaskRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: TransferId,
        url: String,
        destination: PathBuf,
        destination_policy: DestinationPolicy,
        config: HttpDownloadConfig,
        pause_token: CancellationToken,
        pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>>,
        destination_sink: Arc<Mutex<Option<PathBuf>>>,
        resume_from: Option<Vec<ChunkSnapshot>>,
        server_semaphore: Arc<Semaphore>,
        global_throttle: Arc<TokenBucket>,
        runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
    ) -> Self {
        Self {
            id,
            url,
            destination,
            destination_policy,
            config,
            pause_token,
            pause_sink,
            destination_sink,
            resume_from,
            server_semaphore,
            global_throttle,
            disk: DiskHandle::new(),
            runtime_update_tx,
        }
    }
}

fn task_state(
    status: TransferStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> TaskFinalState {
    TaskFinalState {
        status,
        downloaded_bytes,
        total_bytes,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds the file handle and range work
/// Returns a final state when the task ends early
struct ResolvedChunks {
    total_bytes: u64,
    chunks: chunk::ChunkList,
    disk: DiskLease,
    chunk_map_support: ChunkMapSupport,
    ordering: RangeOrdering,
}

struct RangeDownloadPlan {
    ordering: RangeOrdering,
    connection_limit: usize,
    chunks: chunk::ChunkList,
    strategies: HttpRangeStrategyConfig,
}

struct ResolveChunksRequest<'a> {
    resume_from: Option<Vec<ChunkSnapshot>>,
    probe_client: &'a reqwest::Client,
    chunk_client: &'a Arc<reqwest::Client>,
    url: &'a str,
    destination: PathBuf,
    destination_policy: &'a DestinationPolicy,
    config: &'a HttpDownloadConfig,
    id: TransferId,
    disk: DiskHandle,
    runtime_update_tx: &'a mpsc::Sender<TaskRuntimeUpdate>,
    destination_sink: &'a Arc<Mutex<Option<PathBuf>>>,
    pause_token: &'a CancellationToken,
    throttle: Arc<Throttle>,
}

async fn send_progress_update(
    runtime_update_tx: &mpsc::Sender<TaskRuntimeUpdate>,
    id: TransferId,
    status: TransferStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    speed_bytes_per_sec: u64,
) {
    let _ = runtime_update_tx
        .send(TaskRuntimeUpdate::Progress(ProgressUpdate {
            id,
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec,
        }))
        .await;
}

async fn resolve_chunks(
    request: ResolveChunksRequest<'_>,
) -> Result<ResolvedChunks, TaskFinalState> {
    let ResolveChunksRequest {
        resume_from,
        probe_client,
        chunk_client,
        url,
        destination,
        destination_policy,
        config,
        id,
        disk,
        runtime_update_tx,
        destination_sink,
        pause_token,
        throttle,
    } = request;

    match resume_from {
        Some(snapshots) => {
            let Some(snapshot) = RangeResumeSnapshot::from_old_chunks(&snapshots) else {
                send_progress_update(runtime_update_tx, id, TransferStatus::Error, 0, None, 0)
                    .await;
                return Err(task_state(TransferStatus::Error, 0, None));
            };
            let total = snapshot.total_bytes();
            let downloaded = snapshot.downloaded_bytes();

            let probe_result = match probe(probe_client, url).await {
                Ok(result) => result,
                Err(_) => {
                    send_progress_update(
                        runtime_update_tx,
                        id,
                        TransferStatus::Error,
                        downloaded,
                        Some(total),
                        0,
                    )
                    .await;
                    return Err(task_state(TransferStatus::Error, downloaded, Some(total)));
                }
            };
            if !probe_result.accepts_ranges || probe_result.content_length != Some(total) {
                tracing::warn!(
                    expected_total = total,
                    probed_total = probe_result.content_length,
                    accepts_ranges = probe_result.accepts_ranges,
                    "resume data no longer matches remote range response"
                );
                send_progress_update(
                    runtime_update_tx,
                    id,
                    TransferStatus::Error,
                    downloaded,
                    Some(total),
                    0,
                )
                .await;
                return Err(task_state(TransferStatus::Error, downloaded, Some(total)));
            }

            let snapshots = snapshot.chunk_snapshots();
            let cl = chunk_list_from_snapshots(&snapshots);
            let part_path = part_path_for(&destination);
            let disk = match disk.resume_existing(
                id,
                ResolvedDestination {
                    part_path,
                    destination: destination.clone(),
                    finalize_strategy: destination_policy.finalize_strategy(),
                },
                Some(total),
                downloaded,
            ) {
                Ok(disk) => disk,
                Err(_) => {
                    send_progress_update(
                        runtime_update_tx,
                        id,
                        TransferStatus::Error,
                        downloaded,
                        Some(total),
                        0,
                    )
                    .await;
                    return Err(task_state(TransferStatus::Error, downloaded, Some(total)));
                }
            };
            *destination_sink.lock().unwrap() = Some(destination.clone());
            let ordering = config.resolved_ordering_for_destination(&destination);
            tracing::info!(
                total_bytes = total,
                chunks = cl.len(),
                "resuming chunked download"
            );
            Ok(ResolvedChunks {
                total_bytes: total,
                chunks: cl,
                disk,
                chunk_map_support: ChunkMapSupport::Supported,
                ordering,
            })
        }
        None => {
            let initial_destination = destination;
            let probe_result = match probe(probe_client, url).await {
                Ok(p) => p,
                Err(_) => {
                    send_progress_update(runtime_update_tx, id, TransferStatus::Error, 0, None, 0)
                        .await;
                    return Err(task_state(TransferStatus::Error, 0, None));
                }
            };
            tracing::debug!(
                accepts_ranges = probe_result.accepts_ranges,
                content_length = probe_result.content_length,
                filename = probe_result.filename.as_deref(),
                "probe complete"
            );

            let resolved_destination =
                match destination_policy.resolve_checked(url, probe_result.filename.as_deref()) {
                    Ok(resolved) => resolved,
                    Err(_) => {
                        send_progress_update(
                            runtime_update_tx,
                            id,
                            TransferStatus::Error,
                            0,
                            probe_result.content_length,
                            0,
                        )
                        .await;
                        return Err(task_state(
                            TransferStatus::Error,
                            0,
                            probe_result.content_length,
                        ));
                    }
                };
            let ResolvedDestination {
                part_path,
                destination,
                finalize_strategy,
            } = resolved_destination;
            *destination_sink.lock().unwrap() = Some(destination.clone());
            if destination != initial_destination {
                let _ = runtime_update_tx
                    .send(TaskRuntimeUpdate::DestinationChanged {
                        id,
                        destination: destination.clone(),
                    })
                    .await;
            }
            let ordering = config.resolved_ordering_for_destination(&destination);

            if !probe_result.accepts_ranges || probe_result.content_length.is_none() {
                tracing::info!(
                    accepts_ranges = probe_result.accepts_ranges,
                    has_content_length = probe_result.content_length.is_some(),
                    "falling back to single stream"
                );
                let _ = runtime_update_tx
                    .send(TaskRuntimeUpdate::ControlSupportChanged {
                        id,
                        support: single_stream_control_support(),
                    })
                    .await;
                let _ = runtime_update_tx
                    .send(TaskRuntimeUpdate::ChunkMapChanged {
                        id,
                        state: crate::engine::TransferChunkMapState::Unsupported,
                    })
                    .await;
                return Err(single_download(SingleTransferRequest {
                    id,
                    client: Arc::clone(chunk_client),
                    url: url.to_owned(),
                    disk: match disk.create_new(
                        id,
                        ResolvedDestination {
                            part_path,
                            destination,
                            finalize_strategy,
                        },
                        None,
                    ) {
                        Ok(disk) => disk,
                        Err(_) => {
                            send_progress_update(
                                runtime_update_tx,
                                id,
                                TransferStatus::Error,
                                0,
                                probe_result.content_length,
                                0,
                            )
                            .await;
                            return Err(task_state(
                                TransferStatus::Error,
                                0,
                                probe_result.content_length,
                            ));
                        }
                    },
                    stall_timeout_secs: config.stall_timeout_secs,
                    runtime_update_tx: runtime_update_tx.clone(),
                    pause_token: pause_token.clone(),
                    throttle,
                })
                .await);
            }

            let total_bytes = probe_result
                .content_length
                .expect("content length checked before chunked download");
            let part_path_for_log = part_path.clone();

            let disk = match disk.create_new(
                id,
                ResolvedDestination {
                    part_path,
                    destination,
                    finalize_strategy,
                },
                Some(total_bytes),
            ) {
                Ok(disk) => disk,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tracing::warn!(
                        path = %part_path_for_log.display(),
                        "part file already exists, another download may be active"
                    );
                    send_progress_update(
                        runtime_update_tx,
                        id,
                        TransferStatus::Error,
                        0,
                        Some(total_bytes),
                        0,
                    )
                    .await;
                    return Err(task_state(TransferStatus::Error, 0, Some(total_bytes)));
                }
                Err(error) => {
                    tracing::error!(
                        ?error,
                        path = %part_path_for_log.display(),
                        "failed to prepare part file"
                    );
                    send_progress_update(
                        runtime_update_tx,
                        id,
                        TransferStatus::Error,
                        0,
                        Some(total_bytes),
                        0,
                    )
                    .await;
                    return Err(task_state(TransferStatus::Error, 0, Some(total_bytes)));
                }
            };

            let chunks = planned_chunks(total_bytes, ordering, config);
            tracing::info!(
                total_bytes,
                num_chunks = chunks.len(),
                ?ordering,
                "starting chunked download"
            );
            Ok(ResolvedChunks {
                total_bytes,
                chunks,
                disk,
                chunk_map_support: ChunkMapSupport::from_supported(probe_result.accepts_ranges),
                ordering,
            })
        }
    }
}

fn planned_chunks(
    total_bytes: u64,
    ordering: RangeOrdering,
    config: &HttpDownloadConfig,
) -> chunk::ChunkList {
    let chunks = chunk::split_by_size(total_bytes, RANGE_WORK_UNIT_SIZE);
    if ordering == RangeOrdering::Sequential {
        return chunks;
    }

    let min_chunks = config.min_connections.max(1);
    if chunks.len() >= min_chunks || total_bytes == 0 {
        chunks
    } else {
        chunk::split(total_bytes, min_chunks)
    }
}

fn range_download_plan(
    ordering: RangeOrdering,
    chunks: chunk::ChunkList,
    config: &HttpDownloadConfig,
) -> RangeDownloadPlan {
    let strategies = match ordering {
        RangeOrdering::Sequential => HttpRangeStrategyConfig::default(),
        RangeOrdering::Balanced => config.range_strategies,
    };
    let connection_limit = match ordering {
        RangeOrdering::Sequential => 1,
        RangeOrdering::Balanced => chunks.len().max(1).min(config.max_connections.max(1)),
    };

    RangeDownloadPlan {
        ordering,
        connection_limit,
        chunks,
        strategies,
    }
}

fn chunk_list_from_snapshots(snapshots: &[ChunkSnapshot]) -> chunk::ChunkList {
    chunk::ChunkList {
        starts: snapshots.iter().map(|s| s.start).collect(),
        ends: snapshots.iter().map(|s| s.end).collect(),
        downloaded: snapshots.iter().map(|s| s.downloaded).collect(),
        statuses: snapshots
            .iter()
            .map(|s| {
                let len = s.end.saturating_sub(s.start);
                if s.downloaded >= len {
                    chunk::ChunkStatus::Finished
                } else {
                    chunk::ChunkStatus::Pending
                }
            })
            .collect(),
    }
}

/// Starts or resumes one HTTP download
/// On pause, range downloads write chunks into `pause_sink` so the engine can save them
#[tracing::instrument(
    name = "download",
    skip_all,
    fields(id = request.id.0, url = %request.url)
)]
pub async fn download_task(request: DownloadTaskRequest) -> TaskFinalState {
    let DownloadTaskRequest {
        id,
        url,
        destination,
        destination_policy,
        config,
        pause_token,
        pause_sink,
        destination_sink,
        resume_from,
        server_semaphore,
        global_throttle,
        disk,
        runtime_update_tx,
    } = request;

    // Probe uses a normal client
    // Range workers use HTTP/1.1 so each request can get its own connection
    let probe_client = reqwest::Client::new();
    let chunk_client = Arc::new(
        reqwest::Client::builder()
            .http1_only()
            .build()
            .expect("failed to build HTTP/1.1 client"),
    );

    // Build the speed limiter pair
    let throttle = Arc::new(Throttle {
        per_download: Arc::new(TokenBucket::new(config.speed_limit_bps)),
        global: Arc::clone(&global_throttle),
    });

    // Find the path, file handle, and ranges
    //
    // This may (?) end early on probe failure or single-stream fallback
    let resolved = match resolve_chunks(ResolveChunksRequest {
        resume_from,
        probe_client: &probe_client,
        chunk_client: &chunk_client,
        url: &url,
        destination,
        destination_policy: &destination_policy,
        config: &config,
        id,
        disk,
        runtime_update_tx: &runtime_update_tx,
        destination_sink: &destination_sink,
        pause_token: &pause_token,
        throttle: Arc::clone(&throttle),
    })
    .await
    {
        Ok(v) => v,
        Err(final_state) => return final_state,
    };
    let ResolvedChunks {
        total_bytes,
        chunks,
        disk,
        chunk_map_support,
        ordering,
    } = resolved;
    let plan = range_download_plan(ordering, chunks, &config);

    // Pull config values into locals before moving config into the runner
    let write_buffer_size = config.write_buffer_size;
    let stall_timeout = Duration::from_secs(config.stall_timeout_secs);
    let progress_interval = Duration::from_millis(config.progress_interval_ms);
    let max_retries = config.max_retries_per_chunk;
    let min_steal_bytes = config.min_steal_bytes;
    let RangeDownloadPlan {
        ordering,
        connection_limit,
        chunks,
        strategies,
    } = plan;
    tracing::debug!(
        ?ordering,
        connection_limit,
        ?strategies,
        "range download plan resolved"
    );

    run_range_download(RangeDownloadConfig {
        id,
        url,
        client: Arc::clone(&chunk_client),
        disk,
        chunks,
        total_bytes,
        chunk_map_support,
        connection_limit,
        write_buffer_size,
        stall_timeout,
        progress_interval,
        max_retries,
        strategies,
        safe_zone: write_buffer_size as u64,
        min_steal_bytes,
        steal_align: STEAL_ALIGN,
        pause_token,
        pause_sink,
        server_semaphore,
        throttle,
        runtime_update_tx,
    })
    .await
}

fn single_stream_control_support() -> TransferControlSupport {
    TransferControlSupport {
        can_pause: false,
        can_resume: false,
        can_cancel: true,
        can_restore: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mib(value: u64) -> u64 {
        value * 1024 * 1024
    }

    #[test]
    fn balanced_plan_defaults_to_small_work_units_and_live_strategies() {
        let config = HttpDownloadConfig::default();
        let chunks = planned_chunks(mib(5) + 1, RangeOrdering::Balanced, &config);
        let plan = range_download_plan(RangeOrdering::Balanced, chunks, &config);

        assert_eq!(plan.ordering, RangeOrdering::Balanced);
        assert_eq!(plan.chunks.len(), 3);
        assert_eq!(plan.connection_limit, 3);
        assert_eq!(plan.strategies, HttpRangeStrategyConfig::live_balancer());
    }

    #[test]
    fn balanced_plan_preserves_min_connections_for_small_files() {
        let config = HttpDownloadConfig {
            min_connections: 4,
            ..HttpDownloadConfig::default()
        };
        let chunks = planned_chunks(128 * 1024, RangeOrdering::Balanced, &config);
        let plan = range_download_plan(RangeOrdering::Balanced, chunks, &config);

        assert_eq!(plan.chunks.len(), 4);
        assert_eq!(plan.connection_limit, 4);
    }

    #[test]
    fn sequential_plan_uses_one_connection_and_no_live_strategies() {
        let config = HttpDownloadConfig::default();
        let chunks = planned_chunks(mib(5) + 1, RangeOrdering::Sequential, &config);
        let plan = range_download_plan(RangeOrdering::Sequential, chunks, &config);

        assert_eq!(plan.ordering, RangeOrdering::Sequential);
        assert_eq!(plan.chunks.len(), 3);
        assert_eq!(plan.connection_limit, 1);
        assert_eq!(plan.strategies, HttpRangeStrategyConfig::default());
    }

    #[test]
    fn sequential_plan_ignores_requested_live_strategies() {
        let config = HttpDownloadConfig {
            range_strategies: HttpRangeStrategyConfig::live_balancer(),
            ..HttpDownloadConfig::default()
        };
        let chunks = planned_chunks(mib(5), RangeOrdering::Sequential, &config);
        let plan = range_download_plan(RangeOrdering::Sequential, chunks, &config);

        assert_eq!(plan.strategies, HttpRangeStrategyConfig::default());
    }

    #[test]
    fn balanced_plan_respects_live_strategies_being_turned_off() {
        let config = HttpDownloadConfig {
            range_strategies: HttpRangeStrategyConfig::default(),
            ..HttpDownloadConfig::default()
        };
        let chunks = planned_chunks(mib(5), RangeOrdering::Balanced, &config);
        let plan = range_download_plan(RangeOrdering::Balanced, chunks, &config);

        assert_eq!(plan.strategies, HttpRangeStrategyConfig::default());
    }
}
