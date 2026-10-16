//! All processes serialize rotation and open the active file only while holding
//! the lock. No process keeps writing to an inode that another process renamed.
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

const FILE_BYTES: usize = 2 * 1024 * 1024;
const ARCHIVES: usize = 3;

#[derive(Default)]
struct Progress {
    read: u64,
    completed: u64,
    error: Option<io::Error>,
}

/// A completion barrier for stderr bytes already sent to the rotating log.
pub struct StderrLog {
    reader: UnixStream,
    progress: Arc<(Mutex<Progress>, Condvar)>,
}

impl StderrLog {
    /// Wait for bytes queued before this call to reach the file, not for fsync.
    /// Later writes remain supported. Disk/lock stalls cannot hold up the caller
    /// beyond the timeout; errors since redirection are reported, not discarded.
    pub fn flush(&self, timeout: Duration) -> io::Result<()> {
        let started = Instant::now();
        let (progress, changed) = &*self.progress;
        let state = progress.lock().unwrap();
        let mut queued: libc::c_int = 0;
        // The worker updates `read` under this same lock, so a byte is either
        // counted there or still queued in the socket, never between the two.
        if unsafe { libc::ioctl(self.reader.as_raw_fd(), libc::FIONREAD, &mut queued) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let target = state.read + queued as u64;
        let (state, _) = changed
            .wait_timeout_while(state, timeout.saturating_sub(started.elapsed()), |state| {
                state.completed < target
            })
            .unwrap();
        if state.completed < target {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "log flush timed out",
            ));
        }
        if let Some(error) = &state.error {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        Ok(())
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
}

fn archive(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

fn append(path: &Path, lock: &File, mut bytes: &[u8], limit: usize) -> io::Result<()> {
    lock.lock_exclusive()?;
    let result = (|| {
        loop {
            let mut file = private_file(path)?;
            let mut size = file.metadata()?.len() as usize;
            // Migrate a pre-rotation log by retaining its most recent bounded tail.
            if size > limit {
                file.seek(SeekFrom::End(-(limit as i64)))?;
                let mut tail = vec![0; limit];
                file.read_exact(&mut tail)?;
                file.set_len(0)?;
                file.rewind()?;
                file.write_all(&tail)?;
                size = limit;
            }
            if bytes.is_empty() {
                return Ok(());
            }
            if size == limit {
                drop(file);
                for index in (1..=ARCHIVES).rev() {
                    let source = if index == 1 {
                        path.to_path_buf()
                    } else {
                        archive(path, index - 1)
                    };
                    match fs::rename(source, archive(path, index)) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                file = private_file(path)?;
                size = 0;
            }
            let take = bytes.len().min(limit - size);
            file.seek(SeekFrom::End(0))?;
            file.write_all(&bytes[..take])?;
            bytes = &bytes[take..];
            if bytes.is_empty() {
                return Ok(());
            }
        }
    })();
    let unlocked = FileExt::unlock(lock);
    result.and(unlocked)
}

fn start(path: &Path) -> io::Result<(StderrLog, UnixStream)> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    private_file(path)?;
    let lock = private_file(&path.with_extension("log.lock"))?;
    append(path, &lock, &[], FILE_BYTES)?;
    let (mut reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    let log = StderrLog {
        reader: reader.try_clone()?,
        progress: Arc::default(),
    };
    let progress = log.progress.clone();
    let path = path.to_path_buf();
    // dup2 is only done after the reader exists. The kernel socket buffer and this
    // fixed buffer bound queued log bytes even when the disk is slow.
    std::thread::Builder::new()
        .name("verge-log".into())
        .spawn(move || {
            let mut buffer = [0; 8192];
            let (progress, changed) = &*progress;
            loop {
                let mut ready = libc::pollfd {
                    fd: reader.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut ready, 1, -1) } == -1 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    progress.lock().unwrap().error.get_or_insert(error);
                    changed.notify_all();
                    break;
                }
                // Only the nonblocking read and its counter share the barrier
                // lock. File IO and cross-process locking must stay outside it.
                let result = {
                    let mut state = progress.lock().unwrap();
                    let result = reader.read(&mut buffer);
                    if let Ok(count) = result {
                        state.read += count as u64;
                    }
                    result
                };
                match result {
                    Ok(0) => break,
                    Ok(count) => {
                        let result = append(&path, &lock, &buffer[..count], FILE_BYTES);
                        let mut state = progress.lock().unwrap();
                        state.completed += count as u64;
                        if let Err(error) = result {
                            state.error.get_or_insert(error);
                        }
                        changed.notify_all();
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                        ) => {}
                    Err(error) => {
                        progress.lock().unwrap().error.get_or_insert(error);
                        changed.notify_all();
                        break;
                    }
                }
            }
        })?;
    Ok((log, writer))
}

pub(super) fn redirect(path: &Path) -> io::Result<StderrLog> {
    let (log, writer) = start(path)?;
    if unsafe { libc::dup2(writer.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_waits_for_queued_bytes_and_supports_later_writes() {
        let dir = directory("flush");
        let path = dir.join("verge.log");
        let (log, mut writer) = start(&path).unwrap();
        log.flush(Duration::ZERO).unwrap();
        // Multiple worker buffers, followed by another write after the barrier.
        let bytes = vec![b'x'; 32 * 1024];
        writer.write_all(&bytes).unwrap();
        log.flush(Duration::from_secs(2)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        writer.write_all(b"final shutdown log\n").unwrap();
        log.flush(Duration::from_secs(2)).unwrap();
        assert!(fs::read(&path).unwrap().ends_with(b"final shutdown log\n"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flush_waits_for_a_contended_file_lock() {
        let dir = directory("flush-delayed");
        let path = dir.join("verge.log");
        let (log, mut writer) = start(&path).unwrap();
        let lock = private_file(&path.with_extension("log.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        writer.write_all(b"delayed shutdown log\n").unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            FileExt::unlock(&lock).unwrap();
        });
        log.flush(Duration::from_secs(2)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"delayed shutdown log\n");
        releaser.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flush_timeout_is_bounded_and_can_be_retried() {
        let dir = directory("flush-timeout");
        let path = dir.join("verge.log");
        let (log, mut writer) = start(&path).unwrap();
        let lock = private_file(&path.with_extension("log.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        writer.write_all(b"blocked shutdown log\n").unwrap();
        // Force the in-flight case: the socket is empty but append is blocked.
        let deadline = Instant::now() + Duration::from_secs(2);
        while log.progress.0.lock().unwrap().read == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let started = Instant::now();
        assert_eq!(
            log.flush(Duration::from_millis(50)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(fs::read(&path).unwrap().is_empty());
        FileExt::unlock(&lock).unwrap();
        log.flush(Duration::from_secs(2)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"blocked shutdown log\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flush_reports_write_failures_instead_of_success() {
        let dir = directory("flush-error");
        let path = dir.join("verge.log");
        let (log, mut writer) = start(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        writer.write_all(b"unwritable shutdown log\n").unwrap();
        let error = log.flush(Duration::from_secs(2)).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::TimedOut);
        // A later successful append must not hide previously lost bytes.
        fs::remove_dir(&path).unwrap();
        writer.write_all(b"recovered writer\n").unwrap();
        // Report the earlier error only after attempting the new queued bytes.
        assert!(log.flush(Duration::from_secs(2)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"recovered writer\n");
        assert!(log.flush(Duration::ZERO).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_writers_and_flushes_preserve_all_bytes() {
        let dir = directory("flush-concurrent");
        let path = dir.join("verge.log");
        let (log, writer) = start(&path).unwrap();
        let log = Arc::new(log);
        let workers: Vec<_> = (0..4)
            .map(|id| {
                let log = log.clone();
                let mut writer = writer.try_clone().unwrap();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        writer.write_all(&[id; 1024]).unwrap();
                        log.flush(Duration::from_secs(2)).unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        for id in 0..4 {
            assert_eq!(bytes.iter().filter(|byte| **byte == id).count(), 20 * 1024);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn redirected_stderr_is_flushed_before_immediate_process_exit() {
        const CHILD: &str = "VERGE_TEST_LOG_EXIT";
        if let Some(path) = std::env::var_os(CHILD) {
            let log = redirect(Path::new(&path)).unwrap();
            eprintln!("[verge] [error] Shutdown cleanup failed: test fixture");
            eprintln!("[verge] [info] Shutdown backend: success=false");
            eprintln!("[verge] [info] Shutdown ipc: elapsed_ms=0");
            if log.flush(Duration::from_secs(2)).is_err() {
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        let dir = directory("flush-process");
        let path = dir.join("verge.log");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::rotating_log::tests::redirected_stderr_is_flushed_before_immediate_process_exit",
                "--nocapture",
            ])
            .env(CHILD, &path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Shutdown cleanup failed: test fixture"));
        assert!(text.contains("Shutdown backend: success=false"));
        assert!(text.contains("Shutdown ipc: elapsed_ms=0"));
        fs::remove_dir_all(dir).unwrap();
    }

    fn directory(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("verge-logs-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn rotation_bounds_files_and_keeps_latest_bytes_in_order() {
        let dir = directory("bounds");
        let path = dir.join("verge.log");
        let lock = private_file(&dir.join("lock")).unwrap();
        fs::write(&path, vec![b'x'; 1000]).unwrap();
        append(&path, &lock, &(0..200).collect::<Vec<u8>>(), 32).unwrap();
        let mut all = Vec::new();
        for i in (1..=ARCHIVES).rev() {
            let content = fs::read(archive(&path, i)).unwrap();
            assert!(content.len() <= 32);
            all.extend(content);
        }
        let current = fs::read(&path).unwrap();
        assert!(current.len() <= 32);
        all.extend(current);
        assert_eq!(all, (96..200).collect::<Vec<u8>>());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn independent_writers_follow_rotation_without_losing_bytes() {
        let dir = directory("writers");
        let path = dir.join("verge.log");
        let workers: Vec<_> = (0..4)
            .map(|id| {
                let path = path.clone();
                let lock_path = dir.join("lock");
                std::thread::spawn(move || {
                    let lock = private_file(&lock_path).unwrap();
                    for _ in 0..40 {
                        append(&path, &lock, &[id; 10], 512).unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let mut bytes = fs::read(&path).unwrap();
        for i in 1..=ARCHIVES {
            bytes.extend(fs::read(archive(&path, i)).unwrap());
        }
        for id in 0..4 {
            assert_eq!(bytes.iter().filter(|b| **b == id).count(), 400);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
