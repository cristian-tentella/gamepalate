use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::Path,
};

const FILE: &str = "update.lock";

#[derive(Debug)]
pub(crate) struct UpdateLock {
    _file: File,
}

#[derive(Debug)]
pub(crate) enum LockError {
    Taken,
    Io(io::Error),
}

impl UpdateLock {
    pub(crate) fn take(data_dir: &Path) -> Result<UpdateLock, LockError> {
        fs::create_dir_all(data_dir).map_err(LockError::Io)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(data_dir.join(FILE))
            .map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => Ok(UpdateLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::Taken),
            Err(TryLockError::Error(error)) => Err(LockError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_is_refused_until_the_first_is_dropped() {
        let data = tempfile::tempdir().unwrap();

        let first = UpdateLock::take(data.path()).unwrap();
        assert!(matches!(
            UpdateLock::take(data.path()),
            Err(LockError::Taken)
        ));
        drop(first);

        assert!(UpdateLock::take(data.path()).is_ok());
    }
}
