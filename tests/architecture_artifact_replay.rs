#![cfg(feature = "server")]

//! Immutable payload regression checks. Fixtures are inert, local Git objects.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "csb-artifact-replay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("repo")).unwrap();
        fs::create_dir(root.join("repo/src")).unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "--template="]);
        fixture.git(&["config", "user.name", "Replay fixture"]);
        fixture.git(&["config", "user.email", "replay@example.invalid"]);
        fixture
    }
    fn repository(&self) -> PathBuf {
        self.0.join("repo")
    }
    fn git(&self, arguments: &[&str]) {
        let result = clean_command("git")
            .arg("-C")
            .arg(self.repository())
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(arguments)
            .output()
            .unwrap();
        assert!(result.status.success(), "Git fixture setup failed");
    }
    fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-qm", message]);
    }
    fn review(&self, iteration: usize, cache: &Path) -> (Output, Vec<u8>) {
        let artifact = self.0.join(format!("review-{iteration}.json"));
        let result = clean_command(env!("CARGO_BIN_EXE_flowdiff"))
            .args([
                "review",
                "--github-repository",
                "owner/repo",
                "--base",
                "HEAD~1",
                "--head",
                "HEAD",
                "--repo",
            ])
            .arg(self.repository())
            .arg("--cache-dir")
            .arg(cache)
            .arg("--artifact")
            .arg(&artifact)
            .arg("--output")
            .arg(self.0.join(format!("review-{iteration}.html")))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        (result, fs::read(artifact).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn clean_command(executable: &str) -> Command {
    let mut command = Command::new(executable);
    command.env_clear();
    for key in ["PATH", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

#[test]
fn cold_partial_warm_and_warm_artifacts_keep_parser_recovery_evidence() {
    let fixture = Fixture::new();
    fs::write(
        fixture.repository().join("src/a.rs"),
        "fn broken( { old(); }\n",
    )
    .unwrap();
    fs::write(
        fixture.repository().join("src/z.rs"),
        "fn other( { old(); }\n",
    )
    .unwrap();
    fixture.commit("baseline with recovered syntax");
    fs::write(
        fixture.repository().join("src/a.rs"),
        "fn broken( { new(); }\n",
    )
    .unwrap();
    fs::write(
        fixture.repository().join("src/z.rs"),
        "fn other( { new(); }\n",
    )
    .unwrap();
    fixture.commit("proposal with recovered syntax");
    let cache = fixture.0.join("cache");
    let (cold, first) = fixture.review(1, &cache);
    let (warm, second) = fixture.review(2, &cache);
    let artifact: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(
        first, second,
        "Warm reads must not remove parser quality warnings"
    );
    assert_eq!(artifact["warnings"].as_array().unwrap().len(), 4);
    assert!(
        artifact["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| warning.as_str().unwrap().contains("Rust parser recovered"))
    );
    assert_ne!(
        cold.stdout, warm.stdout,
        "Exercise different cache execution paths"
    );
    let mut cache_files = Vec::new();
    for directory in fs::read_dir(&cache).unwrap() {
        for record in fs::read_dir(directory.unwrap().path()).unwrap() {
            let path = record.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "bin") {
                cache_files.push(path);
            }
        }
    }
    assert_eq!(cache_files.len(), 4);
    fs::remove_file(&cache_files[0]).unwrap();
    let (_, partially_warm) = fixture.review(3, &cache);
    assert_eq!(
        first, partially_warm,
        "Partial cache hits must preserve warning order"
    );
}

#[test]
fn pristine_rust_artifact_is_byte_identical_after_cache_reuse() {
    let fixture = Fixture::new();
    fs::write(
        fixture.repository().join("src/lib.rs"),
        "fn run() { old(); }\nfn old() {}\n",
    )
    .unwrap();
    fixture.commit("valid baseline");
    fs::write(
        fixture.repository().join("src/lib.rs"),
        "fn run() { new(); }\nfn new() {}\n",
    )
    .unwrap();
    fixture.commit("valid proposal");
    let cache = fixture.0.join("cache");
    let (cold, first) = fixture.review(1, &cache);
    let (warm, second) = fixture.review(2, &cache);
    assert_eq!(first, second);
    let artifact: serde_json::Value = serde_json::from_slice(&second).unwrap();
    assert!(artifact["warnings"].as_array().unwrap().is_empty());
    assert_ne!(cold.stdout, warm.stdout);
}
