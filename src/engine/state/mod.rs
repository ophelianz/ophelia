mod db;
use db::Db;
pub use db::HistoryReader;

use crate::engine::types::{DbEvent, SavedDownload};

pub struct StateBootstrap {
    pub db_tx: std::sync::mpsc::Sender<DbEvent>,
    pub history_reader: HistoryReader,
    pub saved_downloads: Vec<SavedDownload>,
    pub next_download_id: u64,
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
    spawn_worker(db, db_rx);

    Ok(StateBootstrap {
        db_tx,
        history_reader: HistoryReader::open()?,
        saved_downloads,
        next_download_id: max_id + 1,
    })
}

fn spawn_worker(db: Db, rx: std::sync::mpsc::Receiver<DbEvent>) {
    std::thread::Builder::new()
        .name("db-worker".into())
        .spawn(move || {
            for event in rx {
                if let Err(e) = db.handle(event) {
                    tracing::error!("db worker: {e}");
                }
            }
        })
        .expect("failed to spawn db worker thread");
}
