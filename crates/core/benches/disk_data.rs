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

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use ophelia::engine::TransferId;
use tempfile::TempDir;

mod engine {
    pub(crate) mod alloc {
        pub use ophelia::engine::alloc::preallocate;
    }

    pub(crate) mod destination {
        pub use ophelia::engine::destination::*;
    }

    pub(crate) mod types {
        pub use ophelia::engine::{ArtifactState, TransferId};
    }
}

#[allow(dead_code, unused_imports)]
#[path = "../src/disk.rs"]
mod disk;

use disk::DiskHandle;
use engine::destination::{FinalizeStrategy, ResolvedDestination, part_path_for};

fn resolved(dir: &TempDir, name: &str) -> ResolvedDestination {
    ResolvedDestination {
        destination: dir.path().join(name),
        part_path: dir.path().join(format!("{name}.part")),
        finalize_strategy: FinalizeStrategy::MoveNoReplace,
    }
}

fn bench_disk_session_create(c: &mut Criterion) {
    c.bench_function("disk_session_create_new", |bench| {
        bench.iter_batched(
            tempfile::tempdir,
            |dir| {
                let dir = dir.unwrap();
                DiskHandle::new()
                    .create_new(TransferId(1), resolved(&dir, "file.bin"), Some(1024))
                    .unwrap()
                    .session()
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_disk_logical_confirmation(c: &mut Criterion) {
    c.bench_function("disk_logical_confirm_4096", |bench| {
        bench.iter_batched(
            tempfile::tempdir,
            |dir| {
                let dir = dir.unwrap();
                let session = DiskHandle::new()
                    .create_new(TransferId(2), resolved(&dir, "file.bin"), Some(4096))
                    .unwrap()
                    .into_session();
                for _ in 0..4096 {
                    session.confirm_logical(1);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_disk_commit_incomplete_check(c: &mut Criterion) {
    c.bench_function("disk_commit_incomplete_check", |bench| {
        bench.iter_batched(
            tempfile::tempdir,
            |dir| {
                let dir = dir.unwrap();
                DiskHandle::new()
                    .create_new(TransferId(3), resolved(&dir, "file.bin"), Some(1024))
                    .unwrap()
                    .into_session()
                    .commit()
                    .unwrap_err()
                    .kind()
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_disk_artifact_classification(c: &mut Criterion) {
    c.bench_function("disk_artifact_classification", |bench| {
        bench.iter_batched(
            || {
                let dir = tempfile::tempdir().unwrap();
                let destination = dir.path().join("file.bin");
                std::fs::write(&destination, b"final").unwrap();
                std::fs::write(part_path_for(&destination), b"part").unwrap();
                (dir, destination)
            },
            |(_dir, destination)| DiskHandle::new().classify_artifacts(&destination),
            BatchSize::SmallInput,
        );
    });
}

fn bench_disk_delete_artifacts(c: &mut Criterion) {
    c.bench_function("disk_delete_artifacts", |bench| {
        bench.iter_batched(
            || {
                let dir = tempfile::tempdir().unwrap();
                let destination = dir.path().join("file.bin");
                std::fs::write(&destination, b"final").unwrap();
                std::fs::write(part_path_for(&destination), b"part").unwrap();
                (dir, destination)
            },
            |(_dir, destination)| DiskHandle::new().delete_artifacts(&destination),
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_disk_session_create,
    bench_disk_logical_confirmation,
    bench_disk_commit_incomplete_check,
    bench_disk_artifact_classification,
    bench_disk_delete_artifacts
);
criterion_main!(benches);
