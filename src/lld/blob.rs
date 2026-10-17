//! Verify retained bytes against Git's object identity without a worktree read.
use crate::Cancellation;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn verify(blob: &str, source: &str, cancellation: &Cancellation) -> Result<(), String> {
    cancellation.checkpoint()?;
    if source.len() > 512 * 1024
        || !matches!(blob.len(), 40 | 64)
        || !blob.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Invalid bounded LLD blob identity".into());
    }
    if blob.len() == 64 {
        // Canonical Git object framing; the maintained SHA-256 implementation
        // is golden-tested against an actual SHA-256 Git object repository.
        let mut hash = Sha256::new();
        hash.update(format!("blob {}\0", source.len()).as_bytes());
        hash.update(source.as_bytes());
        let actual = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        return if actual == blob {
            Ok(())
        } else {
            Err("LLD source does not match its pinned Git blob".into())
        };
    }
    // GitHub currently provides SHA-1 Git object IDs. Reuse Git's maintained
    // implementation instead of introducing a hash implementation/dependency.
    // No -w, --path, repository, attributes, hooks, filters or network adapters.
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(std::path::Path::new("/"))
        .args([
            "--no-optional-locks",
            "hash-object",
            "--stdin",
            "--no-filters",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "LLD blob verifier unavailable")?;
    let mut input = child.stdin.take().ok_or("LLD verifier input unavailable")?;
    let output = child
        .stdout
        .take()
        .ok_or("LLD verifier output unavailable")?;
    let bytes = source.as_bytes().to_vec();
    let writer = std::thread::spawn(move || input.write_all(&bytes));
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        output.take(128).read_to_end(&mut bytes).map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        if cancellation.is_cancelled() || started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let _ = reader.join();
            return Err("LLD blob verification cancelled or reached its two-second bound".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                return Err("LLD blob verifier failed".into());
            }
        }
    };
    writer
        .join()
        .map_err(|_| "LLD verifier writer failed")?
        .map_err(|_| "LLD verifier write failed")?;
    let output = reader
        .join()
        .map_err(|_| "LLD verifier reader failed")?
        .map_err(|_| "LLD verifier read failed")?;
    if !status.success() || std::str::from_utf8(&output).ok().map(str::trim) != Some(blob) {
        return Err("LLD source does not match its pinned Git blob".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintained_git_verifies_exact_source_even_if_a_checksum_is_recomputed() {
        verify(
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
            "",
            &Cancellation::default(),
        )
        .unwrap();
        assert!(
            verify(
                "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
                "forged private behavior",
                &Cancellation::default()
            )
            .is_err()
        );
        let cancellation = Cancellation::default();
        cancellation.cancel();
        assert!(
            verify(
                "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
                "",
                &cancellation
            )
            .is_err()
        );
    }
    #[test]
    fn sha256_framing_matches_actual_git_for_unicode_crlf_and_null_bytes() {
        let root = std::env::temp_dir().join(format!("csb-lld-hash-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let result = Command::new("git")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "init.templateDir=",
                "init",
                "--bare",
                "--object-format=sha256",
                "-q",
            ])
            .arg(&root)
            .status()
            .unwrap();
        assert!(result.success());
        for source in ["", "fn f() { /* λ */ }\r\n", "raw\0bytes\n"] {
            let mut child = Command::new("git")
                .arg("--git-dir")
                .arg(&root)
                .args(["hash-object", "--stdin", "--no-filters"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            let blob = std::str::from_utf8(&output.stdout).unwrap().trim();
            assert_eq!(blob.len(), 64);
            verify(blob, source, &Cancellation::default()).unwrap();
            assert!(verify(blob, &format!("{source}tampered"), &Cancellation::default()).is_err());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
