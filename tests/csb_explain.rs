#![cfg(all(feature = "ai", feature = "server"))]

use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self::build(false)
    }

    fn with_callers() -> Self {
        Self::build(true)
    }

    fn build(callers: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "csb-explain-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&root).unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.name", "csb test"]);
        fixture.git(&["config", "user.email", "test@example.invalid"]);
        if callers {
            fixture.write(
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/core\", \"crates/api\"]\nresolver = \"2\"\n",
            );
            fixture.write(
                "crates/api/Cargo.toml",
                "[package]\nname = \"api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore = { path = \"../core\" }\n",
            );
            fixture.write(
                "crates/api/src/lib.rs",
                "pub fn handle(a: i32, b: i32) -> i32 {\n    core::add(a, b)\n}\n\npub fn other(a: i32, b: i32) -> i32 {\n    core::add(b, a)\n}\n",
            );
        } else {
            fixture.write(
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/core\"]\nresolver = \"2\"\n",
            );
        }
        fixture.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        fixture.write(
            "crates/core/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        );
        fixture.commit("baseline");
        fixture.write(
            "crates/core/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a.checked_add(b).unwrap_or(0)\n}\n\npub fn subtract(a: i32, b: i32) -> i32 {\n    helper(a, b)\n}\n\nfn helper(a: i32, b: i32) -> i32 {\n    a - b\n}\n",
        );
        fixture.commit("head");
        fixture
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, path: &str, contents: &str) {
        let target = self.0.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target, contents).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().into()
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-qm", message]);
    }

    fn review(&self) -> (PathBuf, String) {
        let base = self.git(&["rev-parse", "HEAD~1"]);
        let head = self.git(&["rev-parse", "HEAD"]);
        let out = self
            .0
            .join(format!("review-{}", NEXT.fetch_add(1, Ordering::SeqCst)));
        let output = csb(&[
            "review",
            "--repo",
            self.0.to_str().unwrap(),
            "--base",
            &base,
            "--head",
            &head,
            "--out-dir",
            out.to_str().unwrap(),
            "--allow-dirty",
            "--design",
            "auto",
        ]);
        assert!(
            matches!(output.status.code(), Some(0 | 1)),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let artifact: Value =
            serde_json::from_slice(&fs::read(out.join("artifact.json")).unwrap()).unwrap();
        let boundary = artifact["hld"]["delta"]["changes"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        (out, boundary)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn csb(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_csb"))
        .args(arguments)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn serve_once(response: String) -> (String, Receiver<(String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                panic!("connection closed before headers");
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buffer.len() < header_end + length {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        let body = String::from_utf8_lossy(&buffer[header_end..header_end + length]).into_owned();
        let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        sender.send((headers, body)).unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(reply.as_bytes()).unwrap();
    });
    (format!("http://{address}"), receiver)
}

fn valid_explanation() -> String {
    json!({
        "headline": "Overflow fallback changed",
        "changes": [{"claim": "add now returns zero on overflow", "side": "proposal", "evidence_ids": [0]}],
        "questions": ["Is zero the intended fallback?"],
        "unknowns": []
    })
    .to_string()
}

#[test]
fn preview_writes_identity_without_contacting_a_provider() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &boundary,
        "--model",
        "fixture-model",
        "--endpoint",
        "http://127.0.0.1:1",
        "--preview",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let preview: Value =
        serde_json::from_slice(&fs::read(out.join("ai-preview.json")).unwrap()).unwrap();
    assert_eq!(preview["schema"], "csb-ai-preview-v1");
    assert_eq!(preview["boundary"], boundary);
    let evidence = preview["evidence"].as_array().unwrap();
    assert!(!evidence.is_empty());
    assert!(
        evidence
            .iter()
            .any(|item| item["source_kind"] == "lld_calls"),
        "missing LLD call evidence: {preview}"
    );
    assert!(!out.join("ai.md").exists());
    assert!(!out.join("ai.json").exists());
}

#[test]
fn ollama_send_writes_advisory_output_and_keeps_the_report() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let manifest_before = fs::read(out.join("manifest.json")).unwrap();
    let response = json!({"message": {"content": valid_explanation()}}).to_string();
    let (endpoint, request) = serve_once(response);
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &boundary,
        "--model",
        "fixture-model",
        "--endpoint",
        &endpoint,
        "--send",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let (_headers, raw) = request.recv().unwrap();
    let body: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], false);
    assert_eq!(body["format"]["type"], "object");
    let markdown = fs::read_to_string(out.join("ai.md")).unwrap();
    assert!(markdown.contains("Advisory AI interpretation"));
    assert!(markdown.contains("Overflow fallback changed"));
    let explanation: Value =
        serde_json::from_slice(&fs::read(out.join("ai.json")).unwrap()).unwrap();
    assert_eq!(explanation["result"]["changes"][0]["evidence_ids"][0], 0);
    assert_eq!(
        fs::read(out.join("manifest.json")).unwrap(),
        manifest_before
    );
    let verify = csb(&["verify", "--dir", out.to_str().unwrap()]);
    assert!(verify.status.success(), "{}", stderr(&verify));
}

