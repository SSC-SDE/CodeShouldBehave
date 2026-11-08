//! Inactive loopback service behind an operator-managed HTTPS reverse proxy.
//! Dispatch defaults paused. Active work additionally requires separate saved consent.
use flowdiff::{
    Storage,
    github_app_setup::{
        AppDispatchConfig, AppDispatcher, AppExecutionQueue, AppOfflineRecovery, AppReceiptSigner,
        AppReportStore, AppRequestBudget, AppSetupHost, AppWebhookInbox, GitHubInstallationBroker,
        GitHubWebProvider, HttpsReportReadback, PublicationOperatorPolicy, SandboxPrWorker,
        SandboxPreviewWorker, SetupConfig, app_report_router, router, webhook_router,
    },
};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
fn private_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, ()> {
    let before = std::fs::symlink_metadata(path).map_err(|_| ())?;
    if !before.is_file() || before.len() > limit {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if before.permissions().mode() & 0o077 != 0 {
            return Err(());
        }
    }
    let file = std::fs::File::open(path).map_err(|_| ())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let after = file.metadata().map_err(|_| ())?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(());
        }
    }
    let mut value = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut value)
        .map_err(|_| ())?;
    if value.len() < 16 || value.len() as u64 > limit {
        return Err(());
    }
    Ok(value)
}
fn secret_file(path: &Path) -> Result<String, ()> {
    String::from_utf8(private_bytes(path, 1024)?)
        .map(|v| v.trim().to_owned())
        .map_err(|_| ())
}
fn options() -> Result<BTreeMap<String, String>, ()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() % 2 != 0 {
        return Err(());
    }
    let mut result = BTreeMap::new();
    for pair in args.chunks(2) {
        if !matches!(
            pair[0].as_str(),
            "--listen"
                | "--state"
                | "--app-id"
                | "--client-id"
                | "--origin"
                | "--client-secret-file"
                | "--webhook-secret-file"
                | "--python"
                | "--preview-launcher"
                | "--preview-engine"
                | "--preview-sha256"
                | "--app-private-key-file"
                | "--report-signing-key-file"
                | "--report-origin"
                | "--report-listen"
                | "--pr-launcher"
                | "--pr-engine"
                | "--pr-sha256"
                | "--dispatch-mode"
                | "--restore-from"
                | "--recovery-anchor-file"
                | "--recovery-anchor-sha256"
                | "--recovery-receipt-key-file"
                | "--recovery-safety"
        ) || result.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err(());
        }
    }
    Ok(result)
}

