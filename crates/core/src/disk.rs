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

//! Service-owned disk sessions, writes, finalize, and artifact cleanup

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bitvec::prelude::{BitVec, Lsb0};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::engine::alloc::preallocate;
use crate::engine::destination::{
    FinalizeStrategy, ResolvedDestination, finalize_part_file, part_path_for,
};
use crate::engine::types::{ArtifactState, TransferId};

const WRITE_JOB_CAPACITY: usize = 256;
const NO_INDEX: usize = usize::MAX;

#[derive(Clone, Default)]
pub(crate) struct DiskHandle {
    sessions: Arc<Mutex<DiskSessions>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DiskSessionId {
    slot: u32,
    generation: u32,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiskSessionState {
    Open,
    Committed,
    Failed,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiskArtifactClassification {
    Missing,
    FinalOnly,
    PartOnly,
    FinalAndPart,
}

pub(crate) struct DiskLease {
    handle: DiskHandle,
    session: DiskSessionId,
    file: File,
}

#[derive(Clone)]
pub(crate) struct DiskSessionLease {
    handle: DiskHandle,
    session: DiskSessionId,
}

#[derive(Default)]
pub(crate) struct DiskSessions {
    physical_bytes_written: Vec<u64>,
    logical_bytes_confirmed: Vec<u64>,
    expected_lens: Vec<u64>,
    generations: Vec<u32>,
    states: Vec<DiskSessionState>,
    transfer_ids: Vec<TransferId>,
    flags: DiskSessionFlags,
    active_rows: Vec<DiskSessionId>,
    active_positions: Vec<usize>,
    committed_rows: Vec<DiskSessionId>,
    failed_rows: Vec<DiskSessionId>,
    paths: Vec<DiskSessionPaths>,
    failure_messages: Vec<Option<String>>,
    index_by_transfer: HashMap<TransferId, DiskSessionId>,
}

#[derive(Default)]
struct DiskSessionFlags {
    expected_len: BitVec<usize, Lsb0>,
    active: BitVec<usize, Lsb0>,
    committed: BitVec<usize, Lsb0>,
    failed: BitVec<usize, Lsb0>,
}

struct DiskSessionPaths {
    part_path: PathBuf,
    destination: PathBuf,
    finalize_strategy: FinalizeStrategy,
}

impl DiskHandle {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(DiskSessions::new())),
        }
    }

    pub(crate) fn create_new(
        &self,
        transfer_id: TransferId,
        resolved: ResolvedDestination,
        expected_len: Option<u64>,
    ) -> io::Result<DiskLease> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&resolved.part_path)?;
        if let Some(len) = expected_len {
            preallocate(&file, len)?;
        }
        let session =
            self.sessions
                .lock()
                .unwrap()
                .insert(transfer_id, resolved, expected_len, 0, 0);
        Ok(DiskLease {
            handle: self.clone(),
            session,
            file,
        })
    }

    pub(crate) fn resume_existing(
        &self,
        transfer_id: TransferId,
        resolved: ResolvedDestination,
        expected_len: Option<u64>,
        initial_logical_bytes: u64,
    ) -> io::Result<DiskLease> {
        let file = OpenOptions::new().write(true).open(&resolved.part_path)?;
        let session = self.sessions.lock().unwrap().insert(
            transfer_id,
            resolved,
            expected_len,
            0,
            initial_logical_bytes,
        );
        Ok(DiskLease {
            handle: self.clone(),
            session,
            file,
        })
    }

    pub(crate) fn delete_artifacts(&self, destination: &Path) -> ArtifactState {
        let mut removed_any = false;
        for path in artifact_paths(destination) {
            match std::fs::remove_file(&path) {
                Ok(()) => removed_any = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(path = %path.display(), "failed to delete artifact: {error}");
                }
            }
        }

        match self.classify_artifacts(destination) {
            DiskArtifactClassification::Missing if removed_any => ArtifactState::Deleted,
            DiskArtifactClassification::Missing => ArtifactState::Missing,
            _ => ArtifactState::Present,
        }
    }

    pub(crate) fn artifact_state(&self, destination: &Path) -> ArtifactState {
        match self.classify_artifacts(destination) {
            DiskArtifactClassification::Missing => ArtifactState::Missing,
            _ => ArtifactState::Present,
        }
    }

    pub(crate) fn classify_artifacts(&self, destination: &Path) -> DiskArtifactClassification {
        let final_exists = destination.exists();
        let part_exists = part_path_for(destination).exists();
        match (final_exists, part_exists) {
            (false, false) => DiskArtifactClassification::Missing,
            (true, false) => DiskArtifactClassification::FinalOnly,
            (false, true) => DiskArtifactClassification::PartOnly,
            (true, true) => DiskArtifactClassification::FinalAndPart,
        }
    }

    pub(crate) fn remove_stale_part_for_fresh_resume(&self, destination: &Path) {
        let part_path = part_path_for(destination);
        match std::fs::remove_file(&part_path) {
            Ok(()) => {
                tracing::info!(
                    path = %part_path.display(),
                    "removed stale part file before restarting restored download"
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    ?error,
                    path = %part_path.display(),
                    "failed to remove stale part file before restarting restored download"
                );
            }
        }
    }

    fn record_physical(&self, session: DiskSessionId, bytes: u64) {
        self.sessions
            .lock()
            .unwrap()
            .add_physical_bytes(session, bytes);
    }

    fn confirm_logical(&self, session: DiskSessionId, bytes: u64) {
        self.sessions
            .lock()
            .unwrap()
            .add_logical_bytes(session, bytes);
    }

    fn commit(&self, session: DiskSessionId) -> io::Result<()> {
        self.sessions.lock().unwrap().commit(session)
    }

    fn mark_failed(&self, session: DiskSessionId, message: Option<String>) {
        self.sessions.lock().unwrap().mark_failed(session, message);
    }
}