#[test]
fn openai_send_uses_a_bearer_key_from_a_private_file() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let key_path = fixture.path().join("key.txt");
    fs::write(&key_path, "fixture-secret-key\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let content = valid_explanation();
    let response = json!({"choices": [{"message": {"content": content}}]}).to_string();
    let (endpoint, request) = serve_once(response);
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &boundary,
        "--provider",
        "openai",
        "--model",
        "fixture-model",
        "--endpoint",
        &endpoint,
        "--key-file",
        key_path.to_str().unwrap(),
        "--send",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let (headers, body) = request.recv().unwrap();
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-secret-key"),
        "{headers}"
    );
    assert!(body.contains("\"model\":\"fixture-model\""));
    assert!(out.join("ai.md").exists());
}

#[test]
fn invalid_citation_fails_with_code_five() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let content = json!({
        "headline": "Injected",
        "changes": [{"claim": "invented", "side": "proposal", "evidence_ids": [99]}],
        "questions": [],
        "unknowns": []
    })
    .to_string();
    let response = json!({"message": {"content": content}}).to_string();
    let (endpoint, _request) = serve_once(response);
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &boundary,
        "--model",
        "fixture-model",
        "--endpoint",
        &endpoint,
        "--send",
    ]);
    assert_eq!(output.status.code(), Some(5), "{}", stderr(&output));
    assert!(stderr(&output).contains("model_invalid_citation"));
    assert!(!out.join("ai.md").exists());
}

#[test]
fn truncated_provider_output_fails_with_code_five() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let response = json!({
        "done_reason": "length",
        "message": {"content": valid_explanation()}
    })
    .to_string();
    let (endpoint, _request) = serve_once(response);
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &boundary,
        "--model",
        "fixture-model",
        "--endpoint",
        &endpoint,
        "--send",
    ]);
    assert_eq!(output.status.code(), Some(5), "{}", stderr(&output));
    assert!(stderr(&output).contains("provider_output_truncated"));
    assert!(!out.join("ai.md").exists());
}

#[test]
fn explain_requires_a_design_artifact() {
    let fixture = Fixture::new();
    let base = fixture.git(&["rev-parse", "HEAD~1"]);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let out = fixture.path().join("plain");
    let output = csb(&[
        "review",
        "--repo",
        fixture.path().to_str().unwrap(),
        "--base",
        &base,
        "--head",
        &head,
        "--out-dir",
        out.to_str().unwrap(),
        "--allow-dirty",
    ]);
    assert!(matches!(output.status.code(), Some(0 | 1)));
    let explain = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        "change-0",
        "--model",
        "fixture-model",
    ]);
    assert_eq!(explain.status.code(), Some(2));
    assert!(stderr(&explain).contains("no design"));
}