/// A restored database is verified before Storage can migrate or serve it. This
/// one-shot import always boots paused and refuses an existing destination.
fn restore_state(opts: &BTreeMap<String, String>, state: &Path, app_id: i64) -> Result<(), ()> {
    let keys = [
        "--restore-from",
        "--recovery-anchor-file",
        "--recovery-anchor-sha256",
        "--recovery-receipt-key-file",
        "--recovery-safety",
    ];
    if !keys.iter().any(|k| opts.contains_key(*k)) {
        return Ok(());
    }
    if keys.iter().any(|k| !opts.contains_key(*k))
        || opts
            .get("--dispatch-mode")
            .is_some_and(|mode| mode != "paused")
        || opts.get("--recovery-safety").map(String::as_str) != Some("latest-committed-quiesced")
    {
        return Err(());
    }
    let public_key = private_bytes(Path::new(&opts["--recovery-receipt-key-file"]), 32)?;
    if public_key.len() != 32 {
        return Err(());
    }
    AppOfflineRecovery::verify_and_copy(
        Path::new(&opts["--restore-from"]),
        state,
        Path::new(&opts["--recovery-anchor-file"]),
        &opts["--recovery-anchor-sha256"],
        app_id,
        &public_key,
    )
    .map_err(|_| ())?;
    // Import data, not runtime authority. Re-enablement needs a fresh signed
    // operator activation; budgets, attempts and tombstones remain unchanged.
    {
        let imported = rusqlite::Connection::open_with_flags(
            state.join("state.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|_| ())?;
        imported
            .execute("UPDATE app_execution_grants SET enabled=0", [])
            .map_err(|_| ())?;
    }
    std::fs::rename(state.join("state.db"), state.join("app.sqlite")).map_err(|_| ())?;
    std::fs::File::open(state)
        .and_then(|f| f.sync_all())
        .map_err(|_| ())
}
async fn run() -> Result<(), ()> {
    let opts = options()?;
    let get = |key: &str| opts.get(key).ok_or(());
    let listen: std::net::SocketAddr = get("--listen")?.parse().map_err(|_| ())?;
    if !listen.ip().is_loopback() || listen.port() == 0 {
        return Err(());
    }
    let state = PathBuf::from(get("--state")?);
    if !state.is_absolute() {
        return Err(());
    }
    let app_id = get("--app-id")?.parse().map_err(|_| ())?;
    restore_state(&opts, &state, app_id)?;
    if !state.exists() {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&state).map_err(|_| ())?;
    }
    let meta = std::fs::symlink_metadata(&state).map_err(|_| ())?;
    if !meta.is_dir() {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(());
        }
    }
    let config = SetupConfig::new(
        app_id,
        get("--client-id")?.clone(),
        get("--origin")?.clone(),
    )
    .map_err(|_| ())?;
    let provider = GitHubWebProvider::new(
        config.clone(),
        secret_file(Path::new(get("--client-secret-file")?))?,
    )
    .map_err(|_| ())?;
    let mut host = AppSetupHost::new(
        Storage::open(&state.join("app.sqlite"), &state.join("artifacts")).map_err(|_| ())?,
        provider,
        config.clone(),
    );
    let worker_keys = [
        "--python",
        "--preview-launcher",
        "--preview-engine",
        "--preview-sha256",
    ];
    if worker_keys[1..].iter().any(|k| opts.contains_key(*k)) {
        host = host.with_scan_worker(Arc::new(
            SandboxPreviewWorker::new(
                PathBuf::from(get("--python")?),
                PathBuf::from(get("--preview-launcher")?),
                PathBuf::from(get("--preview-engine")?),
                get("--preview-sha256")?.clone(),
            )
            .map_err(|_| ())?,
        ));
    }
    if let (Some(origin), Some(engine)) = (opts.get("--report-origin"), opts.get("--pr-sha256")) {
        let metadata = serde_json::json!({"package":"flowdiff","version":env!("CARGO_PKG_VERSION"),"hld_generator":flowdiff::design::HLD_GENERATOR_VERSION,"lld_generator":flowdiff::lld::LLD_GENERATOR_VERSION,"lld_parser":flowdiff::lld::LLD_IR_VERSION,"public_projection":flowdiff::public_report::PUBLIC_SCHEMA});
        let hash = flowdiff::public_report::sha256(
            &flowdiff::public_report::canonical_json(&metadata).map_err(|_| ())?,
        );
        host = host
            .with_publication_policy(
                PublicationOperatorPolicy::new(origin.clone(), engine.clone(), hash)
                    .map_err(|_| ())?,
            )
            .map_err(|_| ())?;
    }
    let mut app = router(Arc::new(host));
    let mut inbox_handle = None;
    if let Some(path) = opts.get("--webhook-secret-file") {
        let inbox = AppWebhookInbox::new(
            Storage::open(&state.join("app.sqlite"), &state.join("artifacts")).map_err(|_| ())?,
            app_id,
            secret_file(Path::new(path))?.into_bytes(),
        )
        .map_err(|_| ())?;
        let inbox = Arc::new(inbox);
        app = app.merge(webhook_router(inbox.clone()));
        inbox_handle = Some(inbox);
    }
    let execution_keys = [
        "--app-private-key-file",
        "--report-signing-key-file",
        "--report-origin",
        "--report-listen",
        "--pr-launcher",
        "--pr-engine",
        "--pr-sha256",
        "--dispatch-mode",
    ];
    let mut runtime = None;
    if execution_keys.iter().any(|k| opts.contains_key(*k)) {
        let report_origin = get("--report-origin")?.clone();
        if report_origin.eq_ignore_ascii_case(get("--origin")?) {
            return Err(());
        }
        let report_listen: std::net::SocketAddr =
            get("--report-listen")?.parse().map_err(|_| ())?;
        if !report_listen.ip().is_loopback() || report_listen.port() == 0 || report_listen == listen
        {
            return Err(());
        }
        let mode = opts
            .get("--dispatch-mode")
            .map(String::as_str)
            .unwrap_or("paused");
        if !matches!(mode, "paused" | "consented") {
            return Err(());
        }
        let queue = Arc::new(
            AppExecutionQueue::new(
                Storage::open(&state.join("app.sqlite"), &state.join("artifacts"))
                    .map_err(|_| ())?,
                app_id,
            )
            .map_err(|_| ())?,
        );
        let signer = Arc::new(
            AppReceiptSigner::from_pkcs8(&private_bytes(
                Path::new(get("--report-signing-key-file")?),
                16384,
            )?)
            .map_err(|_| ())?,
        );
        let reports = Arc::new(
            AppReportStore::new(
                state.join("reports"),
                Storage::open(&state.join("app.sqlite"), &state.join("artifacts"))
                    .map_err(|_| ())?,
                queue.clone(),
                signer.public_key().to_vec(),
                app_id,
            )
            .map_err(|_| ())?,
        );
        let broker = Arc::new(
            GitHubInstallationBroker::new(
                app_id,
                get("--client-id")?.clone(),
                &private_bytes(Path::new(get("--app-private-key-file")?), 16384)?,
            )
            .map_err(|_| ())?,
        );
        let worker = Arc::new(
            SandboxPrWorker::new(
                PathBuf::from(get("--python")?),
                PathBuf::from(get("--pr-launcher")?),
                PathBuf::from(get("--pr-engine")?),
                get("--pr-sha256")?.clone(),
            )
            .map_err(|_| ())?,
        );
        let readback = Arc::new(HttpsReportReadback::new(report_origin.clone()).map_err(|_| ())?);
        let dispatcher = Arc::new(
            AppDispatcher::new(AppDispatchConfig {
                budget: Arc::new(
                    AppRequestBudget::new(
                        Storage::open(&state.join("app.sqlite"), &state.join("artifacts"))
                            .map_err(|_| ())?,
                        app_id,
                    )
                    .map_err(|_| ())?,
                ),
                queue,
                inbox: inbox_handle.ok_or(())?,
                broker,
                worker,
                reports: reports.clone(),
                readback,
                signer,
                report_origin,
            })
            .map_err(|_| ())?,
        );
        runtime = Some((
            report_listen,
            app_report_router(reports),
            dispatcher,
            mode == "consented",
        ));
    }
    app = app.route(
        "/",
        axum::routing::get(|| async { axum::response::Redirect::temporary("/github-app/setup") }),
    );
    app=app.route("/workspace",axum::routing::get(||async{(axum::http::StatusCode::SERVICE_UNAVAILABLE,"This inactive setup host does not serve PR reports. Report delivery and activation are not configured.")}));
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| ())?;
    if let Some((address, report_app, dispatcher, run)) = runtime {
        let report_listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|_| ())?;
        let dispatch_task = if run {
            Some(tokio::spawn(async move {
                loop {
                    let dispatcher = dispatcher.clone();
                    if !matches!(
                        tokio::task::spawn_blocking(move || dispatcher.tick()).await,
                        Ok(Ok(_))
                    ) {
                        eprintln!("App dispatch interrupted; durable leases retained");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            }))
        } else {
            None
        };
        eprintln!(
            "App host configured; dispatch requires consent and remains subject to operator mode"
        );
        let result = tokio::select! {result=axum::serve(listener,app)=>result,result=axum::serve(report_listener,report_app)=>result};
        if let Some(task) = dispatch_task {
            task.abort();
        }
        result.map_err(|_| ())
    } else {
        eprintln!("CodeShouldBehave setup host listening on loopback; automatic checks OFF");
        axum::serve(listener, app).await.map_err(|_| ())
    }
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!(
            "Cannot start inactive App host. Check operator configuration, private state/secret permissions and loopback address. No credentials were logged."
        );
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn secrets_require_private_regular_files_and_never_follow_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("csb-host-secret-fixture-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("credential");
        std::fs::write(&file, "synthetic_host_fixture_value").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(secret_file(&file).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(secret_file(&file).unwrap(), "synthetic_host_fixture_value");
        let linked = root.join("linked");
        symlink(&file, &linked).unwrap();
        assert!(secret_file(&linked).is_err());
        std::fs::write(&file, vec![b'x'; 1025]).unwrap();
        assert!(secret_file(&file).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn verified_import_pauses_grants_and_preserves_nonrefundable_state() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csb-host-restore-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let database = root.join("source.db");
        let artifacts = root.join("artifacts");
        let storage = Storage::open(&database, &artifacts).unwrap();
        let c = rusqlite::Connection::open(&database).unwrap();
        c.execute_batch("INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) VALUES(1,2,3,'fixture','app',4,1);
            INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) VALUES(1,2,3,1,4,1,'{}');
            INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) VALUES(1,2,3,1,4,1,'{}');
            INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,enabled)
            VALUES(1,2,3,1,printf('%064x',1),printf('%064x',2),printf('%064x',3),'{}',1);
            INSERT INTO app_request_budgets VALUES(1,'job:fixture',123);").unwrap();
        drop(c);
        let queue = AppExecutionQueue::new(storage, 1).unwrap();
        let pkcs8 =
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap();
        let signer = AppReceiptSigner::from_pkcs8(pkcs8.as_ref()).unwrap();
        let backup = root.join("backup");
        let anchor = queue
            .create_recovery_backup(&backup, &artifacts, &signer)
            .unwrap();
        drop(queue);
        let public_key = root.join("receipt.pub");
        std::fs::write(&public_key, signer.public_key()).unwrap();
        std::fs::set_permissions(&public_key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let destination = root.join("imported");
        let mut opts = BTreeMap::from([
            ("--restore-from".into(), backup.to_str().unwrap().into()),
            (
                "--recovery-anchor-file".into(),
                backup.join("anchor.json").to_str().unwrap().into(),
            ),
            ("--recovery-anchor-sha256".into(), anchor.digest().unwrap()),
            (
                "--recovery-receipt-key-file".into(),
                public_key.to_str().unwrap().into(),
            ),
            (
                "--recovery-safety".into(),
                "latest-committed-quiesced".into(),
            ),
            ("--dispatch-mode".into(), "consented".into()),
        ]);
        assert!(restore_state(&opts, &destination, 1).is_err());
        assert!(!destination.exists());
        opts.insert("--dispatch-mode".into(), "paused".into());
        restore_state(&opts, &destination, 1).unwrap();
        let c = rusqlite::Connection::open(destination.join("app.sqlite")).unwrap();
        assert_eq!(
            c.query_row("SELECT enabled FROM app_execution_grants", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT used FROM app_request_budgets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            123
        );
        assert!(restore_state(&opts, &destination, 1).is_err());
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
}
