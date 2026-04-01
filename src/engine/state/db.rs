use std::path::PathBuf;

use rusqlite::{Connection, params};

use crate::engine::types::{ChunkSnapshot, DbEvent, DownloadId, DownloadStatus, HistoryFilter, HistoryRow, SavedDownload};

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> rusqlite::Result<Self> {
        let path = db_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("
            CREATE TABLE IF NOT EXISTS downloads (
                id          INTEGER PRIMARY KEY,
                url         TEXT NOT NULL,
                destination TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'pending',
                total_bytes INTEGER,
                downloaded  INTEGER NOT NULL DEFAULT 0,
                added_at    INTEGER NOT NULL,
                finished_at INTEGER,
                etag        TEXT,
                mime_type   TEXT
            );
            CREATE TABLE IF NOT EXISTS chunks (
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                slot        INTEGER NOT NULL,
                start       INTEGER NOT NULL,
                end_byte    INTEGER NOT NULL,
                downloaded  INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (download_id, slot)
            );
        ")
    }

    /// On startup: any row still marked 'downloading' means we crashed mid-transfer.
    /// Normalize to 'paused' so they show up as resumable.
    pub fn normalize_stale(&self) -> rusqlite::Result<usize> {
        let n = self.conn.execute(
            "UPDATE downloads SET status = 'paused' WHERE status = 'downloading'",
            [],
        )?;
        if n > 0 {
            tracing::info!(count = n, "normalized stale downloads → paused");
        }
        Ok(n)
    }

    /// Remove DB rows whose .ophelia_part file no longer exists on disk.
    /// Happens when the user manually deleted a partial download.
    pub fn validate_integrity(&self) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, destination FROM downloads WHERE status IN ('paused', 'pending')",
        )?;

        let orphans: Vec<i64> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, dest)| {
                let part = format!("{}.ophelia_part", dest);
                !std::path::Path::new(&part).exists()
            })
            .map(|(id, _)| id)
            .collect();

        if !orphans.is_empty() {
            tracing::info!(count = orphans.len(), "removing orphaned downloads (part file missing)");
            for id in orphans {
                self.conn.execute("DELETE FROM downloads WHERE id = ?1", params![id])?;
            }
        }
        Ok(())
    }

    /// Load all paused/pending downloads and their chunk snapshots for startup restoration.
    /// Also returns the global max id so DownloadEngine can continue the id sequence.
    pub fn load_for_restore(&self) -> rusqlite::Result<(Vec<SavedDownload>, u64)> {
        let max_id = self.conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM downloads", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64;

        let mut stmt = self.conn.prepare(
            "SELECT id, url, destination, downloaded, total_bytes
             FROM downloads WHERE status IN ('paused', 'pending') ORDER BY id",
        )?;

        let mut downloads: Vec<SavedDownload> = stmt
            .query_map([], |row| {
                Ok(SavedDownload {
                    id: DownloadId(row.get::<_, i64>(0)? as u64),
                    url: row.get(1)?,
                    destination: PathBuf::from(row.get::<_, String>(2)?),
                    downloaded_bytes: row.get::<_, i64>(3)? as u64,
                    total_bytes: row.get::<_, Option<i64>>(4)?.map(|b| b as u64),
                    chunks: Vec::new(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        for dl in &mut downloads {
            let mut cstmt = self.conn.prepare(
                "SELECT start, end_byte, downloaded FROM chunks
                 WHERE download_id = ?1 ORDER BY slot",
            )?;
            dl.chunks = cstmt
                .query_map(params![dl.id.0 as i64], |row| {
                    Ok(ChunkSnapshot {
                        start: row.get::<_, i64>(0)? as u64,
                        end: row.get::<_, i64>(1)? as u64,
                        downloaded: row.get::<_, i64>(2)? as u64,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
        }

        Ok((downloads, max_id))
    }

    /// Sole write path, called only from the DbEventWorker thread.
    pub fn handle(&self, event: DbEvent) -> rusqlite::Result<()> {
        match event {
            DbEvent::Started { id, url, destination } => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO downloads (id, url, destination, status, added_at)
                     VALUES (?1, ?2, ?3, 'downloading', ?4)",
                    params![
                        id.0 as i64,
                        url,
                        destination.to_string_lossy().as_ref(),
                        unix_ms()
                    ],
                )?;
            }
            DbEvent::Paused { id, downloaded_bytes, chunks } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'paused', downloaded = ?1 WHERE id = ?2",
                    params![downloaded_bytes as i64, id.0 as i64],
                )?;
                self.save_chunks(id, &chunks)?;
            }
            DbEvent::Resumed { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'downloading' WHERE id = ?1",
                    params![id.0 as i64],
                )?;
            }
            DbEvent::Finished { id, total_bytes } => {
                self.conn.execute(
                    "UPDATE downloads
                     SET status = 'finished', total_bytes = ?1, downloaded = ?1, finished_at = ?2
                     WHERE id = ?3",
                    params![total_bytes as i64, unix_ms(), id.0 as i64],
                )?;
                // Chunks not needed once finished; CASCADE would handle it on delete
                // but we delete explicitly to free space immediately.
                self.conn.execute(
                    "DELETE FROM chunks WHERE download_id = ?1",
                    params![id.0 as i64],
                )?;
            }
            DbEvent::Error { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'error' WHERE id = ?1",
                    params![id.0 as i64],
                )?;
            }
            DbEvent::Removed { id } => {
                // ON DELETE CASCADE handles the chunks table.
                self.conn.execute(
                    "DELETE FROM downloads WHERE id = ?1",
                    params![id.0 as i64],
                )?;
            }
        }
        Ok(())
    }

    fn save_chunks(&self, id: DownloadId, chunks: &[ChunkSnapshot]) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM chunks WHERE download_id = ?1",
            params![id.0 as i64],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO chunks (download_id, slot, start, end_byte, downloaded)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (slot, chunk) in chunks.iter().enumerate() {
            stmt.execute(params![
                id.0 as i64,
                slot as i64,
                chunk.start as i64,
                chunk.end as i64,
                chunk.downloaded as i64,
            ])?;
        }
        Ok(())
    }
}

pub fn db_path() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Library")
        .join("Application Support")
        .join("Ophelia")
        .join("downloads.db")
}

