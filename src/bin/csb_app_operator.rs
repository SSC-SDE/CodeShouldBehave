//! Local operator process only. No routes, ambient credentials, or automatic mode.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flowdiff::{
    Storage,
    github_app_setup::{
        ActivationAuthorities, ActivationDocument, ActivationEngineEvidence,
        ActivationHostEvidence, ActivationRuntimePins, ActivationSelection, AppActivationOperator,
        AppRequestBudget, BrokerActivationProvider, GitHubInstallationBroker,
        SignedActivationDocument, verify_activation_qualification,
    },
    public_report::{canonical_json, sha256},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorConfig {
    app_id: i64,
    client_id: String,
    state: PathBuf,
    artifacts: PathBuf,
    app_private_key_file: Option<PathBuf>,
    runtime: ActivationRuntimePins,
    operator_public_key: String,
    engine_verifier_public_key: String,
    host_qualifier_public_key: String,
    engine_file: PathBuf,
    engine_metadata_file: PathBuf,
    launcher_file: PathBuf,
    interpreter_file: PathBuf,
}
fn at() -> Result<i64, ()> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs()
        .try_into()
        .map_err(|_| ())
}
fn operator_key(bytes: &[u8], expected: &[u8; 32]) -> Result<Ed25519KeyPair, ()> {
    // Standard PKCS#8 v1 omits the public component. Independently bind the
    // seed-derived public key to the protected operator root for both versions.
    let pair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(bytes).map_err(|_| ())?;
    if pair.public_key().as_ref() != expected {
        return Err(());
    }
    Ok(pair)
}
fn protected_file(path: &Path, private: bool, limit: u64) -> Result<std::fs::File, ()> {
    if !path.is_absolute() {
        return Err(());
    }
    let before = std::fs::symlink_metadata(path).map_err(|_| ())?;
    if !before.is_file() || before.len() > limit {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if before.permissions().mode() & (if private { 0o077 } else { 0o022 }) != 0 {
            return Err(());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = private;
        return Err(());
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
    Ok(file)
}
fn bytes(path: &Path, limit: u64) -> Result<Vec<u8>, ()> {
    let mut result = Vec::new();
    protected_file(path, true, limit)?
        .take(limit + 1)
        .read_to_end(&mut result)
        .map_err(|_| ())?;
    if result.len() as u64 > limit {
        return Err(());
    }
    Ok(result)
}
fn document<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, ()> {
    serde_json::from_slice(&bytes(Path::new(path), 65536)?).map_err(|_| ())
}
fn key(s: &str) -> Result<[u8; 32], ()> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ())?
        .try_into()
        .map_err(|_| ())
}
impl OperatorConfig {
    fn authorities(&self) -> Result<ActivationAuthorities, ()> {
        Ok(ActivationAuthorities {
            operator: key(&self.operator_public_key)?,
            engine_verifier: key(&self.engine_verifier_public_key)?,
            host_qualifier: key(&self.host_qualifier_public_key)?,
        })
    }
    fn check_files(&self) -> Result<(), ()> {
        for (path, expected) in [
            (&self.engine_file, &self.runtime.engine_digest),
            (&self.launcher_file, &self.runtime.launcher_digest),
            (&self.interpreter_file, &self.runtime.interpreter_digest),
        ] {
            let mut file = protected_file(path, false, 512 * 1024 * 1024)?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            let mut total = 0u64;
            loop {
                let count = file.read(&mut buffer).map_err(|_| ())?;
                if count == 0 {
                    break;
                }
                total += count as u64;
                if total > 512 * 1024 * 1024 {
                    return Err(());
                }
                hash.update(&buffer[..count]);
            }
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let actual = hash.finalize();
            if expected.len() != 64
                || !actual
                    .iter()
                    .zip(expected.as_bytes().chunks_exact(2))
                    .all(|(&b, h)| h == [HEX[(b >> 4) as usize], HEX[(b & 15) as usize]])
            {
                return Err(());
            }
        }
        let metadata: serde_json::Value =
            serde_json::from_slice(&bytes(&self.engine_metadata_file, 65536)?).map_err(|_| ())?;
        if sha256(&canonical_json(&metadata).map_err(|_| ())?)
            != self.runtime.engine_metadata_digest
        {
            return Err(());
        }
        Ok(())
    }
}
fn options(args: &[String]) -> Result<BTreeMap<String, String>, ()> {
    if args.len() % 2 != 0 {
        return Err(());
    }
    let mut result = BTreeMap::new();
    for pair in args.chunks(2) {
        if !matches!(
            pair[0].as_str(),
            "--config"
                | "--engine-evidence"
                | "--host-evidence"
                | "--approval"
                | "--document"
                | "--operator-key-file"
                | "--installation"
                | "--repository"
                | "--consent-revision"
                | "--activation-revision"
        ) || result.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err(());
        }
    }
    Ok(result)
}
fn run(args: &[String]) -> Result<(), ()> {
    let command = args.first().map(String::as_str).ok_or(())?;
    if command == "--help" && args.len() == 1 {
        println!(
            "csb-app-operator verify-evidence --config ABS --engine-evidence ABS --host-evidence ABS\ncsb-app-operator approve --config ABS --document ABS --operator-key-file ABS\ncsb-app-operator preview --config ABS --engine-evidence ABS --host-evidence ABS --installation ID --repository ID --consent-revision N --activation-revision N\ncsb-app-operator commit --config ABS --engine-evidence ABS --host-evidence ABS --approval ABS\nAll JSON/key files require private regular absolute paths. Three independent Ed25519 public keys use base64url without padding. Approval signing uses protected PKCS#8 DER. Preview/commit use live GitHub only with explicitly configured credentials; verify-evidence/approve never open state or contact GitHub. No command qualifies a host or verifies release provenance itself."
        );
        return Ok(());
    }
    let opts = options(&args[1..])?;
    let get = |name: &str| opts.get(name).map(String::as_str).ok_or(());
    let expected: &[&str] = match command {
        "approve" => &["--config", "--document", "--operator-key-file"],
        "verify-evidence" => &["--config", "--engine-evidence", "--host-evidence"],
        "preview" => &[
            "--config",
            "--engine-evidence",
            "--host-evidence",
            "--installation",
            "--repository",
            "--consent-revision",
            "--activation-revision",
        ],
        "commit" => &[
            "--config",
            "--engine-evidence",
            "--host-evidence",
            "--approval",
        ],
        _ => return Err(()),
    };
    if opts.len() != expected.len() || expected.iter().any(|name| !opts.contains_key(*name)) {
        return Err(());
    }
    let config: OperatorConfig = document(get("--config")?)?;
    let authorities = config.authorities()?;
    if command == "approve" {
        let approval: ActivationDocument = document(get("--document")?)?;
        if approval.schema != "csb-app-operator-activation-v1"
            || approval.app_id != config.app_id
            || approval.runtime != config.runtime
            || approval.expires_at <= at()?
            || approval.reviewed_at > at()?
            || approval.reviewed_at <= 0
            || approval
                .expires_at
                .checked_sub(approval.reviewed_at)
                .is_none_or(|lifetime| !(1..=600).contains(&lifetime))
        {
            return Err(());
        }
        let pair = operator_key(
            &bytes(Path::new(get("--operator-key-file")?), 4096)?,
            &authorities.operator,
        )?;
        let signature = URL_SAFE_NO_PAD.encode(
            pair.sign(&canonical_json(&approval).map_err(|_| ())?)
                .as_ref(),
        );
        println!(
            "{}",
            serde_json::to_string(&SignedActivationDocument {
                document: approval,
                signature
            })
            .map_err(|_| ())?
        );
        return Ok(());
    }
    let engine: SignedActivationDocument<ActivationEngineEvidence> =
        document(get("--engine-evidence")?)?;
    let host: SignedActivationDocument<ActivationHostEvidence> = document(get("--host-evidence")?)?;
    verify_activation_qualification(&config.runtime, &authorities, &engine, &host, at()?)
        .map_err(|_| ())?;
    if command == "verify-evidence" {
        println!(
            "Evidence signatures and configured bindings verified offline; no host qualification performed; no grants changed."
        );
        return Ok(());
    }
    config.check_files()?;
    if !config.state.is_absolute() || !config.artifacts.is_absolute() || config.app_id <= 0 {
        return Err(());
    }
    // Opening an existing DB is required: typoed paths must not create a second
    // operator state store. Storage enforces its private-directory boundary.
    protected_file(&config.state, true, u64::MAX)?;
    let approval: Option<SignedActivationDocument<ActivationDocument>> = if command == "commit" {
        Some(document(get("--approval")?)?)
    } else {
        None
    };
    let (installation, repository) = if let Some(approval) = &approval {
        (
            approval.document.installation_id,
            approval.document.repository_id,
        )
    } else {
        (
            get("--installation")?.parse().map_err(|_| ())?,
            get("--repository")?.parse().map_err(|_| ())?,
        )
    };
    let scope = if let Some(approval) = &approval {
        approval.digest().map_err(|_| ())?
    } else {
        sha256(&canonical_json(&serde_json::json!({"operation":"operator-activation-preview-v1","app":config.app_id,"installation":installation,"repository":repository,"consent_revision":get("--consent-revision")?,"activation_revision":get("--activation-revision")?,"engine":engine.digest().map_err(|_| ())?,"host":host.digest().map_err(|_| ())?})).map_err(|_| ())?)
    };
    let budget = AppRequestBudget::new(
        Storage::open(&config.state, &config.artifacts).map_err(|_| ())?,
        config.app_id,
    )
    .map_err(|_| ())?;
    // Durable before provider IO; never refunded, including validation failures.
    budget
        .reserve(installation, &scope, 5, true)
        .map_err(|_| ())?;
    let broker = GitHubInstallationBroker::new(
        config.app_id,
        config.client_id.clone(),
        &bytes(config.app_private_key_file.as_deref().ok_or(())?, 16384)?,
    )
    .map_err(|_| ())?;
    let _pass = broker.begin_pass(5).map_err(|_| ())?;
    let provider = BrokerActivationProvider::new(&broker);
    let operator = AppActivationOperator::new(
        Storage::open(&config.state, &config.artifacts).map_err(|_| ())?,
        config.app_id,
        config.runtime,
        authorities,
    )
    .map_err(|_| ())?;
    if let Some(approval) = approval {
        let result = operator
            .commit(&approval, &engine, &host, &provider)
            .map_err(|_| ())?;
        println!("{}", serde_json::to_string(&result).map_err(|_| ())?);
    } else {
        let preview = operator
            .preview(
                ActivationSelection {
                    installation_id: installation,
                    repository_id: repository,
                    expected_consent_revision: get("--consent-revision")?
                        .parse()
                        .map_err(|_| ())?,
                    expected_activation_revision: get("--activation-revision")?
                        .parse()
                        .map_err(|_| ())?,
                },
                &engine,
                &host,
                &provider,
            )
            .map_err(|_| ())?;
        println!("{}", serde_json::to_string(&preview).map_err(|_| ())?);
    }
    Ok(())
}
fn main() {
    if run(&std::env::args().skip(1).collect::<Vec<_>>()).is_err() {
        eprintln!(
            "Operator command refused. Check protected configuration, exact revisions, unrevoked consent, signed evidence, approval expiry and fresh provider access. No credentials were logged."
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "csb_app_operator/tests.rs"]
mod tests;
