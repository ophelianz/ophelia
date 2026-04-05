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

//! Platform-specific file preallocation.
//!
//! `preallocate` reserves `size` bytes on disk before any writes begin.
//! True preallocation avoids fragmentation and lets the OS fail immediately on
//! ENOSPC rather than partway through a multi-GB download.
//!
//! - Linux:   fallocate(2)          - contiguous, no zeroing, instant
//! - macOS:   fcntl(F_PREALLOCATE)  - contiguous hint, then set_len for size
//! - Other:   set_len (ftruncate)   - sparse file, always works

pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
    imp::preallocate(file, size)
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        use std::os::unix::io::AsRawFd;
        // FALLOC_FL_KEEP_SIZE is not set, we want the file size updated too
        let ret = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, size as libc::off_t) };
        if ret == 0 {
            Ok(())
        } else {
            // fallocate fails on tmpfs, NFS, FAT32. Fall back to ftruncate.
            file.set_len(size)
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        // Ask for contiguous blocks first; fall back to non-contiguous.
        let mut fst = libc::fstore_t {
            fst_flags: libc::F_ALLOCATECONTIG,
            fst_posmode: libc::F_PEOFPOSMODE,
            fst_offset: 0,
            fst_length: size as libc::off_t,
            fst_bytesalloc: 0,
        };
        if unsafe { libc::fcntl(fd, libc::F_PREALLOCATE, &fst) } == -1 {
            fst.fst_flags = libc::F_ALLOCATEALL;
            let _ = unsafe { libc::fcntl(fd, libc::F_PREALLOCATE, &fst) };
        }
        // F_PREALLOCATE reserves blocks but does not update the file size.
        file.set_len(size)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        file.set_len(size)
    }
}
