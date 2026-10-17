//! Operator-only SQLite backup. Never registered as an HTTP onboarding action.
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(5);
const BUSY_WAIT: Duration = Duration::from_millis(100);
const FAILURE: &str = "local_database_copy_unavailable";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    owner: u32,
}

fn version_supported(version: i32) -> Result<(), ()> {
    if version >= 3_051_003 {
        Ok(())
    } else {
        Err(())
    }
}

#[cfg(unix)]
fn metadata(path: &Path, owner: Option<u32>, directory: bool) -> Result<Identity, ()> {
    use std::os::unix::fs::MetadataExt;
    let item = fs::symlink_metadata(path).map_err(|_| ())?;
    if item.file_type().is_symlink()
        || item.mode() & 0o077 != 0
        || owner.is_some_and(|expected| item.uid() != expected)
        || (directory && !item.is_dir())
        || (!directory && (!item.is_file() || item.nlink() != 1 || item.len() > MAX_BYTES))
    {
        return Err(());
    }
    Ok(Identity {
        device: item.dev(),
        inode: item.ino(),
        owner: item.uid(),
    })
}
#[cfg(not(unix))]
fn metadata(_: &Path, _: Option<u32>, _: bool) -> Result<Identity, ()> {
    Err(())
}

fn family(path: &Path, expected: Identity) -> Result<(), ()> {
    if metadata(path, Some(expected.owner), false)? != expected {
        return Err(());
    }
    let mut bytes = fs::symlink_metadata(path).map_err(|_| ())?.len();
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.to_str().ok_or(())?));
        match fs::symlink_metadata(&sidecar) {
            Ok(item) => {
                metadata(&sidecar, Some(expected.owner), false)?;
                bytes = bytes.checked_add(item.len()).ok_or(())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(()),
        }
    }
    if bytes > MAX_BYTES { Err(()) } else { Ok(()) }
}

fn checked_path(path: &Path) -> Result<(PathBuf, Identity), ()> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(());
    }
    // Host validates these parents' UID against the authenticated operator UID.
    let parent = path.parent().ok_or(())?;
    let parent_identity = metadata(parent, None, true)?;
    let name = path.file_name().ok_or(())?;
    let canonical = parent.canonicalize().map_err(|_| ())?.join(name);
    let identity = metadata(&canonical, Some(parent_identity.owner), false)?;
    family(&canonical, identity)?;
    Ok((canonical, identity))
}

fn pages(connection: &Connection) -> Result<(u64, u64), ()> {
    let count: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(|_| ())?;
    let size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(|_| ())?;
    let count = u64::try_from(count).map_err(|_| ())?;
    let size = u64::try_from(size).map_err(|_| ())?;
    if !(512..=65536).contains(&size)
        || !size.is_power_of_two()
        || count.checked_mul(size).ok_or(())? > MAX_BYTES
    {
        return Err(());
    }
    Ok((count, size))
}

fn complete(
    backup: &Backup<'_, '_>,
    page_size: u64,
    source: (&Path, Identity),
    destination: (&Path, Identity),
    mut elapsed: impl FnMut() -> Duration,
) -> Result<(), ()> {
    loop {
        if elapsed() >= DEADLINE {
            return Err(());
        }
        family(source.0, source.1)?;
        family(destination.0, destination.1)?;
        match backup.step(64).map_err(|_| ())? {
            // DONE has committed. Never report a timeout or metadata error afterwards.
            StepResult::Done => return Ok(()),
            StepResult::More => {
                let count = u64::try_from(backup.progress().pagecount).map_err(|_| ())?;
                if count.checked_mul(page_size).ok_or(())? > MAX_BYTES {
                    return Err(());
                }
            }
            StepResult::Busy | StepResult::Locked => {
                let remaining = DEADLINE.saturating_sub(elapsed());
                thread::sleep(BUSY_WAIT.min(remaining));
            }
            _ => return Err(()),
        }
    }
}

fn copy(request: Request) -> Result<(), ()> {
    version_supported(rusqlite::version_number())?;
    let started = Instant::now();
    let (source_path, source_id) = checked_path(&request.source)?;
    let (destination_path, destination_id) = checked_path(&request.destination)?;
    if source_id == destination_id || source_id.owner != destination_id.owner {
        return Err(());
    }
    let source = Connection::open_with_flags(
        &source_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| ())?;
    let mut destination = Connection::open_with_flags(
        &destination_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| ())?;
    source.busy_timeout(BUSY_WAIT).map_err(|_| ())?;
    destination.busy_timeout(BUSY_WAIT).map_err(|_| ())?;
    // Pin the source snapshot so writes cannot reset backup progress or expand page count.
    source.execute_batch("BEGIN").map_err(|_| ())?;
    let (_, page_size) = pages(&source)?;
    pages(&destination)?;
    let backup = Backup::new(&source, &mut destination).map_err(|_| ())?;
    // Dropping a non-DONE backup rolls its destination transaction back in SQLite.
    complete(
        &backup,
        page_size,
        (&source_path, source_id),
        (&destination_path, destination_id),
        || started.elapsed(),
    )
}

pub(super) fn invoke(request: Value) -> Result<Value, String> {
    let copied = serde_json::from_value(request)
        .map_err(|_| ())
        .and_then(copy);
    if copied.is_ok() {
        Ok(json!({"status":200,"value":{"copied":true}}))
    } else {
        Err(FAILURE.into())
    }
}

