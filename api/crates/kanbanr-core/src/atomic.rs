//! The one way a board file is written (DEF-025, factor 9): the new bytes go to a temporary file
//! in the same folder and are renamed over the old file, so the path holds the whole old content
//! or the whole new content at every moment. A crash, a kill or a full disk leaves the old file
//! and at most a `.tmp` leftover; a reader that takes no lock never sees half a file.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// The suffix every temporary file ends in, for the ignore rule, `leftovers` and the guard test.
pub const TEMP_SUFFIX: &str = ".tmp";

/// How often, and how long between tries, a rename refused with `PermissionDenied` is retried
/// (Windows only: a scanner or a reader may hold the target open for a moment).
const RENAME_TRIES: u32 = 5;
const RENAME_WAIT: Duration = Duration::from_millis(20);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` so that `path` holds the whole old content or the whole new content at
/// every moment (DEF-025). The bytes go to `.<name>.<pid>.<n>.tmp` in the SAME folder (a rename
/// across folders is not atomic and can fail across mounts), are synced, and are renamed over
/// `path`. A symbolic link is written through: the file it points to is replaced, the link stays.
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_with(path.as_ref(), bytes.as_ref(), |_| Ok(()))
}

/// `write` with a hook that runs after the temporary file is complete and synced and before the
/// rename. Test seam only: `write` calls it with a no-op. The hook gets the temporary file's path;
/// an `Err` from it aborts the write like any failure.
pub(crate) fn write_with(
    path: &Path,
    bytes: &[u8],
    before_rename: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let folder = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} has no file name", target.display()),
        )
    })?;
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = folder.join(format!(
        ".{}.{}.{n}{TEMP_SUFFIX}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(old) = std::fs::metadata(&target) {
            file.set_permissions(old.permissions())?;
        }
        drop(file);
        before_rename(&tmp)?;
        rename(&tmp, &target)
    })();
    match result {
        Ok(()) => {
            sync_folder(&folder);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(windows)]
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 1;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < RENAME_TRIES => {
                tries += 1;
                std::thread::sleep(RENAME_WAIT);
            }
            other => return other,
        }
    }
}

#[cfg(not(windows))]
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let _ = (RENAME_TRIES, RENAME_WAIT);
    std::fs::rename(from, to)
}

/// Best effort: sync the folder so the rename itself survives a power cut.
#[cfg(unix)]
fn sync_folder(folder: &Path) {
    if let Ok(dir) = std::fs::File::open(folder) {
        let _ = dir.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_folder(_folder: &Path) {}

/// Temporary files of this write left in `dir` (not below it) whose last change is older than
/// `age`. Read-only; doctor uses it.
pub fn leftovers(dir: &Path, age: Duration) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| is_temp_name(&e.file_name().to_string_lossy()))
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|d| d >= age)
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}

/// `.<name>.<digits>.<digits>.tmp`
fn is_temp_name(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix('.')
        .and_then(|r| r.strip_suffix(TEMP_SUFFIX))
    else {
        return false;
    };
    let mut parts = rest.rsplitn(3, '.');
    let (n, pid, base) = (parts.next(), parts.next(), parts.next());
    let digits =
        |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    digits(n) && digits(pid) && base.is_some_and(|b| !b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanbanr-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_old_file_stays_whole_until_the_rename() {
        let dir = scratch("old-whole");
        let path = dir.join("f.yaml");
        std::fs::write(&path, "old").unwrap();
        write_with(&path, b"new", |tmp| {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
            assert_eq!(std::fs::read_to_string(tmp).unwrap(), "new");
            assert_eq!(tmp.parent(), path.parent());
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names(&dir), vec!["f.yaml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reader_never_sees_a_partial_file() {
        let dir = scratch("reader");
        let path = dir.join("f.yaml");
        let a = format!("A\n{}", "a".repeat(256 * 1024));
        let b = format!("B\n{}", "b".repeat(256 * 1024));
        write(&path, &a).unwrap();
        let done = std::sync::atomic::AtomicBool::new(false);
        let bad = std::sync::Mutex::new(Vec::<String>::new());
        let reads = AtomicU64::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    while !done.load(Ordering::Relaxed) {
                        match std::fs::read_to_string(&path) {
                            Ok(text) if text == a || text == b => {}
                            Ok(text) => bad.lock().unwrap().push(format!("{} bytes", text.len())),
                            Err(e) => bad.lock().unwrap().push(e.to_string()),
                        }
                        reads.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            for i in 0..2000 {
                write(&path, if i % 2 == 0 { &b } else { &a }).unwrap();
            }
            done.store(true, Ordering::Relaxed);
        });
        assert!(bad.lock().unwrap().is_empty(), "{:?}", bad.lock().unwrap());
        assert!(reads.load(Ordering::Relaxed) >= 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_temporary_file() {
        let dir = scratch("failed");
        let path = dir.join("f.yaml");
        std::fs::write(&path, "old").unwrap();
        let err = write_with(&path, b"new", |_| Err(std::io::Error::other("stop"))).unwrap_err();
        assert_eq!(err.to_string(), "stop");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names(&dir), vec!["f.yaml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_an_error_and_makes_nothing() {
        let dir = scratch("missing");
        let folder = dir.join("nope");
        let err = write(folder.join("f.yaml"), "x").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(!folder.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_old_files_permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        for mode in [0o600, 0o755] {
            let path = dir.join(format!("f{mode:o}"));
            std::fs::write(&path, "old").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            write(&path, "new").unwrap();
            let got = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(got, mode);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_written_through() {
        let dir = scratch("link");
        let real = dir.join("real.yaml");
        let link = dir.join("link.yaml");
        std::fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leftovers_names_only_old_temporary_files() {
        let dir = scratch("leftovers");
        std::fs::write(dir.join(".f.yaml.12.3.tmp"), "x").unwrap();
        std::fs::write(dir.join("keep.tmp"), "x").unwrap();
        std::fs::write(dir.join(".dot.tmp"), "x").unwrap();
        assert_eq!(
            leftovers(&dir, Duration::ZERO),
            vec![dir.join(".f.yaml.12.3.tmp")]
        );
        assert!(leftovers(&dir, Duration::from_secs(3600)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Prints the cost of the sync (DEF-025 T1): `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fsync_cost_is_recorded() {
        let dir = scratch("cost");
        let bytes = vec![b'x'; 20 * 1024];
        let mut ms: Vec<f64> = (0..200)
            .map(|i| {
                let t = std::time::Instant::now();
                write(dir.join(format!("f{}.yaml", i % 20)), &bytes).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "atomic write 20 KB: median {:.2} ms, p95 {:.2} ms",
            ms[100], ms[190]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
