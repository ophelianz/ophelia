mod db;
pub use db::{Db, HistoryReader};

use crate::engine::types::DbEvent;

pub fn spawn_worker(db: Db, rx: std::sync::mpsc::Receiver<DbEvent>) {
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