impl DiskLease {
    pub(crate) fn session(&self) -> DiskSessionId {
        self.session
    }

    pub(crate) fn mark_failed(self, message: Option<String>) {
        self.into_session().mark_failed(message);
    }

    pub(crate) fn into_session(self) -> DiskSessionLease {
        DiskSessionLease {
            handle: self.handle,
            session: self.session,
        }
    }

    pub(crate) fn split_for_writes<T>(self) -> (DiskSessionLease, DiskWriter<T>)
    where
        T: Copy + Send + 'static,
    {
        let lease = DiskSessionLease {
            handle: self.handle,
            session: self.session,
        };
        let writer = DiskWriter::spawn(self.file, lease.clone());
        (lease, writer)
    }
}

impl DiskSessionLease {
    pub(crate) fn session(&self) -> DiskSessionId {
        self.session
    }

    pub(crate) fn confirm_logical(&self, bytes: u64) {
        self.handle.confirm_logical(self.session, bytes);
    }

    pub(crate) fn commit(self) -> io::Result<()> {
        self.handle.commit(self.session)
    }

    pub(crate) fn mark_failed(self, message: Option<String>) {
        self.handle.mark_failed(self.session, message);
    }

    fn record_physical(&self, bytes: u64) {
        self.handle.record_physical(self.session, bytes);
    }
}

impl DiskSessions {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn insert(
        &mut self,
        transfer_id: TransferId,
        resolved: ResolvedDestination,
        expected_len: Option<u64>,
        initial_physical_bytes: u64,
        initial_logical_bytes: u64,
    ) -> DiskSessionId {
        let generation = 1;
        let session = DiskSessionId {
            slot: self.transfer_ids.len() as u32,
            generation,
        };
        self.physical_bytes_written.push(initial_physical_bytes);
        self.logical_bytes_confirmed.push(initial_logical_bytes);
        self.expected_lens.push(expected_len.unwrap_or(0));
        self.generations.push(generation);
        self.states.push(DiskSessionState::Open);
        self.transfer_ids.push(transfer_id);
        self.flags.expected_len.push(expected_len.is_some());
        self.flags.active.push(true);
        self.flags.committed.push(false);
        self.flags.failed.push(false);
        self.active_positions.push(self.active_rows.len());
        self.active_rows.push(session);
        self.paths.push(DiskSessionPaths {
            part_path: resolved.part_path,
            destination: resolved.destination,
            finalize_strategy: resolved.finalize_strategy,
        });
        self.failure_messages.push(None);
        self.index_by_transfer.insert(transfer_id, session);
        session
    }

    fn add_physical_bytes(&mut self, session: DiskSessionId, bytes: u64) {
        let Some(slot) = self.valid_slot(session) else {
            return;
        };
        self.physical_bytes_written[slot] = self.physical_bytes_written[slot].saturating_add(bytes);
    }