fn component_id(out: &Path, name: &str) -> String {
    let artifact: Value =
        serde_json::from_slice(&fs::read(out.join("artifact.json")).unwrap()).unwrap();
    artifact["hld"]["baseline"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["kind"] == "component" && element["name"] == name)
        .and_then(|element| element["id"].as_str())
        .unwrap()
        .to_owned()
}

#[test]
fn component_preview_aggregates_relations_declarations_and_lexical_callers() {
    let fixture = Fixture::with_callers();
    let (out, _boundary) = fixture.review();
    let core = component_id(&out, "core");
    let manifest_before = fs::read(out.join("manifest.json")).unwrap();
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--component",
        &core,
        "--model",
        "fixture-model",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let preview: Value =
        serde_json::from_slice(&fs::read(out.join("ai-preview.json")).unwrap()).unwrap();
    assert_eq!(preview["boundary"], format!("component:{core}"));
    let evidence = preview["evidence"].as_array().unwrap();
    assert!(evidence.len() <= 16, "{preview}");
    assert!(
        evidence
            .iter()
            .filter(|item| item["source_kind"] == "component")
            .count()
            >= 2,
        "missing component evidence: {preview}"
    );
    assert!(
        evidence
            .iter()
            .filter(|item| item["source_kind"] == "call_candidate")
            .count()
            >= 2,
        "missing two lexical callers: {preview}"
    );
    assert!(
        evidence
            .iter()
            .any(|item| item["source_kind"] == "lld_body"),
        "missing component declarations: {preview}"
    );
    assert!(
        evidence
            .iter()
            .filter_map(|item| item["bytes"].as_u64())
            .sum::<u64>()
            <= 16 * 1024
    );
    assert_eq!(
        fs::read(out.join("manifest.json")).unwrap(),
        manifest_before
    );
    assert!(!out.join("ai.md").exists());
}

#[test]
fn component_boundary_prefix_selects_the_same_component() {
    let fixture = Fixture::with_callers();
    let (out, _boundary) = fixture.review();
    let core = component_id(&out, "core");
    let output = csb(&[
        "explain",
        "--dir",
        out.to_str().unwrap(),
        "--boundary",
        &format!("component:{core}"),
        "--model",
        "fixture-model",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let preview: Value =
        serde_json::from_slice(&fs::read(out.join("ai-preview.json")).unwrap()).unwrap();
    assert_eq!(preview["boundary"], format!("component:{core}"));
    assert!(
        preview["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["source_kind"] == "call_candidate")
    );
}

#[test]
fn list_prints_boundaries_deterministically_without_a_provider() {
    let fixture = Fixture::new();
    let (out, boundary) = fixture.review();
    let run = || csb(&["explain", "--dir", out.to_str().unwrap(), "--list"]);
    let first = run();
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let first_stdout = String::from_utf8_lossy(&first.stdout).into_owned();
    assert!(first_stdout.starts_with("id\tkind\tpath\tdeclaration\tevidence_bytes\tuncertain\n"));
    let rows = first_stdout.lines().skip(1).collect::<Vec<_>>();
    assert!(rows.iter().any(|row| row.starts_with(&boundary)));
    let mut ids = rows
        .iter()
        .map(|row| row.split('\t').next().unwrap().to_owned())
        .collect::<Vec<_>>();
    let sorted = {
        let mut sorted = ids.clone();
        sorted.sort();
        sorted
    };
    assert_eq!(ids, sorted);
    ids.clear();
    let second = run();
    assert_eq!(
        first_stdout,
        String::from_utf8_lossy(&second.stdout).into_owned()
    );
    assert!(!out.join("ai-preview.json").exists());
}

#[test]
fn explain_help_is_available() {
    let output = csb(&["explain", "--help"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("preview only"));
}