/// Creates only the operator-selected local state, never a browser-supplied path.
#[cfg(unix)]
pub(super) fn prepare_operator_database(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let prepare = || -> Result<(), ()> {
        match fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(root)
                    .map_err(|_| ())?;
            }
            Err(_) => return Err(()),
            Ok(_) => {}
        }
        let owner = metadata(root, None, true)?.owner;
        let database = root.join("onboarding.sqlite");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&database)
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(()),
        }
        metadata(&database, Some(owner), false)?;
        Ok(())
    };
    prepare().map_err(|_| "Cannot prepare private onboarding database".into())
}
#[cfg(not(unix))]
pub(super) fn prepare_operator_database(_: &Path) -> Result<(), String> {
    Err("Private onboarding database unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "csb-native-backup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn database(&self, name: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join(name);
            fs::File::create(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bundled_version_gate_and_wal_copy_include_committed_frames_and_keep_inode() {
        assert!(version_supported(3_051_002).is_err());
        assert!(version_supported(3_051_003).is_ok());
        assert!(version_supported(rusqlite::version_number()).is_ok());
        println!("native backup bundled SQLite: {}", rusqlite::version());
        let fixture = Fixture::new();
        let source_path = fixture.database("source.sqlite");
        let destination_path = fixture.database("destination.sqlite");
        let source = Connection::open(&source_path).unwrap();
        source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE test(value); INSERT INTO test VALUES(42);").unwrap();
        assert!(source_path.with_file_name("source.sqlite-wal").exists());
        let live = Connection::open(&destination_path).unwrap();
        live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE previously_committed(value); INSERT INTO previously_committed VALUES(7);").unwrap();
        assert!(
            destination_path
                .with_file_name("destination.sqlite-wal")
                .exists()
        );
        let identity = metadata(&destination_path, None, false).unwrap();
        copy(Request {
            source: source_path.clone(),
            destination: destination_path.clone(),
        })
        .unwrap();
        assert!(metadata(&destination_path, None, false).unwrap() == identity);
        let destination = Connection::open(&destination_path).unwrap();
        assert_eq!(
            destination
                .query_row("SELECT value FROM test", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert_eq!(
            live.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='previously_committed'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(destination);
        drop(live);
        drop(source);
        // Cold WAL header, with cleaned sidecars, must also remain readable.
        assert!(!source_path.with_file_name("source.sqlite-wal").exists());
        copy(Request {
            source: source_path,
            destination: destination_path,
        })
        .unwrap();
    }

    #[test]
    fn incomplete_backup_rolls_back_but_done_is_never_rejected_after_commit() {
        let fixture = Fixture::new();
        let source_path = fixture.database("source.sqlite");
        let destination_path = fixture.database("destination.sqlite");
        let source = Connection::open(&source_path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE payload(value); INSERT INTO payload VALUES(zeroblob(1024*1024));",
            )
            .unwrap();
        let mut destination = Connection::open(&destination_path).unwrap();
        destination
            .execute_batch("CREATE TABLE retained(value); INSERT INTO retained VALUES(7);")
            .unwrap();
        let source_id = metadata(&source_path, None, false).unwrap();
        let destination_id = metadata(&destination_path, None, false).unwrap();
        let calls = Cell::new(0);
        {
            let backup = Backup::new(&source, &mut destination).unwrap();
            assert!(
                complete(
                    &backup,
                    4096,
                    (&source_path, source_id),
                    (&destination_path, destination_id),
                    || {
                        let count = calls.get();
                        calls.set(count + 1);
                        if count == 0 { Duration::ZERO } else { DEADLINE }
                    }
                )
                .is_err()
            );
        }
        assert_eq!(calls.get(), 2);
        assert_eq!(
            destination
                .query_row("SELECT value FROM retained", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            7
        );
        let small = Connection::open_in_memory().unwrap();
        small
            .execute_batch("CREATE TABLE copied(value); INSERT INTO copied VALUES(9);")
            .unwrap();
        calls.set(0);
        {
            let backup = Backup::new(&small, &mut destination).unwrap();
            complete(
                &backup,
                4096,
                (&source_path, source_id),
                (&destination_path, destination_id),
                || {
                    let count = calls.get();
                    calls.set(count + 1);
                    if count == 0 { Duration::ZERO } else { DEADLINE }
                },
            )
            .unwrap();
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(
            destination
                .query_row("SELECT value FROM copied", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
    }

    #[test]
    fn adapter_creates_private_state_and_database_without_replacing_existing_file() {
        use std::os::unix::fs::MetadataExt;
        let fixture = Fixture::new();
        let root = fixture.0.join("state");
        prepare_operator_database(&root).unwrap();
        let database = root.join("onboarding.sqlite");
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&database).unwrap().mode() & 0o777, 0o600);
        let inode = fs::metadata(&database).unwrap().ino();
        fs::write(&database, "preserved existing bytes").unwrap();
        prepare_operator_database(&root).unwrap();
        assert_eq!(fs::metadata(&database).unwrap().ino(), inode);
        assert_eq!(
            fs::read_to_string(&database).unwrap(),
            "preserved existing bytes"
        );
    }

    #[test]
    fn copy_rejects_symlinks_aliases_large_files_and_shared_parents() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        let source = fixture.database("source.sqlite");
        let destination = fixture.database("destination.sqlite");
        let link = fixture.0.join("link.sqlite");
        symlink(&source, &link).unwrap();
        assert!(
            copy(Request {
                source: link,
                destination: destination.clone()
            })
            .is_err()
        );
        assert!(
            copy(Request {
                source: source.clone(),
                destination: source.clone()
            })
            .is_err()
        );
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(
            copy(Request {
                source: source.clone(),
                destination: destination.clone()
            })
            .is_err()
        );
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(0)
            .unwrap();
        fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            copy(Request {
                source,
                destination
            })
            .is_err()
        );
    }
}
