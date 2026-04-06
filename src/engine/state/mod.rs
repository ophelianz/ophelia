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

/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

mod db;
mod http;
use db::Db;
pub use db::HistoryReader;

use crate::engine::types::{DbEvent, SavedDownload};

pub struct DbWorkerHandle {
    #[allow(dead_code)] // kept to tie worker lifetime to the owner in non-test builds
    join: Option<std::thread::JoinHandle<()>>,
}

impl DbWorkerHandle {
    fn new(join: std::thread::JoinHandle<()>) -> Self {
        Self { join: Some(join) }
    }

    #[cfg(test)]
    fn join(mut self) {
        if let Some(join) = self.join.take() {
            join.join().expect("db worker thread panicked");
        }
    }
}

pub struct StateBootstrap {
    pub db_tx: std::sync::mpsc::Sender<DbEvent>,
    pub history_reader: HistoryReader,
    pub saved_downloads: Vec<SavedDownload>,
    pub next_download_id: u64,
    #[allow(dead_code)] // kept alive for worker lifetime
    worker: DbWorkerHandle,
}

pub fn bootstrap() -> rusqlite::Result<StateBootstrap> {
    let db = Db::open()?;
    if let Err(error) = db.normalize_stale() {
        tracing::warn!("normalize stale: {error}");
    }
    if let Err(error) = db.validate_integrity() {
        tracing::warn!("integrity check: {error}");
    }

    let (saved_downloads, max_id) = db.load_for_restore()?;
    let (db_tx, db_rx) = std::sync::mpsc::channel::<DbEvent>();
    let worker = spawn_worker(db, db_rx);

    Ok(StateBootstrap {
        db_tx,
        history_reader: HistoryReader::open()?,
        saved_downloads,
        next_download_id: max_id + 1,
        worker,
    })
}

fn spawn_worker(db: Db, rx: std::sync::mpsc::Receiver<DbEvent>) -> DbWorkerHandle {
    let join = std::thread::Builder::new()
        .name("db-worker".into())
        .spawn(move || {
            for event in rx {
                if let Err(e) = db.handle(event) {
                    tracing::error!("db worker: {e}");
                }
            }
        })
        .expect("failed to spawn db worker thread");
    DbWorkerHandle::new(join)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::{
        ChunkSnapshot, DownloadId, DownloadStatus, HttpResumeData, PersistedDownloadSource,
        ProviderResumeData,
    };
    use std::path::PathBuf;

    fn temp_db_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("downloads.db");
        (dir, path)
    }

    #[test]
    fn db_worker_persists_event_flow_end_to_end() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = spawn_worker(db, rx);

        tx.send(DbEvent::Added {
            id: DownloadId(10),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/movie.mkv".to_string(),
            },
            destination: PathBuf::from("/tmp/movie.mkv"),
        })
        .unwrap();
        tx.send(DbEvent::Started { id: DownloadId(10) }).unwrap();
        tx.send(DbEvent::Paused {
            id: DownloadId(10),
            downloaded_bytes: 64,
            resume_data: Some(ProviderResumeData::Http(HttpResumeData::new(vec![
                ChunkSnapshot {
                    start: 0,
                    end: 100,
                    downloaded: 64,
                },
            ]))),
        })
        .unwrap();
        drop(tx);
        worker.join();

        let history = HistoryReader::open_at(&db_path).unwrap();
        let rows = history
            .load(crate::engine::types::HistoryFilter::Paused, "")
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, DownloadStatus::Paused);
        assert_eq!(rows[0].downloaded_bytes, 64);

        let db = Db::open_at(&db_path).unwrap();
        let (saved, next_id) = db.load_for_restore().unwrap();
        assert_eq!(next_id, 10);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, DownloadId(10));
        assert_eq!(saved[0].source.kind(), "http");
        let resume = saved[0].resume_data.as_ref().unwrap().as_http().unwrap();
        assert_eq!(resume.chunks.len(), 1);
        assert_eq!(resume.chunks[0].downloaded, 64);
    }
}
