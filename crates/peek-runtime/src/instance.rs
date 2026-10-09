//! OS releases the lock when the process exits, including crashes.
pub fn acquire(path: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        // Another instance holds the lock.
        Err(e) if is_already_locked(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Whether a failed `try_lock_exclusive` means "someone else holds it" rather
/// than a genuine error.
///
/// Unix reports `WouldBlock`. Windows reports raw Win32 errors instead, so a
/// second instance used to surface as an error there rather than the quiet
/// "already running" path.
fn is_already_locked(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        const ERROR_SHARING_VIOLATION: i32 = 32;
        const ERROR_LOCK_VIOLATION: i32 = 33;
        matches!(
            error.raw_os_error(),
            Some(code) if code == ERROR_SHARING_VIOLATION || code == ERROR_LOCK_VIOLATION
        )
    }
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_denied_then_lock_released() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("instance.lock");
        let first = acquire(&path).unwrap().unwrap();
        assert!(acquire(&path).unwrap().is_none());
        drop(first);
        assert!(acquire(&path).unwrap().is_some());
    }

    #[test]
    fn already_locked_covers_both_platforms_without_swallowing_real_errors() {
        assert!(is_already_locked(&std::io::Error::from(
            std::io::ErrorKind::WouldBlock
        )));
        #[cfg(windows)]
        {
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            assert!(is_already_locked(&std::io::Error::from_raw_os_error(32)));
            assert!(is_already_locked(&std::io::Error::from_raw_os_error(33)));
        }
        assert!(!is_already_locked(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));
        #[cfg(windows)]
        assert!(!is_already_locked(&std::io::Error::from_raw_os_error(5)));
    }
}