    fn add_logical_bytes(&mut self, session: DiskSessionId, bytes: u64) {
        let Some(slot) = self.valid_slot(session) else {
            return;
        };
        self.logical_bytes_confirmed[slot] =
            self.logical_bytes_confirmed[slot].saturating_add(bytes);
    }

    fn commit(&mut self, session: DiskSessionId) -> io::Result<()> {
        let Some(slot) = self.valid_slot(session) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "stale disk session id",
            ));
        };
        if self.flags.expected_len.get(slot).is_some_and(|bit| *bit)
            && self.logical_bytes_confirmed[slot] < self.expected_lens[slot]
        {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "disk session committed before all logical bytes were confirmed",
            ));
        }

        let paths = &self.paths[slot];
        finalize_part_file(
            &paths.part_path,
            &paths.destination,
            paths.finalize_strategy,
        )?;
        self.move_to_committed(session);
        Ok(())
    }

    fn mark_failed(&mut self, session: DiskSessionId, message: Option<String>) {
        let Some(slot) = self.valid_slot(session) else {
            return;
        };
        self.failure_messages[slot] = message;
        self.move_to_failed(session);
    }

    fn move_to_committed(&mut self, session: DiskSessionId) {
        let slot = session.slot as usize;
        self.leave_active(slot);
        self.flags.committed.set(slot, true);
        self.states[slot] = DiskSessionState::Committed;
        self.committed_rows.push(session);
    }

    fn move_to_failed(&mut self, session: DiskSessionId) {
        let slot = session.slot as usize;
        self.leave_active(slot);
        self.flags.failed.set(slot, true);
        self.states[slot] = DiskSessionState::Failed;
        self.failed_rows.push(session);
    }

    fn leave_active(&mut self, slot: usize) {
        if !self.flags.active.get(slot).is_some_and(|bit| *bit) {
            return;
        }
        self.flags.active.set(slot, false);
        let pos = self.active_positions[slot];
        if pos == NO_INDEX {
            return;
        }
        self.active_rows.swap_remove(pos);
        if let Some(&moved) = self.active_rows.get(pos) {
            self.active_positions[moved.slot as usize] = pos;
        }
        self.active_positions[slot] = NO_INDEX;
    }

    fn valid_slot(&self, session: DiskSessionId) -> Option<usize> {
        let slot = session.slot as usize;
        if self.generations.get(slot).copied()? != session.generation {
            return None;
        }
        Some(slot)
    }

    #[cfg(test)]
    fn index_for(&self, transfer_id: TransferId) -> Option<DiskSessionId> {
        self.index_by_transfer.get(&transfer_id).copied()
    }

    #[cfg(test)]
    fn state(&self, session: DiskSessionId) -> Option<DiskSessionState> {
        let slot = self.valid_slot(session)?;
        self.states.get(slot).copied()
    }

    #[cfg(test)]
    fn physical_bytes_written(&self, session: DiskSessionId) -> Option<u64> {
        let slot = self.valid_slot(session)?;
        self.physical_bytes_written.get(slot).copied()
    }

    #[cfg(test)]
    fn logical_bytes_confirmed(&self, session: DiskSessionId) -> Option<u64> {
        let slot = self.valid_slot(session)?;
        self.logical_bytes_confirmed.get(slot).copied()
    }

    #[cfg(test)]
    fn expected_len(&self, session: DiskSessionId) -> Option<Option<u64>> {
        let slot = self.valid_slot(session)?;
        if !self.flags.expected_len.get(slot).is_some_and(|bit| *bit) {
            return Some(None);
        }
        self.expected_lens.get(slot).copied().map(Some)
    }
}

