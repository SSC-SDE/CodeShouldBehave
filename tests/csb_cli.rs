use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "csb-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn join(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Repo {
    scratch: Scratch,
}

impl Repo {
    #[cfg(feature = "server")]
    fn bare() -> Self {
        let scratch = Scratch::new();
        let repo = Self { scratch };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "csb test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo
    }

    fn new() -> Self {
        let scratch = Scratch::new();
        let repo = Self { scratch };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "csb test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.write(
            "src/lib.rs",
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        );
        repo.write(
            "src/main.rs",
            "fn main() {\n    println!(\"{}\", add(1, 2));\n}\n",
        );
        repo.commit("initial");
        repo
    }
    fn path(&self) -> &Path {
        &self.scratch.0
    }
    fn write(&self, path: &str, contents: &str) {
        let target = self.scratch.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target, contents).unwrap();
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-C")
            .arg(&self.scratch.0)
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
    fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["-c", "core.hooksPath=/dev/null", "commit", "-qm", message]);
        self.git(&["rev-parse", "HEAD"])
    }
    fn change_and_commit(&self) -> String {
        self.write(
            "src/lib.rs",
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn multiply(a: i32, b: i32) -> i32 {\n    a * b\n}\n",
        );
        self.commit("add multiply")
    }

    #[cfg(feature = "server")]
    fn cargo_workspace(&self) -> String {
        self.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/core\", \"crates/app\"]\nresolver = \"2\"\n",
        );
        self.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        self.write(
            "crates/app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ncore = { path = \"../core\" }\n",
        );
        self.write(
            "crates/core/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        );
        self.write(
            "crates/app/src/lib.rs",
            "use core::add;\npub fn total() -> i32 {\n    add(1, 2)\n}\n",
        );
        self.commit("workspace baseline")
    }

    #[cfg(feature = "server")]
    fn change_workspace(&self) -> String {
        self.write(
            "crates/core/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a.checked_add(b).unwrap_or(0)\n}\n\npub fn subtract(a: i32, b: i32) -> i32 {\n    a - b\n}\n",
        );
        self.write(
            "crates/app/src/lib.rs",
            "use core::{add, subtract};\npub fn total() -> i32 {\n    add(1, 2) - subtract(3, 1)\n}\n",
        );
        self.commit("change workspace")
    }
}

fn csb(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_csb"))
        .args(arguments)
        .output()
        .unwrap()
}

