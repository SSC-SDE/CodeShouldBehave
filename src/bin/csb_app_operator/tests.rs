use super::*;
use serde_json::json;

fn save<T: serde::Serialize>(path: &Path, value: &T) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}
fn sign<T: serde::Serialize>(document: T, signer: &Ed25519KeyPair) -> SignedActivationDocument<T> {
    let signature =
        URL_SAFE_NO_PAD.encode(signer.sign(&canonical_json(&document).unwrap()).as_ref());
    SignedActivationDocument {
        document,
        signature,
    }
}
#[test]
fn verify_evidence_is_offline_without_credentials_or_state() {
    let root = std::env::temp_dir().join(format!(
        "csb-operator-offline-{}-{}",
        std::process::id(),
        at().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let runtime = ActivationRuntimePins {
        authentication_origin: "https://auth.example.test".into(),
        report_origin: "https://reports.example.test".into(),
        engine_digest: "a".repeat(64),
        engine_metadata_digest: "b".repeat(64),
        launcher_digest: "c".repeat(64),
        interpreter_digest: "d".repeat(64),
        host_identity: "e".repeat(64),
        host_target: "x86_64-unknown-linux-gnu".into(),
    };
    let operator_der = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let operator = Ed25519KeyPair::from_pkcs8(operator_der.as_ref()).unwrap();
    let verifier = Ed25519KeyPair::from_seed_unchecked(&[12; 32]).unwrap();
    let qualifier = Ed25519KeyPair::from_seed_unchecked(&[13; 32]).unwrap();
    let now = at().unwrap();
    let engine = sign(
        ActivationEngineEvidence {
            schema: "csb-app-signed-engine-verification-v1".into(),
            engine_digest: runtime.engine_digest.clone(),
            engine_metadata_digest: runtime.engine_metadata_digest.clone(),
            target: runtime.host_target.clone(),
            provenance_digest: "f".repeat(64),
            verified_at: now - 1,
            expires_at: now + 3600,
        },
        &verifier,
    );
    let host = sign(
        ActivationHostEvidence {
            schema: "csb-app-host-qualification-v1".into(),
            runtime: runtime.clone(),
            engine_evidence_digest: engine.digest().unwrap(),
            sandbox_profile: "csb-linux-bwrap-prlimit-v1".into(),
            qualification_evidence_digest: "9".repeat(64),
            qualified_at: now - 1,
            expires_at: now + 3600,
        },
        &qualifier,
    );
    let config_path = root.join("config.json");
    let engine_path = root.join("engine.json");
    let host_path = root.join("host.json");
    let state = root.join("must-not-create.db");
    save(
        &config_path,
        &json!({
            "app_id":1,"client_id":"fixture","state":state,"artifacts":root.join("must-not-create-artifacts"),
            "app_private_key_file":null,"runtime":runtime,
            "operator_public_key":URL_SAFE_NO_PAD.encode(operator.public_key().as_ref()),
            "engine_verifier_public_key":URL_SAFE_NO_PAD.encode(verifier.public_key().as_ref()),
            "host_qualifier_public_key":URL_SAFE_NO_PAD.encode(qualifier.public_key().as_ref()),
            "engine_file":root.join("nonexistent-engine"),"engine_metadata_file":root.join("nonexistent-metadata"),
            "launcher_file":root.join("nonexistent-launcher"),"interpreter_file":root.join("nonexistent-interpreter")
        }),
    );
    save(&engine_path, &engine);
    save(&host_path, &host);
    let args = vec![
        "verify-evidence".into(),
        "--config".into(),
        config_path.to_str().unwrap().into(),
        "--engine-evidence".into(),
        engine_path.to_str().unwrap().into(),
        "--host-evidence".into(),
        host_path.to_str().unwrap().into(),
    ];
    assert!(run(&args).is_ok());
    assert!(!state.exists());
    assert!(!root.join("must-not-create-artifacts").exists());
    let approval = ActivationDocument {
        schema: "csb-app-operator-activation-v1".into(),
        app_id: 1,
        installation_id: 2,
        repository_id: 3,
        actor_id: 4,
        scope_revision: 1,
        scope_digest: "1".repeat(64),
        context_digest: "2".repeat(64),
        consent_revision: 1,
        consent_digest: "3".repeat(64),
        expected_activation_revision: 0,
        runtime: runtime.clone(),
        engine_evidence_digest: engine.digest().unwrap(),
        host_evidence_digest: host.digest().unwrap(),
        reviewed_at: now,
        expires_at: now + 600,
    };
    let approval_path = root.join("approval-document.json");
    let key_path = root.join("operator.der");
    save(&approval_path, &approval);
    std::fs::write(&key_path, operator_der.as_ref()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        run(&[
            "approve".into(),
            "--config".into(),
            config_path.to_str().unwrap().into(),
            "--document".into(),
            approval_path.to_str().unwrap().into(),
            "--operator-key-file".into(),
            key_path.to_str().unwrap().into()
        ])
        .is_ok()
    );
    assert!(!state.exists());
    let mut forged = host;
    forged.signature.clear();
    save(&host_path, &forged);
    assert!(run(&args).is_err());
    assert!(!state.exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn operator_parser_refuses_ambient_or_automatic_modes() {
    assert!(run(&["activate".into()]).is_err());
    assert!(run(&["commit".into(), "--automatic".into(), "true".into()]).is_err());
    assert!(
        run(&[
            "approve".into(),
            "--config".into(),
            "relative.json".into(),
            "--document".into(),
            "relative.json".into(),
            "--operator-key-file".into(),
            "relative.der".into()
        ])
        .is_err()
    );
}
#[cfg(unix)]
#[test]
fn protected_operator_input_refuses_symlinks_and_public_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = std::env::temp_dir().join(format!(
        "csb-operator-input-{}-{}",
        std::process::id(),
        at().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("config.json");
    save(&file, &json!({"fixture":true}));
    assert!(bytes(&file, 65536).is_ok());
    let link = root.join("link.json");
    symlink(&file, &link).unwrap();
    assert!(bytes(&link, 65536).is_err());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(bytes(&file, 65536).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pkcs8_versions_sign_only_for_the_independently_pinned_operator() {
    // RFC 8410 PrivateKeyInfo: Ed25519 seed without an embedded public key.
    let mut v1 = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    v1.extend_from_slice(&[11; 32]);
    let expected = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
    let public: [u8; 32] = expected.public_key().as_ref().try_into().unwrap();
    let v2 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair2 = Ed25519KeyPair::from_pkcs8(v2.as_ref()).unwrap();
    let public2: [u8; 32] = pair2.public_key().as_ref().try_into().unwrap();
    for (document, root) in [(v1.as_slice(), public), (v2.as_ref(), public2)] {
        let signer = operator_key(document, &root).unwrap();
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, root)
            .verify(
                b"exact operator approval",
                signer.sign(b"exact operator approval").as_ref(),
            )
            .unwrap();
        assert!(operator_key(document, &[0; 32]).is_err());
    }
}
