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

use rusqlite::{Connection, params};

use crate::engine::types::{ChunkSnapshot, DownloadId, HttpResumeData, ProviderResumeData};

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS chunks (
            download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
            slot        INTEGER NOT NULL,
            start       INTEGER NOT NULL,
            end_byte    INTEGER NOT NULL,
            downloaded  INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (download_id, slot)
        );
        ",
    )
}

pub(super) fn load_resume_data(
    conn: &Connection,
    download_id: DownloadId,
) -> rusqlite::Result<Option<ProviderResumeData>> {
    let mut stmt = conn.prepare(
        "SELECT start, end_byte, downloaded FROM chunks
         WHERE download_id = ?1 ORDER BY slot",
    )?;
    let chunks: Vec<ChunkSnapshot> = stmt
        .query_map(params![download_id.0 as i64], |row| {
            Ok(ChunkSnapshot {
                start: row.get::<_, i64>(0)? as u64,
                end: row.get::<_, i64>(1)? as u64,
                downloaded: row.get::<_, i64>(2)? as u64,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    Ok((!chunks.is_empty()).then(|| ProviderResumeData::Http(HttpResumeData::new(chunks))))
}

pub(super) fn save_resume_data(
    conn: &Connection,
    download_id: DownloadId,
    resume_data: Option<&ProviderResumeData>,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM chunks WHERE download_id = ?1",
        params![download_id.0 as i64],
    )?;

    if let Some(ProviderResumeData::Http(data)) = resume_data {
        let mut stmt = conn.prepare(
            "INSERT INTO chunks (download_id, slot, start, end_byte, downloaded)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (slot, chunk) in data.chunks.iter().enumerate() {
            stmt.execute(params![
                download_id.0 as i64,
                slot as i64,
                chunk.start as i64,
                chunk.end as i64,
                chunk.downloaded as i64,
            ])?;
        }
    }

    Ok(())
}
