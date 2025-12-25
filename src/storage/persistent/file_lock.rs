//! File locking for single-process database access.
//!
//! This module provides cross-platform exclusive file locking to ensure
//! only one process can access the database at a time.
//!
//! # Safety
//! - Lock is released when FileLock is dropped
//! - Lock file is created if it doesn't exist
//! - Non-blocking lock attempt with clear error on failure

use std::fs::{File, OpenOptions};
use std::io::{Error as IoError, ErrorKind, Result as IoResult};
use std::path::{Path, PathBuf};

/// Exclusive file lock for database access.
///
/// The lock is held for the lifetime of this struct and automatically
/// released when dropped.
#[derive(Debug)]
pub struct FileLock {
    _file: File,
    path: PathBuf,
}

impl FileLock {
    /// Attempt to acquire an exclusive lock on the database directory.
    ///
    /// # Arguments
    /// * `dir` - The database directory to lock
    ///
    /// # Returns
    /// A `FileLock` if successful, or an error if another process holds the lock.
    ///
    /// # Errors
    /// - `ErrorKind::WouldBlock` if another process holds the lock
    /// - `ErrorKind::PermissionDenied` if we don't have write access
    pub fn acquire(dir: &Path) -> IoResult<Self> {
        let lock_path = dir.join(".lock");
        
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        
        Self::try_lock(&file)?;
        
        Ok(Self {
            _file: file,
            path: lock_path,
        })
    }
    
    /// Returns the path to the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    
    #[cfg(unix)]
    fn try_lock(file: &File) -> IoResult<()> {
        use std::os::unix::io::AsRawFd;
        
        // Use non-blocking exclusive lock
        let fd = file.as_raw_fd();
        let result = unsafe {
            libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB)
        };
        
        if result != 0 {
            let errno = std::io::Error::last_os_error();
            if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(IoError::new(
                    ErrorKind::WouldBlock,
                    "database is locked by another process",
                ));
            }
            return Err(errno);
        }
        
        Ok(())
    }
    
    #[cfg(windows)]
    fn try_lock(file: &File) -> IoResult<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        use windows_sys::Win32::Foundation::HANDLE;
        
        let handle = file.as_raw_handle() as HANDLE;
        let result = unsafe {
            let mut overlapped = std::mem::zeroed::<windows_sys::Win32::System::IO::OVERLAPPED>();
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        
        if result == 0 {
            let err = std::io::Error::last_os_error();
            return Err(IoError::new(
                ErrorKind::WouldBlock,
                format!("database is locked by another process: {}", err),
            ));
        }
        
        Ok(())
    }
    
    #[cfg(not(any(unix, windows)))]
    fn try_lock(_file: &File) -> IoResult<()> {
        #[cfg(feature = "allow_no_lock")]
        {
            eprintln!("warning: file locking not supported on this platform; proceeding without lock");
            Ok(())
        }

        #[cfg(not(feature = "allow_no_lock"))]
        {
            Err(IoError::new(
                ErrorKind::Unsupported,
                "file locking not supported on this platform",
            ))
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Lock is automatically released when file is closed
        // No explicit unlock needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_lock_acquire_release() {
        let dir = tempdir().unwrap();
        
        {
            let lock = FileLock::acquire(dir.path()).unwrap();
            assert!(lock.path().exists());
        }
        // Lock released on drop
    }
    
    #[test]
    fn test_lock_prevents_double_acquire() {
        let dir = tempdir().unwrap();
        
        let _lock1 = FileLock::acquire(dir.path()).unwrap();
        
        // Second acquire should fail
        let result = FileLock::acquire(dir.path());
        assert!(result.is_err());
        
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
    }
}