fn artifact_paths(destination: &Path) -> [PathBuf; 2] {
    [destination.to_path_buf(), part_path_for(destination)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiskWriteRange {
    start: u64,
    end: u64,
}

impl DiskWriteRange {
    pub(crate) fn from_len(start: u64, len: u64) -> Option<Self> {
        let end = start.checked_add(len)?;
        Some(Self { start, end })
    }

    pub(crate) fn start(self) -> u64 {
        self.start
    }

    pub(crate) fn end(self) -> u64 {
        self.end
    }

    pub(crate) fn len(self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiskWriteFailure {
    FatalIo { message: String },
    RetryableIo { message: String },
}

pub(crate) type DiskWriteSender<T> = mpsc::Sender<DiskWriteJob<T>>;

pub(crate) struct DiskWriter<T> {
    jobs: DiskWriteSender<T>,
    handle: JoinHandle<Vec<DiskWriteResult<T>>>,
}

pub(crate) struct DiskWriteJob<T> {
    session: DiskSessionId,
    owner: T,
    offset: u64,
    bytes: Vec<u8>,
    reply: oneshot::Sender<DiskWriteResult<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiskWriteResult<T> {
    Written {
        session: DiskSessionId,
        owner: T,
        range: DiskWriteRange,
    },
    Failed {
        session: DiskSessionId,
        owner: T,
        failure: DiskWriteFailure,
    },
}

impl<T> DiskWriter<T>
where
    T: Copy + Send + 'static,
{
    fn spawn(file: File, lease: DiskSessionLease) -> Self {
        let (jobs, rx) = mpsc::channel(WRITE_JOB_CAPACITY);
        let handle = tokio::task::spawn_blocking(move || run_writer(file, lease, rx));
        Self { jobs, handle }
    }

    pub(crate) fn sender(&self) -> DiskWriteSender<T> {
        self.jobs.clone()
    }

    pub(crate) async fn shutdown(self) -> Vec<DiskWriteResult<T>> {
        drop(self.jobs);
        match self.handle.await {
            Ok(orphaned) => orphaned,
            Err(error) => {
                tracing::error!(?error, "disk writer task failed");
                Vec::new()
            }
        }
    }
}

impl<T> DiskWriteJob<T> {
    pub(crate) fn new(
        session: DiskSessionId,
        owner: T,
        offset: u64,
        bytes: Vec<u8>,
        reply: oneshot::Sender<DiskWriteResult<T>>,
    ) -> Self {
        Self {
            session,
            owner,
            offset,
            bytes,
            reply,
        }
    }
}

fn run_writer<T>(
    mut file: File,
    lease: DiskSessionLease,
    mut jobs: mpsc::Receiver<DiskWriteJob<T>>,
) -> Vec<DiskWriteResult<T>>
where
    T: Copy,
{
    let mut orphaned = Vec::new();
    while let Some(job) = jobs.blocking_recv() {
        let result = write_job(&mut file, &lease, &job);
        if job.reply.send(result.clone()).is_err() {
            orphaned.push(result);
        }
    }
    orphaned
}

fn write_job<T>(
    file: &mut File,
    lease: &DiskSessionLease,
    job: &DiskWriteJob<T>,
) -> DiskWriteResult<T>
where
    T: Copy,
{
    if job.session != lease.session() {
        return DiskWriteResult::Failed {
            session: job.session,
            owner: job.owner,
            failure: DiskWriteFailure::FatalIo {
                message: "write job belongs to another disk session".to_string(),
            },
        };
    }

    let Some(range) = DiskWriteRange::from_len(job.offset, job.bytes.len() as u64) else {
        return DiskWriteResult::Failed {
            session: job.session,
            owner: job.owner,
            failure: DiskWriteFailure::FatalIo {
                message: "write range overflow".to_string(),
            },
        };
    };

    match write_all_at(file, &job.bytes, job.offset) {
        Ok(()) => {
            lease.record_physical(range.len());
            tracing::trace!(
                session = ?job.session,
                bytes = range.len(),
                offset = job.offset,
                "disk write confirmed"
            );
            DiskWriteResult::Written {
                session: job.session,
                owner: job.owner,
                range,
            }
        }
        Err(error) => {
            tracing::trace!(
                session = ?job.session,
                offset = job.offset,
                error = %error,
                "disk write failed"
            );
            DiskWriteResult::Failed {
                session: job.session,
                owner: job.owner,
                failure: failure_from_io(error),
            }
        }
    }
}

fn failure_from_io(error: io::Error) -> DiskWriteFailure {
    match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::PermissionDenied => DiskWriteFailure::FatalIo {
            message: error.to_string(),
        },
        _ => DiskWriteFailure::RetryableIo {
            message: error.to_string(),
        },
    }
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    while !buf.is_empty() {
        let written = file.seek_write(buf, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        offset = offset.saturating_add(written as u64);
        buf = &buf[written..];
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn resolved(dir: &tempfile::TempDir, name: &str) -> ResolvedDestination {
        ResolvedDestination {
            destination: dir.path().join(name),
            part_path: dir.path().join(format!("{name}.part")),
            finalize_strategy: FinalizeStrategy::MoveNoReplace,
        }
    }

    fn replace_resolved(dir: &tempfile::TempDir, name: &str) -> ResolvedDestination {
        ResolvedDestination {
            destination: dir.path().join(name),
            part_path: dir.path().join(format!("{name}.part")),
            finalize_strategy: FinalizeStrategy::ReplaceExisting,
        }
    }

    #[test]
    fn disk_sessions_store_paths_and_state_in_dense_tables() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = DiskSessions::new();

        let first = sessions.insert(TransferId(7), resolved(&dir, "first.bin"), Some(8), 0, 0);
        let second = sessions.insert(TransferId(8), resolved(&dir, "second.bin"), None, 0, 0);

        assert_eq!(first.slot, 0);
        assert_eq!(second.slot, 1);
        assert_eq!(sessions.expected_len(first), Some(Some(8)));
        assert_eq!(sessions.expected_len(second), Some(None));
        assert_eq!(sessions.index_for(TransferId(7)), Some(first));
        assert_eq!(sessions.state(first), Some(DiskSessionState::Open));

        sessions.add_physical_bytes(first, 4);
        sessions.add_logical_bytes(first, 3);
        assert_eq!(sessions.physical_bytes_written(first), Some(4));
        assert_eq!(sessions.logical_bytes_confirmed(first), Some(3));
    }

    #[test]
    fn disk_sessions_reject_stale_generation_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = DiskSessions::new();
        let session = sessions.insert(TransferId(11), resolved(&dir, "stale.bin"), None, 0, 0);
        let stale = DiskSessionId {
            slot: session.slot,
            generation: session.generation + 1,
        };

        sessions.add_physical_bytes(stale, 8);
        sessions.add_logical_bytes(stale, 8);

        assert_eq!(sessions.physical_bytes_written(session), Some(0));
        assert_eq!(sessions.logical_bytes_confirmed(session), Some(0));
        assert_eq!(sessions.physical_bytes_written(stale), None);
        assert_eq!(
            sessions.commit(stale).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[tokio::test]
    async fn disk_writer_writes_at_offsets_and_reports_range() {
        let dir = tempfile::tempdir().unwrap();
        let handle = DiskHandle::new();
        let lease = handle
            .create_new(TransferId(1), resolved(&dir, "part.bin"), Some(8))
            .unwrap();
        let session = lease.session();
        let (session_lease, writer) = lease.split_for_writes();
        let jobs = writer.sender();
        let (reply_tx, reply_rx) = oneshot::channel();

        jobs.send(DiskWriteJob::new(
            session,
            TransferId(1),
            4,
            b"efgh".to_vec(),
            reply_tx,
        ))
        .await
        .unwrap();

        assert_eq!(
            reply_rx.await.unwrap(),
            DiskWriteResult::Written {
                session,
                owner: TransferId(1),
                range: DiskWriteRange::from_len(4, 4).unwrap(),
            }
        );
        drop(jobs);
        assert!(writer.shutdown().await.is_empty());
        assert_eq!(
            handle
                .sessions
                .lock()
                .unwrap()
                .physical_bytes_written(session),
            Some(4)
        );

        session_lease.confirm_logical(4);
        assert_eq!(
            handle
                .sessions
                .lock()
                .unwrap()
                .logical_bytes_confirmed(session),
            Some(4)
        );

        let mut written = Vec::new();
        File::open(dir.path().join("part.bin.part"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(&written[4..8], b"efgh");
    }

    #[test]
    fn logical_confirmation_does_not_double_count_duplicate_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = DiskSessions::new();
        let session = sessions.insert(TransferId(9), resolved(&dir, "dupe.bin"), Some(8), 0, 0);

        sessions.add_physical_bytes(session, 8);
        sessions.add_physical_bytes(session, 8);
        sessions.add_logical_bytes(session, 8);
        sessions.add_logical_bytes(session, 0);

        assert_eq!(sessions.physical_bytes_written(session), Some(16));
        assert_eq!(sessions.logical_bytes_confirmed(session), Some(8));
    }

    #[test]
    fn commit_refuses_incomplete_logical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = DiskSessions::new();
        let session = sessions.insert(TransferId(10), resolved(&dir, "short.bin"), Some(8), 8, 4);

        let error = sessions.commit(session).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn commit_finalizes_move_strategy() {
        let dir = tempfile::tempdir().unwrap();
        let handle = DiskHandle::new();
        let lease = handle
            .create_new(TransferId(12), resolved(&dir, "done.bin"), Some(4))
            .unwrap();
        let session = lease.session();
        let session_lease = lease.into_session();
        write_all_at(
            &File::options()
                .write(true)
                .open(dir.path().join("done.bin.part"))
                .unwrap(),
            b"done",
            0,
        )
        .unwrap();
        session_lease.confirm_logical(4);

        session_lease.commit().unwrap();

        let mut written = String::new();
        File::open(dir.path().join("done.bin"))
            .unwrap()
            .read_to_string(&mut written)
            .unwrap();
        assert_eq!(written, "done");
        assert_eq!(
            handle.sessions.lock().unwrap().state(session),
            Some(DiskSessionState::Committed)
        );
    }

    #[test]
    fn commit_finalizes_replace_existing_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("replace.bin"), b"old").unwrap();
        std::fs::write(dir.path().join("replace.bin.part"), b"new").unwrap();
        let mut sessions = DiskSessions::new();
        let session = sessions.insert(
            TransferId(13),
            replace_resolved(&dir, "replace.bin"),
            Some(3),
            3,
            3,
        );

        sessions.commit(session).unwrap();

        assert_eq!(
            std::fs::read(dir.path().join("replace.bin")).unwrap(),
            b"new"
        );
        assert!(!dir.path().join("replace.bin.part").exists());
        assert_eq!(sessions.state(session), Some(DiskSessionState::Committed));
    }

    #[test]
    fn delete_removes_final_and_part_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let handle = DiskHandle::new();
        let destination = dir.path().join("delete.bin");
        std::fs::write(&destination, b"final").unwrap();
        std::fs::write(part_path_for(&destination), b"part").unwrap();

        assert_eq!(
            handle.delete_artifacts(&destination),
            ArtifactState::Deleted
        );
        assert!(!destination.exists());
        assert!(!part_path_for(&destination).exists());
    }

    #[test]
    fn stale_partial_classification_is_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        let handle = DiskHandle::new();
        let destination = dir.path().join("classify.bin");
        let part = part_path_for(&destination);
        std::fs::write(&part, b"part").unwrap();

        assert_eq!(
            handle.classify_artifacts(&destination),
            DiskArtifactClassification::PartOnly
        );
        assert!(part.exists());
    }

    #[tokio::test]
    async fn disk_writer_returns_orphaned_result_when_worker_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let handle = DiskHandle::new();
        let lease = handle
            .create_new(TransferId(2), resolved(&dir, "orphan.bin"), None)
            .unwrap();
        let session = lease.session();
        let (_session_lease, writer) = lease.split_for_writes();
        let jobs = writer.sender();
        let (reply_tx, reply_rx) = oneshot::channel();

        jobs.send(DiskWriteJob::new(
            session,
            TransferId(2),
            0,
            b"abcd".to_vec(),
            reply_tx,
        ))
        .await
        .unwrap();
        drop(reply_rx);
        drop(jobs);

        assert_eq!(
            writer.shutdown().await,
            vec![DiskWriteResult::Written {
                session,
                owner: TransferId(2),
                range: DiskWriteRange::from_len(0, 4).unwrap(),
            }]
        );
    }

    #[tokio::test]
    async fn disk_writer_shuts_down_when_sender_closes() {
        let dir = tempfile::tempdir().unwrap();
        let lease = DiskHandle::new()
            .create_new(TransferId(3), resolved(&dir, "closed.bin"), None)
            .unwrap();
        let (_session_lease, writer) = lease.split_for_writes::<TransferId>();
        let jobs = writer.sender();

        drop(jobs);

        assert!(writer.shutdown().await.is_empty());
    }

    #[test]
    fn disk_writer_maps_disk_full_and_permission_errors_to_fatal() {
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::PermissionDenied] {
            assert!(matches!(
                failure_from_io(io::Error::from(kind)),
                DiskWriteFailure::FatalIo { .. }
            ));
        }
    }

    #[test]
    fn disk_writer_maps_other_io_errors_to_retryable() {
        assert!(matches!(
            failure_from_io(io::Error::from(io::ErrorKind::Interrupted)),
            DiskWriteFailure::RetryableIo { .. }
        ));
    }
}
