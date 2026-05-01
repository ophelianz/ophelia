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

use std::fs;
use std::path::{Path, PathBuf};

fn gui_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read_source_files(dir: &Path, files: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            read_source_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let body = fs::read_to_string(&path).unwrap();
            files.push((path, body));
        }
    }
}

#[test]
fn gui_source_no_longer_references_stale_engine_bridge() {
    let mut files = Vec::new();
    read_source_files(&gui_src_dir(), &mut files);

    for (path, body) in files {
        assert!(
            !body.contains("EngineBridge") && !body.contains("engine_bridge"),
            "{} still references the stale GUI engine bridge",
            path.display()
        );
    }
}

#[test]
fn app_model_does_not_own_backend_runtime_objects() {
    let app_rs = fs::read_to_string(gui_src_dir().join("app.rs")).unwrap();

    for forbidden in [
        "EngineBridge",
        "DownloadEngine",
        "StateBootstrap",
        "DbWorkerHandle",
        "HistoryReader",
    ] {
        assert!(
            !app_rs.contains(forbidden),
            "app.rs should not own or construct {forbidden}"
        );
    }
}