fn review(repo: &Repo, base: &str, head: &str, out: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_csb"));
    command
        .arg("review")
        .arg("--repo")
        .arg(repo.path())
        .arg("--base")
        .arg(base)
        .arg("--head")
        .arg(head)
        .arg("--out-dir")
        .arg(out)
        .args(extra);
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_and_version_are_available() {
    let help = csb(&["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("Usage:"));
    let version = csb(&["version"]);
    assert!(version.status.success());
    assert!(stdout(&version).starts_with("csb "));
    let review_help = csb(&["review", "--help"]);
    assert!(review_help.status.success());
    assert!(stdout(&review_help).contains("csb review"));
}

#[test]
fn review_requires_arguments_and_a_repository() {
    let missing = csb(&["review"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("--repo"));
    let missing_repo = csb(&[
        "review",
        "--repo",
        "/definitely/not/here",
        "--base",
        &"a".repeat(40),
        "--head",
        &"b".repeat(40),
        "--out-dir",
        "/tmp/csb-nope",
    ]);
    assert_eq!(missing_repo.status.code(), Some(2));
}

#[test]
fn symbolic_short_and_equal_commits_are_rejected() {
    let repo = Repo::new();
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    for input in ["HEAD", "main", "HEAD~1", &head[..12]] {
        let output = review(&repo, input, &head, &scratch.join("out"), &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{input}: {}",
            stderr(&output)
        );
        assert!(stderr(&output).contains("exact 40-character"));
    }
    let same = review(&repo, &head, &head, &scratch.join("out"), &[]);
    assert_eq!(same.status.code(), Some(2));
    assert!(stderr(&same).contains("must differ"));
}

#[test]
fn non_commit_object_is_a_git_error() {
    let repo = Repo::new();
    let head = repo.change_and_commit();
    let blob = repo.git(&["rev-parse", "HEAD:src/lib.rs"]);
    let scratch = Scratch::new();
    let output = review(&repo, &blob, &head, &scratch.join("out"), &[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(stderr(&output).contains("not a commit"));
}

#[test]
fn dirty_worktree_is_rejected_unless_allowed() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    fs::write(repo.path().join("src/lib.rs"), "fn dirty() {}\n").unwrap();
    let scratch = Scratch::new();
    let rejected = review(&repo, &base, &head, &scratch.join("blocked"), &[]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(stderr(&rejected).contains("dirty"));
    assert!(!scratch.join("blocked").exists());
    let allowed = review(
        &repo,
        &base,
        &head,
        &scratch.join("out"),
        &["--allow-dirty"],
    );
    assert_eq!(allowed.status.code(), Some(0), "{}", stderr(&allowed));
}

#[test]
fn existing_output_is_never_overwritten() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let out = scratch.join("out");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("keep.txt"), "keep\n").unwrap();
    let output = review(&repo, &base, &head, &out, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("already exists"));
    assert_eq!(fs::read_to_string(out.join("keep.txt")).unwrap(), "keep\n");
}

#[test]
fn empty_existing_output_directory_is_allowed() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let out = scratch.join("out");
    fs::create_dir_all(&out).unwrap();
    let output = review(&repo, &base, &head, &out, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(out.join("index.html").is_file());
}

#[test]
fn review_emits_index_manifest_sources_and_summary() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let out = scratch.join("out");
    let output = review(&repo, &base, &head, &out, &["--summary", "check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("csb: "));
    for file in [
        "index.html",
        "index.json",
        "manifest.json",
        "review.json",
        "review.md",
        "summary.md",
    ] {
        assert!(out.join(file).is_file(), "missing {file}");
    }
    let sources = fs::read_dir(out.join("sources"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(sources.len(), 1);
    let summary = fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(summary.contains("## CodeShouldBehave Review"));
    assert!(summary.contains("### Behavioral changes"));
    assert!(summary.contains("multiply"));
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["schema"], "csb-local-review-v1");
    assert_eq!(index["verdict"]["kind"], "change_detected");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "csb-local-manifest-v1");
    assert!(!manifest["content_sha256"].as_str().unwrap().is_empty());
}

#[test]
fn output_is_byte_identical_across_runs() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let first = scratch.join("first");
    let second = scratch.join("second");
    assert_eq!(
        review(&repo, &base, &head, &first, &[]).status.code(),
        Some(0)
    );
    assert_eq!(
        review(&repo, &base, &head, &second, &[]).status.code(),
        Some(0)
    );
    let first_manifest = fs::read_to_string(first.join("manifest.json")).unwrap();
    let second_manifest = fs::read_to_string(second.join("manifest.json")).unwrap();
    assert_eq!(first_manifest, second_manifest);
    assert_eq!(
        fs::read(first.join("index.html")).unwrap(),
        fs::read(second.join("index.html")).unwrap()
    );
    assert_eq!(
        fs::read(first.join("review.json")).unwrap(),
        fs::read(second.join("review.json")).unwrap()
    );
}

#[test]
fn verify_accepts_intact_output_and_rejects_tampering() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let out = scratch.join("out");
    assert_eq!(
        review(&repo, &base, &head, &out, &[]).status.code(),
        Some(0)
    );
    let ok = csb(&["verify", "--dir", out.to_str().unwrap()]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));
    assert!(stdout(&ok).starts_with("OK "));
    let mut index = fs::read(out.join("index.json")).unwrap();
    index.push(b' ');
    fs::write(out.join("index.json"), index).unwrap();
    let tampered = csb(&["verify", "--dir", out.to_str().unwrap()]);
    assert_eq!(tampered.status.code(), Some(1));
    assert!(stderr(&tampered).contains("content_sha256"));
}

#[test]
fn summary_formats_are_machine_readable() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let markdown = review(
        &repo,
        &base,
        &head,
        &scratch.join("markdown"),
        &["--summary", "markdown"],
    );
    assert_eq!(markdown.status.code(), Some(0));
    assert!(stdout(&markdown).contains("**Baseline:**"));
    let json = review(
        &repo,
        &base,
        &head,
        &scratch.join("json"),
        &["--summary", "json"],
    );
    assert_eq!(json.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_str(stdout(&json).trim()).unwrap();
    assert_eq!(value["verdict"]["kind"], "change_detected");
    assert_eq!(value["counts"]["files"], 1);
}

#[test]
fn unsupported_language_exits_one_and_marks_incomplete() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    repo.write("script.go", "package main\n");
    let head = repo.commit("add go");
    let scratch = Scratch::new();
    let out = scratch.join("out");
    let output = review(&repo, &base, &head, &out, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let summary = fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(summary.contains("Coverage is incomplete"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["incomplete"], true);
    let verified = csb(&["verify", "--dir", out.to_str().unwrap()]);
    assert_eq!(verified.status.code(), Some(0));
    assert!(stderr(&verified).contains("incomplete"));
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(directory: &Path) -> (Server, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_csb"))
        .args(["serve", "--dir", directory.to_str().unwrap(), "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let url = line.trim().to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "server said: {url}");
    (Server(child), url)
}

fn get(url: &str, path: &str) -> String {
    let address = url.trim_start_matches("http://").trim_end_matches('/');
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

#[test]
fn serve_returns_the_generated_site() {
    let repo = Repo::new();
    let base = repo.git(&["rev-parse", "HEAD"]);
    let head = repo.change_and_commit();
    let scratch = Scratch::new();
    let out = scratch.join("out");
    assert_eq!(
        review(&repo, &base, &head, &out, &[]).status.code(),
        Some(0)
    );
    let (_server, url) = start_server(&out);
    let index = get(&url, "/");
    assert!(index.starts_with("HTTP/1.1 200 OK"), "{index}");
    assert!(index.contains("CodeShouldBehave"));
    let json = get(&url, "/index.json");
    assert!(json.contains("csb-local-review-v1"));
    let traversal = get(&url, "/%2e%2e/manifest.json");
    assert!(traversal.starts_with("HTTP/1.1 404"), "{traversal}");
}

#[test]
fn serve_rejects_directories_without_review_output() {
    let scratch = Scratch::new();
    let output = csb(&["serve", "--dir", scratch.0.to_str().unwrap(), "--port", "0"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("run `csb review` first"));
}

#[cfg(feature = "server")]
#[cfg(feature = "server")]
#[cfg(feature = "server")]
#[cfg(feature = "server")]