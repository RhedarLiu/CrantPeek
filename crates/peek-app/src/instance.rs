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
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn second_instance_denied_then_lock_released() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("instance.lock");
        let first = super::acquire(&path).unwrap().unwrap();
        assert!(super::acquire(&path).unwrap().is_none());
        drop(first);
        assert!(super::acquire(&path).unwrap().is_some());
    }
}
