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

use std::io::{Error, ErrorKind};

use rusqlite::{Connection, params, types::Type};

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
            let start = checked_u64(row.get::<_, i64>(0)?, 0)?;
            let end = checked_u64(row.get::<_, i64>(1)?, 1)?;
            let downloaded = checked_u64(row.get::<_, i64>(2)?, 2)?;
            if start >= end {
                return Err(invalid_chunk_row(1, "chunk end must be greater than start"));
            }
            Ok(ChunkSnapshot {
                start,
                end,
                downloaded: downloaded.min(end - start),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

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
            if chunk.start >= chunk.end {
                return Err(invalid_chunk_row(3, "chunk end must be greater than start"));
            }
            let downloaded = chunk.downloaded.min(chunk.end - chunk.start);
            stmt.execute(params![
                download_id.0 as i64,
                slot as i64,
                checked_i64(chunk.start)?,
                checked_i64(chunk.end)?,
                checked_i64(downloaded)?,
            ])?;
        }
    }

    Ok(())
}

fn checked_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn checked_i64(value: u64) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(Error::new(
            ErrorKind::InvalidData,
            "chunk value does not fit in SQLite INTEGER",
        )))
    })
}

fn invalid_chunk_row(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Integer,
        Box::new(Error::new(ErrorKind::InvalidData, message)),
    )
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::{load_resume_data, migrate, save_resume_data};
    use crate::engine::types::{ChunkSnapshot, DownloadId, HttpResumeData, ProviderResumeData};

    #[test]
    fn load_resume_data_rejects_negative_chunk_values() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO chunks (download_id, slot, start, end_byte, downloaded)
             VALUES (?1, 0, -1, 100, 0)",
            params![1_i64],
        )
        .unwrap();

        assert!(load_resume_data(&conn, DownloadId(1)).is_err());
    }

    #[test]
    fn load_resume_data_rejects_empty_or_reversed_ranges() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO chunks (download_id, slot, start, end_byte, downloaded)
             VALUES (?1, 0, 100, 100, 0)",
            params![1_i64],
        )
        .unwrap();

        assert!(load_resume_data(&conn, DownloadId(1)).is_err());
    }

    #[test]
    fn save_resume_data_clamps_downloaded_to_chunk_length() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let resume = ProviderResumeData::Http(HttpResumeData::new(vec![ChunkSnapshot {
            start: 10,
            end: 20,
            downloaded: 50,
        }]));

        save_resume_data(&conn, DownloadId(1), Some(&resume)).unwrap();
        let loaded = load_resume_data(&conn, DownloadId(1)).unwrap().unwrap();

        let ProviderResumeData::Http(data) = loaded;
        assert_eq!(data.chunks[0].downloaded, 10);
    }
}
