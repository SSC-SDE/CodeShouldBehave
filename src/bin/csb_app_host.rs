//! Inactive loopback service behind an operator-managed HTTPS reverse proxy.
//! No automatic jobs, report publication or activation switch is provided.
use flowdiff::{
    Storage,
    github_app_setup::{
        AppSetupHost, AppWebhookInbox, GitHubWebProvider, SandboxPreviewWorker, SetupConfig,
        router, webhook_router,
    },
};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
fn secret_file(path: &Path) -> Result<String, ()> {
    let before = std::fs::symlink_metadata(path).map_err(|_| ())?;
    if !before.is_file() || before.len() > 1024 {
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
    let mut value = String::new();
    file.take(1025).read_to_string(&mut value).map_err(|_| ())?;
    let value = value.trim().to_owned();
    if value.len() < 16 || value.len() > 1024 {
        return Err(());
    }
    Ok(value)
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
        ) || result.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err(());
        }
    }
    Ok(result)
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
    let app_id = get("--app-id")?.parse().map_err(|_| ())?;
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
        config,
    );
    let worker_keys = [
        "--python",
        "--preview-launcher",
        "--preview-engine",
        "--preview-sha256",
    ];
    if worker_keys.iter().any(|k| opts.contains_key(*k)) {
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
    let mut app = router(Arc::new(host));
    if let Some(path) = opts.get("--webhook-secret-file") {
        let inbox = AppWebhookInbox::new(
            Storage::open(&state.join("app.sqlite"), &state.join("artifacts")).map_err(|_| ())?,
            app_id,
            secret_file(Path::new(path))?.into_bytes(),
        )
        .map_err(|_| ())?;
        app = app.merge(webhook_router(Arc::new(inbox)));
    }
    app = app.route(
        "/",
        axum::routing::get(|| async { axum::response::Redirect::temporary("/github-app/setup") }),
    );
    app=app.route("/workspace",axum::routing::get(||async{(axum::http::StatusCode::SERVICE_UNAVAILABLE,"This inactive setup host does not serve PR reports. Report delivery and activation are not configured.")}));
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| ())?;
    eprintln!(
        "CodeShouldBehave setup host listening on loopback; activation and automatic checks OFF"
    );
    axum::serve(listener, app).await.map_err(|_| ())
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
}
