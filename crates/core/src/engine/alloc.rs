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

//! Preallocates space for downloads
//!
//! Lets a chunked download fail early if there is not enough disk space

pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
    imp::preallocate(file, size)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn off_t_len(size: u64) -> std::io::Result<libc::off_t> {
    size.try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "requested allocation size exceeds platform off_t",
        )
    })
}

#[cfg(target_os = "windows")]
fn i64_len(size: u64) -> std::io::Result<i64> {
    size.try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "requested allocation size exceeds Windows allocation size",
        )
    })
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        use std::io;
        use std::os::unix::io::AsRawFd;

        let length = super::off_t_len(size)?;
        loop {
            // Do not use FALLOC_FL_KEEP_SIZE because the visible file size should change
            let ret = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, length) };
            if ret == 0 {
                return Ok(());
            }

            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if is_unsupported_preallocation(&error) {
                return file.set_len(size);
            }
            return Err(error);
        }
    }

    fn is_unsupported_preallocation(error: &std::io::Error) -> bool {
        matches!(error.raw_os_error(), Some(code) if code == libc::EOPNOTSUPP || code == libc::ENOSYS)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        use std::os::unix::io::AsRawFd;

        let fd = file.as_raw_fd();
        let length = super::off_t_len(size)?;

        if f_preallocate(fd, libc::F_ALLOCATECONTIG, length).is_err() {
            match f_preallocate(fd, libc::F_ALLOCATEALL, length) {
                Ok(()) => {}
                Err(error) if is_unsupported_preallocation(&error) => return file.set_len(size),
                Err(error) => return Err(error),
            }
        }

        // F_PREALLOCATE reserves blocks but doesn't update the file size
        file.set_len(size)
    }

    fn f_preallocate(
        fd: std::os::unix::io::RawFd,
        flags: libc::c_uint,
        length: libc::off_t,
    ) -> std::io::Result<()> {
        loop {
            let fst = libc::fstore_t {
                fst_flags: flags,
                fst_posmode: libc::F_PEOFPOSMODE,
                fst_offset: 0,
                fst_length: length,
                fst_bytesalloc: 0,
            };
            if unsafe { libc::fcntl(fd, libc::F_PREALLOCATE, &fst) } != -1 {
                return Ok(());
            }

            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn is_unsupported_preallocation(error: &std::io::Error) -> bool {
        matches!(error.raw_os_error(), Some(code) if code == libc::ENOTSUP || code == libc::EOPNOTSUPP || code == libc::ENOSYS)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        use std::mem::size_of;
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ALLOCATION_INFO, FileAllocationInfo, SetFileInformationByHandle,
        };

        let allocation_size = super::i64_len(size)?;
        let info = FILE_ALLOCATION_INFO {
            AllocationSize: allocation_size,
        };

        let ret = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileAllocationInfo,
                &info as *const _ as *const core::ffi::c_void,
                size_of::<FILE_ALLOCATION_INFO>() as u32,
            )
        };
        if ret == 0 {
            return Err(std::io::Error::last_os_error());
        }

        file.set_len(size)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod imp {
    pub fn preallocate(file: &std::fs::File, size: u64) -> std::io::Result<()> {
        file.set_len(size)
    }
}
