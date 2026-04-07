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

#![allow(dead_code)]

//! Shared platform-specific process I/O counters.
//!
//! These counters are used for lightweight app-session disk I/O metrics in the
//! Transfers stats bar. They are intentionally process-wide rather than
//! per-download.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessIoCounters {
    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub(crate) fn sample_process_io_counters() -> Option<ProcessIoCounters> {
    sample_process_io_counters_impl()
}

#[cfg(target_os = "linux")]
fn sample_process_io_counters_impl() -> Option<ProcessIoCounters> {
    let content = std::fs::read_to_string("/proc/self/io").ok()?;
    parse_linux_proc_self_io(&content)
}

#[cfg(target_os = "macos")]
fn sample_process_io_counters_impl() -> Option<ProcessIoCounters> {
    use std::mem::MaybeUninit;

    let mut info = MaybeUninit::<libc::rusage_info_v2>::uninit();
    let result = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr() as *mut libc::rusage_info_t,
        )
    };
    if result != 0 {
        return None;
    }
    let info = unsafe { info.assume_init() };

    Some(ProcessIoCounters {
        read_bytes: info.ri_diskio_bytesread,
        write_bytes: info.ri_diskio_byteswritten,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sample_process_io_counters_impl() -> Option<ProcessIoCounters> {
    None
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_self_io(content: &str) -> Option<ProcessIoCounters> {
    let mut read_bytes = None;
    let mut write_bytes = None;

    for line in content.lines() {
        let (key, value) = line.split_once(':')?;
        let value = value.trim().parse::<u64>().ok()?;
        match key.trim() {
            "read_bytes" => read_bytes = Some(value),
            "write_bytes" => write_bytes = Some(value),
            _ => {}
        }
    }

    Some(ProcessIoCounters {
        read_bytes: read_bytes?,
        write_bytes: write_bytes?,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_proc_self_io_counters() {
        let counters = super::parse_linux_proc_self_io(
            "rchar: 123\nwchar: 456\nsyscr: 1\nsyscw: 2\nread_bytes: 789\nwrite_bytes: 321\ncancelled_write_bytes: 0\n",
        )
        .unwrap();

        assert_eq!(
            counters,
            super::ProcessIoCounters {
                read_bytes: 789,
                write_bytes: 321,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_linux_proc_self_io_fields_return_none() {
        assert!(super::parse_linux_proc_self_io("read_bytes: 12\n").is_none());
    }
}
