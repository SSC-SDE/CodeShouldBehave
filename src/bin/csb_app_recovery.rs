//! Offline recovery only: no network, activation, deployment, or in-place restore.
use flowdiff::{
    Storage,
    github_app_setup::{
        AppExecutionQueue, AppOfflineRecovery, AppReceiptSigner, AppRecoveryDecision,
        AppRecoveryRequest,
    },
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const USAGE: &str = "csb-app-recovery (offline; stop ALL writers first)\n\
  backup APP EXISTING_DB ARTIFACT_ROOT SIGNING_KEY_DER NEW_BACKUP_DIR --quiesced\n\
  verify APP BACKUP_DIR EXTERNAL_ANCHOR_JSON INDEPENDENT_ANCHOR_SHA256 PUBLIC_KEY_RAW\n\
  copy APP BACKUP_DIR EXTERNAL_ANCHOR_JSON INDEPENDENT_ANCHOR_SHA256 PUBLIC_KEY_RAW NEW_DESTINATION --quiesced\n\
  abandon APP EXISTING_DB ARTIFACT_ROOT JOB_ID OPERATOR REASON --quiesced\n\
  self-test\n\
Only an independently retained anchor for LATEST COMMITTED state authorizes restore.\n\
Any later DB mutation invalidates that authority until a new anchor is captured.\n\
Keep all writers stopped through capture/verification/copy. A newest AVAILABLE backup\n\
does not prove that later withdrawals/reservations did not occur; qualify external fencing.\n\
Copy never overwrites, migrates, opens a host, or serves a restored database.\n\
Report directories need separate immutable retention and normal authorization.\n\
Use canonical local paths in operator-owned directories, not symlinks.";

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() || arguments == ["--help"] || arguments == ["-h"] {
        println!("{USAGE}");
        return;
    }
    if run(&arguments).is_err() {
        eprintln!("Offline App recovery refused; no database was activated or served.\n{USAGE}");
        std::process::exit(1);
    }
}
fn run(args: &[String]) -> Result<(), ()> {
    match args.first().map(String::as_str) {
        Some("backup") if args.len() == 7 && args[6] == "--quiesced" => {
            let app = app_id(&args[1])?;
            let storage = open_current_storage(Path::new(&args[2]), Path::new(&args[3]))?;
            let key = protected_key(Path::new(&args[4]))?;
            let signer = AppReceiptSigner::from_pkcs8(&key).map_err(|_| ())?;
            let queue = AppExecutionQueue::new(storage, app).map_err(|_| ())?;
            let anchor = queue
                .create_recovery_backup(Path::new(&args[5]), Path::new(&args[3]), &signer)
                .map_err(|_| ())?;
            println!(
                "Independent anchor SHA-256: {}",
                anchor.digest().map_err(|_| ())?
            );
            println!(
                "Backup created only; retain the pin independently. Any later DB mutation invalidates this snapshot's latest-state restore authority until recaptured."
            );
            Ok(())
        }
        Some("verify") if args.len() == 6 => {
            let key = read_regular(Path::new(&args[5]), 32)?;
            if key.len() != 32 {
                return Err(());
            }
            let verified = AppOfflineRecovery::verify(
                Path::new(&args[2]),
                Path::new(&args[3]),
                &args[4],
                app_id(&args[1])?,
                &key,
            )
            .map_err(|_| ())?;
            println!(
                "Exact trusted snapshot verified for App {}; {} receipts, {} artifacts; not serving.",
                verified.snapshot.app_id, verified.snapshot.receipt_count, verified.artifact_count
            );
            Ok(())
        }
        Some("copy") if args.len() == 8 && args[7] == "--quiesced" => {
            let key = read_regular(Path::new(&args[5]), 32)?;
            if key.len() != 32 {
                return Err(());
            }
            let verified = AppOfflineRecovery::verify_and_copy(
                Path::new(&args[2]),
                Path::new(&args[6]),
                Path::new(&args[3]),
                &args[4],
                app_id(&args[1])?,
                &key,
            )
            .map_err(|_| ())?;
            println!(
                "Exact trusted snapshot copied to NEW destination for App {}; not serving.",
                verified.snapshot.app_id
            );
            Ok(())
        }
        Some("abandon") if args.len() == 8 && args[7] == "--quiesced" => {
            let storage = open_current_storage(Path::new(&args[2]), Path::new(&args[3]))?;
            let queue = AppExecutionQueue::new(storage, app_id(&args[1])?).map_err(|_| ())?;
            let request = AppRecoveryRequest {
                job_id: &args[4],
                actor: &args[5],
                reason: &args[6],
                decision: AppRecoveryDecision::Abandon,
            };
            queue.recover_job(&request, None).map_err(|_| ())?;
            println!(
                "Explicitly abandoned; attempts, lifetime budgets and create fence retained. Create and independently retain a NEW backup anchor before resuming writers."
            );
            Ok(())
        }
        Some("self-test") if args.len() == 1 => self_test(),
        _ => Err(()),
    }
}
fn open_current_storage(database: &Path, artifacts: &Path) -> Result<Storage, ()> {
    for path in [database, artifacts] {
        for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
            if fs::symlink_metadata(ancestor)
                .map_err(|_| ())?
                .file_type()
                .is_symlink()
            {
                return Err(());
            }
        }
    }
    // Explicitly refuse creation or implicit schema upgrade. Advance this protocol
    // version only alongside a newly qualified recovery schema.
    let source =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| ())?;
    let version: i64 = source
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|_| ())?;
    if version != 25 {
        return Err(());
    }
    drop(source);
    Storage::open(database, artifacts).map_err(|_| ())
}
fn app_id(text: &str) -> Result<i64, ()> {
    text.parse::<i64>().ok().filter(|id| *id > 0).ok_or(())
}
fn read_regular(path: &Path, maximum: u64) -> Result<Vec<u8>, ()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if fs::symlink_metadata(ancestor)
            .map_err(|_| ())?
            .file_type()
            .is_symlink()
        {
            return Err(());
        }
    }
    let file = fs::File::open(path).map_err(|_| ())?;
    if !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > maximum {
        return Err(());
    }
    Ok(bytes)
}
fn protected_key(path: &Path) -> Result<Vec<u8>, ()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path).map_err(|_| ())?.permissions().mode() & 0o077 != 0 {
            return Err(());
        }
    }
    read_regular(path, 16384)
}
struct TemporaryDirectory(PathBuf);
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn self_test() -> Result<(), ()> {
    use ring::{
        rand::{SecureRandom, SystemRandom},
        signature::Ed25519KeyPair,
    };
    let random = SystemRandom::new();
    let mut nonce = [0u8; 16];
    random.fill(&mut nonce).map_err(|_| ())?;
    let name = nonce.iter().map(|n| format!("{n:02x}")).collect::<String>();
    let root = std::env::temp_dir()
        .canonicalize()
        .map_err(|_| ())?
        .join(format!("csb-offline-recovery-{name}"));
    fs::create_dir(&root).map_err(|_| ())?;
    let root = TemporaryDirectory(root);
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).map_err(|_| ())?;
    let signer = AppReceiptSigner::from_pkcs8(pkcs8.as_ref()).map_err(|_| ())?;
    let artifacts = root.0.join("artifacts");
    let storage = Storage::open(&root.0.join("source.db"), &artifacts).map_err(|_| ())?;
    let queue = AppExecutionQueue::new(storage, 1).map_err(|_| ())?;
    let backup = root.0.join("backup");
    let anchor = queue
        .create_recovery_backup(&backup, &artifacts, &signer)
        .map_err(|_| ())?;
    // Simulates separately retained operator trust, not a pin auto-read from DB.
    let pin = anchor.digest().map_err(|_| ())?;
    let trusted = root.0.join("independent-anchor.json");
    fs::write(&trusted, anchor.bytes().map_err(|_| ())?).map_err(|_| ())?;
    drop(queue);
    AppOfflineRecovery::verify(&backup, &trusted, &pin, 1, signer.public_key()).map_err(|_| ())?;
    let copied = root.0.join("copied");
    AppOfflineRecovery::verify_and_copy(&backup, &copied, &trusted, &pin, 1, signer.public_key())
        .map_err(|_| ())?;
    if AppOfflineRecovery::verify(&backup, &trusted, &pin, 2, signer.public_key()).is_ok()
        || AppOfflineRecovery::verify_and_copy(
            &backup,
            &copied,
            &trusted,
            &pin,
            1,
            signer.public_key(),
        )
        .is_ok()
    {
        return Err(());
    }
    println!(
        "Offline backup/anchor/verify/new-copy and foreign-App/no-overwrite boundaries passed; ephemeral fixtures only, no production credentials or network."
    );
    Ok(())
}
