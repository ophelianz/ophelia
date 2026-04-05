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

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::engine::state::http;
use crate::engine::types::{
    ArtifactState, DbEvent, DownloadId, DownloadStatus, HistoryFilter, HistoryRow,
    PersistedDownloadSource, ProviderResumeData, SavedDownload,
};

#[cfg(test)]
const HTTP_PROVIDER_KIND: &str = "http";

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> rusqlite::Result<Self> {
        Self::open_at(db_path())
    }

    pub(super) fn open_at(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path)?;
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
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS downloads (
                id          INTEGER PRIMARY KEY,
                provider_kind TEXT NOT NULL DEFAULT 'http',
                url         TEXT NOT NULL,
                destination TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'pending',
                artifact_state TEXT NOT NULL DEFAULT 'present',
                total_bytes INTEGER,
                downloaded  INTEGER NOT NULL DEFAULT 0,
                added_at    INTEGER NOT NULL,
                finished_at INTEGER,
                etag        TEXT,
                mime_type   TEXT
            );
        ",
        )?;
        self.ensure_downloads_provider_kind_column()?;
        self.ensure_downloads_artifact_state_column()?;
        http::migrate(&self.conn)?;
        Ok(())
    }

    fn ensure_downloads_provider_kind_column(&self) -> rusqlite::Result<()> {
        if self.downloads_has_column("provider_kind")? {
            return Ok(());
        }

        self.conn.execute(
            "ALTER TABLE downloads ADD COLUMN provider_kind TEXT NOT NULL DEFAULT 'http'",
            [],
        )?;
        Ok(())
    }

    fn ensure_downloads_artifact_state_column(&self) -> rusqlite::Result<()> {
        if self.downloads_has_column("artifact_state")? {
            return Ok(());
        }

        self.conn.execute(
            "ALTER TABLE downloads ADD COLUMN artifact_state TEXT NOT NULL DEFAULT 'present'",
            [],
        )?;
        Ok(())
    }

    fn downloads_has_column(&self, column_name: &str) -> rusqlite::Result<bool> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(downloads)")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column_name {
                return Ok(true);
            }
        }
        Ok(false)
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
            "SELECT id, destination FROM downloads
             WHERE status IN ('paused', 'pending') AND downloaded > 0",
        )?;

        let orphans: Vec<i64> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .filter(|(_, dest)| {
                let part = format!("{}.ophelia_part", dest);
                !std::path::Path::new(&part).exists()
            })
            .map(|(id, _)| id)
            .collect();

        if !orphans.is_empty() {
            tracing::info!(
                count = orphans.len(),
                "marking orphaned downloads missing (part file missing)"
            );
            for id in orphans {
                self.conn.execute(
                    "UPDATE downloads
                     SET status = 'cancelled', artifact_state = 'missing', finished_at = COALESCE(finished_at, ?1)
                     WHERE id = ?2",
                    params![unix_ms(), id],
                )?;
                self.save_resume_data(DownloadId(id as u64), None)?;
            }
        }
        Ok(())
    }

    /// Load all paused/pending downloads and their provider-specific resume state
    /// for startup restoration.
    /// Also returns the global max id so DownloadEngine can continue the id sequence.
    pub fn load_for_restore(&self) -> rusqlite::Result<(Vec<SavedDownload>, u64)> {
        let max_id = self
            .conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM downloads", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64;

        let mut stmt = self.conn.prepare(
            "SELECT id, provider_kind, url, destination, downloaded, total_bytes
             FROM downloads WHERE status IN ('paused', 'pending') ORDER BY id",
        )?;

        struct SavedDownloadRow {
            id: DownloadId,
            provider_kind: String,
            url: String,
            destination: PathBuf,
            downloaded_bytes: u64,
            total_bytes: Option<u64>,
        }

        let mut downloads = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok(SavedDownloadRow {
                id: DownloadId(row.get::<_, i64>(0)? as u64),
                provider_kind: row.get(1)?,
                url: row.get(2)?,
                destination: PathBuf::from(row.get::<_, String>(3)?),
                downloaded_bytes: row.get::<_, i64>(4)? as u64,
                total_bytes: row.get::<_, Option<i64>>(5)?.map(|b| b as u64),
            })
        })?;

        for row in rows {
            let row = row?;
            let Some(source) = PersistedDownloadSource::from_parts(&row.provider_kind, row.url)
            else {
                tracing::warn!(
                    id = row.id.0,
                    provider_kind = row.provider_kind,
                    "skipping restore for unsupported persisted provider kind"
                );
                continue;
            };

            downloads.push(SavedDownload {
                id: row.id,
                source,
                destination: row.destination,
                downloaded_bytes: row.downloaded_bytes,
                total_bytes: row.total_bytes,
                resume_data: None,
            });
        }

        for dl in &mut downloads {
            dl.resume_data = self.load_provider_resume_data(dl.id, &dl.source)?;
        }

        Ok((downloads, max_id))
    }

    /// Sole write path, called only from the DbEventWorker thread.
    pub fn handle(&self, event: DbEvent) -> rusqlite::Result<()> {
        match event {
            DbEvent::Added {
                id,
                source,
                destination,
            } => {
                self.conn.execute(
                    "INSERT OR IGNORE INTO downloads
                     (id, provider_kind, url, destination, status, artifact_state, added_at)
                     VALUES (?1, ?2, ?3, ?4, 'pending', 'present', ?5)",
                    params![
                        id.0 as i64,
                        source.kind(),
                        source.locator(),
                        destination.to_string_lossy().as_ref(),
                        unix_ms()
                    ],
                )?;
            }
            DbEvent::DestinationChanged { id, destination } => {
                self.conn.execute(
                    "UPDATE downloads SET destination = ?1 WHERE id = ?2",
                    params![destination.to_string_lossy().as_ref(), id.0 as i64],
                )?;
            }
            DbEvent::Queued { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'pending' WHERE id = ?1",
                    params![id.0 as i64],
                )?;
            }
            DbEvent::Started { id } | DbEvent::Resumed { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'downloading' WHERE id = ?1",
                    params![id.0 as i64],
                )?;
            }
            DbEvent::Paused {
                id,
                downloaded_bytes,
                resume_data,
            } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'paused', downloaded = ?1 WHERE id = ?2",
                    params![downloaded_bytes as i64, id.0 as i64],
                )?;
                self.save_resume_data(id, resume_data.as_ref())?;
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
                self.save_resume_data(id, None)?;
            }
            DbEvent::Error { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'error', finished_at = COALESCE(finished_at, ?1) WHERE id = ?2",
                    params![unix_ms(), id.0 as i64],
                )?;
            }
            DbEvent::Cancelled { id } => {
                self.conn.execute(
                    "UPDATE downloads SET status = 'cancelled', finished_at = COALESCE(finished_at, ?1) WHERE id = ?2",
                    params![unix_ms(), id.0 as i64],
                )?;
                self.save_resume_data(id, None)?;
            }
            DbEvent::ArtifactStateChanged { id, artifact_state } => {
                self.conn.execute(
                    "UPDATE downloads SET artifact_state = ?1 WHERE id = ?2",
                    params![artifact_state_to_str(artifact_state), id.0 as i64],
                )?;
            }
        }
        Ok(())
    }

    fn save_resume_data(
        &self,
        id: DownloadId,
        resume_data: Option<&ProviderResumeData>,
    ) -> rusqlite::Result<()> {
        http::save_resume_data(&self.conn, id, resume_data)
    }

    fn load_provider_resume_data(
        &self,
        download_id: DownloadId,
        source: &PersistedDownloadSource,
    ) -> rusqlite::Result<Option<ProviderResumeData>> {
        match source {
            PersistedDownloadSource::Http { .. } => http::load_resume_data(&self.conn, download_id),
        }
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
        Self::open_at(db_path())
    }

    pub(super) fn open_at(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path.as_ref())?;
        // Best-effort read-only hint; doesn't error on older SQLite.
        let _ = conn.execute_batch("PRAGMA query_only=ON;");
        Ok(Self { conn })
    }

    pub fn load(&self, filter: HistoryFilter, search: &str) -> rusqlite::Result<Vec<HistoryRow>> {
        let status_clause = match filter {
            HistoryFilter::All => "",
            HistoryFilter::Finished => "AND status = 'finished'",
            HistoryFilter::Error => "AND status = 'error'",
            HistoryFilter::Paused => "AND status = 'paused'",
            HistoryFilter::Cancelled => "AND status = 'cancelled'",
        };
        let sql = format!(
            "SELECT id, provider_kind, url, destination, status, artifact_state, total_bytes, downloaded, added_at, finished_at
             FROM downloads
             WHERE 1=1 {status_clause}
               AND (?1 = ''
                    OR destination LIKE '%' || ?1 || '%'
                    OR url LIKE '%' || ?1 || '%'
                    OR provider_kind LIKE '%' || ?1 || '%')
             ORDER BY added_at DESC LIMIT 500"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![search], |row| {
                let provider_kind: String = row.get(1)?;
                let locator: String = row.get(2)?;
                let status_str: String = row.get(4)?;
                let artifact_state_str: String = row.get(5)?;
                Ok(HistoryRow {
                    id: DownloadId(row.get::<_, i64>(0)? as u64),
                    provider_kind: provider_kind.clone(),
                    source_label: history_source_label(&provider_kind, locator),
                    destination: row.get(3)?,
                    status: status_from_str(&status_str),
                    artifact_state: artifact_state_from_str(&artifact_state_str),
                    total_bytes: row.get::<_, Option<i64>>(6)?.map(|b| b as u64),
                    downloaded_bytes: row.get::<_, i64>(7)? as u64,
                    added_at: row.get(8)?,
                    finished_at: row.get(9)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

fn history_source_label(provider_kind: &str, locator: String) -> String {
    PersistedDownloadSource::from_parts(provider_kind, locator.clone())
        .map(|source| source.display_label().to_string())
        .unwrap_or(locator)
}

fn status_from_str(s: &str) -> DownloadStatus {
    match s {
        "finished" => DownloadStatus::Finished,
        "error" => DownloadStatus::Error,
        "paused" => DownloadStatus::Paused,
        "downloading" => DownloadStatus::Downloading,
        "cancelled" => DownloadStatus::Cancelled,
        _ => DownloadStatus::Pending,
    }
}

fn artifact_state_from_str(s: &str) -> ArtifactState {
    match s {
        "deleted" => ArtifactState::Deleted,
        "missing" => ArtifactState::Missing,
        _ => ArtifactState::Present,
    }
}

fn artifact_state_to_str(state: ArtifactState) -> &'static str {
    match state {
        ArtifactState::Present => "present",
        ArtifactState::Deleted => "deleted",
        ArtifactState::Missing => "missing",
    }
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::{ChunkSnapshot, HttpResumeData};
    use tempfile::TempDir;

    fn temp_db_path() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("downloads.db");
        (dir, path)
    }

    #[test]
    fn migrate_adds_provider_kind_to_legacy_downloads_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE downloads (
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
            CREATE TABLE chunks (
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                slot        INTEGER NOT NULL,
                start       INTEGER NOT NULL,
                end_byte    INTEGER NOT NULL,
                downloaded  INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (download_id, slot)
            );
            ",
        )
        .unwrap();

        let db = Db { conn };
        db.migrate().unwrap();

        assert!(db.downloads_has_column("provider_kind").unwrap());
        assert!(db.downloads_has_column("artifact_state").unwrap());

        let provider_kind: String = db
            .conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('downloads') WHERE name = 'provider_kind'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provider_kind, format!("'{HTTP_PROVIDER_KIND}'"));

        let artifact_state: String = db
            .conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('downloads') WHERE name = 'artifact_state'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artifact_state, "'present'");
    }

    #[test]
    fn load_for_restore_reads_provider_kind_and_http_resume_data() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();
        db.migrate().unwrap();

        db.conn
            .execute(
                "INSERT INTO downloads
                 (id, provider_kind, url, destination, status, total_bytes, downloaded, added_at)
                 VALUES (?1, ?2, ?3, ?4, 'paused', ?5, ?6, ?7)",
                params![
                    7_i64,
                    HTTP_PROVIDER_KIND,
                    "https://example.com/file.bin",
                    "/tmp/file.bin",
                    100_i64,
                    25_i64,
                    1_i64
                ],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO chunks (download_id, slot, start, end_byte, downloaded)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![7_i64, 0_i64, 0_i64, 100_i64, 25_i64],
            )
            .unwrap();

        let (downloads, max_id) = db.load_for_restore().unwrap();
        assert_eq!(max_id, 7);
        assert_eq!(downloads.len(), 1);

        let saved = &downloads[0];
        assert_eq!(saved.id, DownloadId(7));
        assert_eq!(saved.source.kind(), HTTP_PROVIDER_KIND);
        assert_eq!(saved.source.locator(), "https://example.com/file.bin");
        assert_eq!(saved.downloaded_bytes, 25);
        assert_eq!(saved.total_bytes, Some(100));

        let resume = saved.resume_data.as_ref().unwrap().as_http().unwrap();
        assert_eq!(resume.chunks.len(), 1);
        assert_eq!(resume.chunks[0].start, 0);
        assert_eq!(resume.chunks[0].end, 100);
        assert_eq!(resume.chunks[0].downloaded, 25);
    }

    #[test]
    fn load_for_restore_skips_unknown_provider_kind() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();

        db.conn
            .execute(
                "INSERT INTO downloads
                 (id, provider_kind, url, destination, status, downloaded, added_at)
                 VALUES (?1, ?2, ?3, ?4, 'paused', ?5, ?6)",
                params![1_i64, "unknown", "opaque:thing", "/tmp/thing", 0_i64, 1_i64],
            )
            .unwrap();

        let (downloads, max_id) = db.load_for_restore().unwrap();
        assert_eq!(max_id, 1);
        assert!(downloads.is_empty());
    }

    #[test]
    fn destination_changed_updates_restore_and_history_views() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();

        db.handle(DbEvent::Added {
            id: DownloadId(11),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/download".to_string(),
            },
            destination: PathBuf::from("/tmp/download"),
        })
        .unwrap();
        db.handle(DbEvent::DestinationChanged {
            id: DownloadId(11),
            destination: PathBuf::from("/tmp/Movies/movie.mp4"),
        })
        .unwrap();
        db.handle(DbEvent::Paused {
            id: DownloadId(11),
            downloaded_bytes: 10,
            resume_data: Some(ProviderResumeData::Http(HttpResumeData::new(vec![
                ChunkSnapshot {
                    start: 0,
                    end: 100,
                    downloaded: 10,
                },
            ]))),
        })
        .unwrap();

        let history = HistoryReader::open_at(&db_path).unwrap();
        let rows = history.load(HistoryFilter::Paused, "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].destination, "/tmp/Movies/movie.mp4");

        let (saved, _) = db.load_for_restore().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].destination, PathBuf::from("/tmp/Movies/movie.mp4"));
    }

    #[test]
    fn history_reader_filters_and_searches_transfer_rows() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();

        db.handle(DbEvent::Added {
            id: DownloadId(1),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/success.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/success.zip"),
        })
        .unwrap();
        db.handle(DbEvent::Started { id: DownloadId(1) }).unwrap();
        db.handle(DbEvent::Finished {
            id: DownloadId(1),
            total_bytes: 100,
        })
        .unwrap();

        db.handle(DbEvent::Added {
            id: DownloadId(2),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/failure.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/failure.zip"),
        })
        .unwrap();
        db.handle(DbEvent::Started { id: DownloadId(2) }).unwrap();
        db.handle(DbEvent::Error { id: DownloadId(2) }).unwrap();

        db.handle(DbEvent::Added {
            id: DownloadId(3),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/paused.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/paused.zip"),
        })
        .unwrap();
        db.handle(DbEvent::Started { id: DownloadId(3) }).unwrap();
        db.handle(DbEvent::Paused {
            id: DownloadId(3),
            downloaded_bytes: 12,
            resume_data: Some(ProviderResumeData::Http(HttpResumeData::new(vec![
                ChunkSnapshot {
                    start: 0,
                    end: 100,
                    downloaded: 12,
                },
            ]))),
        })
        .unwrap();

        let history = HistoryReader::open_at(&db_path).unwrap();

        let finished = history.load(HistoryFilter::Finished, "").unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].status, DownloadStatus::Finished);
        assert_eq!(finished[0].provider_kind, HTTP_PROVIDER_KIND);
        assert_eq!(finished[0].source_label, "https://example.com/success.zip");
        assert_eq!(finished[0].destination, "/tmp/success.zip");

        let paused = history.load(HistoryFilter::Paused, "").unwrap();
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].downloaded_bytes, 12);

        let searched = history.load(HistoryFilter::All, "failure").unwrap();
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].status, DownloadStatus::Error);
        assert!(searched[0].source_label.contains("failure"));

        let provider_search = history
            .load(HistoryFilter::All, HTTP_PROVIDER_KIND)
            .unwrap();
        assert_eq!(provider_search.len(), 3);
    }

    #[test]
    fn history_reader_keeps_cancelled_rows_and_tracks_artifact_state() {
        let (_dir, db_path) = temp_db_path();
        let db = Db::open_at(&db_path).unwrap();

        db.handle(DbEvent::Added {
            id: DownloadId(4),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/deleted.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/deleted.zip"),
        })
        .unwrap();
        db.handle(DbEvent::Cancelled { id: DownloadId(4) }).unwrap();
        db.handle(DbEvent::ArtifactStateChanged {
            id: DownloadId(4),
            artifact_state: ArtifactState::Deleted,
        })
        .unwrap();

        let history = HistoryReader::open_at(&db_path).unwrap();
        let cancelled = history.load(HistoryFilter::Cancelled, "").unwrap();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].status, DownloadStatus::Cancelled);
        assert_eq!(cancelled[0].artifact_state, ArtifactState::Deleted);
    }
}
