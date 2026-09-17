//! All processes serialize rotation and open the active file only while holding
//! the lock. No process keeps writing to an inode that another process renamed.
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const FILE_BYTES: usize = 2 * 1024 * 1024;
const ARCHIVES: usize = 3;

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

pub(super) fn redirect(path: &Path) -> io::Result<()> {
    use std::{
        os::{fd::AsRawFd, unix::net::UnixStream},
        thread,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    private_file(path)?;
    let lock = private_file(&path.with_extension("log.lock"))?;
    append(path, &lock, &[], FILE_BYTES)?;
    let (mut reader, writer) = UnixStream::pair()?;
    let path = path.to_path_buf();
    // dup2 is only done after the reader exists. The kernel socket buffer and this
    // fixed buffer bound queued log bytes even when the disk is slow.
    thread::Builder::new()
        .name("verge-log".into())
        .spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let _ = append(&path, &lock, &buffer[..count], FILE_BYTES);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })?;
    if unsafe { libc::dup2(writer.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