// --- read-only history queries -------------------------------------------

/// A lightweight read-only connection used by the UI history view.
/// WAL mode lets this coexist with the DbEventWorker's write connection.
pub struct HistoryReader {
    conn: Connection,
}

impl HistoryReader {
    pub fn open() -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path())?;
        // Best-effort read-only hint; doesn't error on older SQLite.
        let _ = conn.execute_batch("PRAGMA query_only=ON;");
        Ok(Self { conn })
    }

    pub fn load(&self, filter: HistoryFilter, search: &str) -> rusqlite::Result<Vec<HistoryRow>> {
        let status_clause = match filter {
            HistoryFilter::All      => "",
            HistoryFilter::Finished => "AND status = 'finished'",
            HistoryFilter::Error    => "AND status = 'error'",
            HistoryFilter::Paused   => "AND status = 'paused'",
        };
        let sql = format!(
            "SELECT id, url, destination, status, total_bytes, downloaded, added_at, finished_at
             FROM downloads
             WHERE 1=1 {status_clause}
               AND (?1 = '' OR destination LIKE '%' || ?1 || '%' OR url LIKE '%' || ?1 || '%')
             ORDER BY added_at DESC LIMIT 500"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![search], |row| {
                let status_str: String = row.get(3)?;
                Ok(HistoryRow {
                    id: DownloadId(row.get::<_, i64>(0)? as u64),
                    url: row.get(1)?,
                    destination: row.get(2)?,
                    status: status_from_str(&status_str),
                    total_bytes: row.get::<_, Option<i64>>(4)?.map(|b| b as u64),
                    downloaded_bytes: row.get::<_, i64>(5)? as u64,
                    added_at: row.get(6)?,
                    finished_at: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

fn status_from_str(s: &str) -> DownloadStatus {
    match s {
        "finished"    => DownloadStatus::Finished,
        "error"       => DownloadStatus::Error,
        "paused"      => DownloadStatus::Paused,
        "downloading" => DownloadStatus::Downloading,
        _             => DownloadStatus::Pending,
    }
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
