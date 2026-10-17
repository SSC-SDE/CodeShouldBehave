use crate::{
    Action,
    analysis::{FileReview, Review, analyze, impacts},
    authorize,
    core::{ComparisonMode, ReviewRequest},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    sync::mpsc,
    thread,
    time::Duration,
};

const MAX_FILES: usize = 150;
const MAX_BLOB: usize = 512 * 1024;
static NEXT_LEASE: AtomicU64 = AtomicU64::new(0);

/// Pins immutable commits while a mirror is being read so a concurrent
/// force-push and Git maintenance cannot make their objects unreachable.
pub struct MirrorReadLease {
    repository: std::path::PathBuf,
    references: Vec<String>,
}

impl MirrorReadLease {
    pub fn acquire(repository: &Path, commits: &[String]) -> Result<Self, String> {
        if commits.is_empty() || commits.len() > 16 {
            return Err("A mirror read lease requires 1–16 commits".into());
        }
        let lease = NEXT_LEASE.fetch_add(1, Ordering::Relaxed);
        let mut references = Vec::with_capacity(commits.len());
        for (index, commit) in commits.iter().enumerate() {
            if !matches!(commit.len(), 40 | 64)
                || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                release_refs(repository, &references);
                return Err("Invalid commit ID for mirror read lease".into());
            }
            let reference = format!(
                "refs/codeshouldbehave/read-leases/{}-{lease}-{index}",
                std::process::id()
            );
            if let Err(error) = git(
                repository,
                &["update-ref", "--create-reflog", &reference, commit],
                1024,
            ) {
                release_refs(repository, &references);
                return Err(format!("Cannot acquire mirror read lease: {error}"));
            }
            references.push(reference);
        }
        Ok(Self {
            repository: repository.to_path_buf(),
            references,
        })
    }
}

impl Drop for MirrorReadLease {
    fn drop(&mut self) {
        release_refs(&self.repository, &self.references);
    }
}

fn release_refs(repository: &Path, references: &[String]) {
    for reference in references {
        let _ = git(repository, &["update-ref", "-d", reference], 1024);
    }
}

#[derive(Debug)]
pub(crate) enum BatchBlob {
    Data(Vec<u8>),
    Missing,
    Oversized(u64),
}

pub(crate) fn batch_blobs(
    repo: &Path,
    object_ids: &[String],
    max_blob_bytes: usize,
    max_total_bytes: usize,
    cancellation: &crate::core::Cancellation,
) -> Result<Vec<BatchBlob>, String> {
    if object_ids.is_empty() {
        return Ok(Vec::new());
    }
    if object_ids.len() > 256 {
        return Err("Git batch is limited to 256 objects".into());
    }
    for object_id in object_ids {
        if !matches!(object_id.len(), 40 | 64)
            || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Invalid object ID in Git batch".into());
        }
    }
    cancellation.checkpoint()?;
    let mut child = Command::new("git")
        .arg("--no-pager")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
        ])
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Cannot start Git batch reader: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("Git batch stdin unavailable")?;
    for object_id in object_ids {
        if let Err(error) = writeln!(stdin, "{object_id}") {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot submit Git batch request: {error}"));
        }
    }
    drop(stdin);
    let stdout = child.stdout.take().ok_or("Git batch stdout unavailable")?;
    let expected = object_ids.to_vec();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = parse_batch_output(
            BufReader::new(stdout),
            &expected,
            max_blob_bytes,
            max_total_bytes,
        );
        let _ = tx.send(result);
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let result = loop {
        if cancellation.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git batch read cancelled".into());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git batch read exceeded the 15 second deadline".into());
        }
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break Err("Git batch reader stopped unexpectedly".into()),
        }
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return result;
    }
    let status = child
        .wait()
        .map_err(|error| format!("Cannot reap Git batch reader: {error}"))?;
    if !status.success() {
        return Err("Git batch reader failed (object may be unavailable)".into());
    }
    result
}

fn parse_batch_output<R: BufRead>(
    mut reader: R,
    expected: &[String],
    max_blob_bytes: usize,
    max_total_bytes: usize,
) -> Result<Vec<BatchBlob>, String> {
    let mut results = Vec::with_capacity(expected.len());
    let mut total = 0_usize;
    for expected_id in expected {
        let mut header = Vec::new();
        let count = reader
            .by_ref()
            .take(257)
            .read_until(b'\n', &mut header)
            .map_err(|error| format!("Cannot read Git batch header: {error}"))?;
        if count == 0 || count > 256 || !header.ends_with(b"\n") {
            return Err("Malformed Git batch header".into());
        }
        header.pop();
        let header = std::str::from_utf8(&header).map_err(|_| "Non-UTF-8 Git batch header")?;
        if header == format!("{expected_id} missing") {
            results.push(BatchBlob::Missing);
            continue;
        }
        let parts = header.split_ascii_whitespace().collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != expected_id || parts[1] != "blob" {
            return Err("Unexpected Git batch object header".into());
        }
        let size = parts[2]
            .parse::<u64>()
            .map_err(|_| "Invalid Git batch object length")?;
        let size_usize = usize::try_from(size).map_err(|_| "Git object length is too large")?;
        total = total
            .checked_add(size_usize)
            .ok_or("Git batch byte count overflow")?;
        if total > max_total_bytes {
            return Err("Git batch exceeds the total byte limit".into());
        }
        if size_usize > max_blob_bytes {
            let discarded = std::io::copy(&mut reader.by_ref().take(size), &mut std::io::sink())
                .map_err(|error| format!("Cannot discard oversized Git object: {error}"))?;
            if discarded != size {
                return Err("Truncated oversized Git object".into());
            }
            let mut delimiter = [0_u8; 1];
            reader
                .read_exact(&mut delimiter)
                .map_err(|_| "Truncated oversized Git object")?;
            if delimiter != [b'\n'] {
                return Err("Malformed Git batch object delimiter".into());
            }
            results.push(BatchBlob::Oversized(size));
            continue;
        }
        let mut bytes = vec![0_u8; size_usize];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| "Truncated Git batch object")?;
        let mut delimiter = [0_u8; 1];
        reader
            .read_exact(&mut delimiter)
            .map_err(|_| "Missing Git batch object delimiter")?;
        if delimiter != [b'\n'] {
            return Err("Malformed Git batch object delimiter".into());
        }
        results.push(BatchBlob::Data(bytes));
    }
    let mut trailing = [0_u8; 1];
    if reader
        .read(&mut trailing)
        .map_err(|error| error.to_string())?
        != 0
    {
        return Err("Unexpected trailing Git batch output".into());
    }
    Ok(results)
}

pub(crate) fn git(repo: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("--no-pager")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
        ])
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start Git: {e}"))?;
    let stdout = child.stdout.take().ok_or("Git stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(Duration::from_secs(15));
    let bytes = match result {
        Ok(Ok(bytes)) if bytes.len() <= limit => bytes,
        other => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(match other {
                Ok(Ok(_)) => format!("Git output exceeds {} KiB safety limit", limit / 1024),
                _ => "Git read failed or exceeded the 15 second deadline".into(),
            });
        }
    };
    // Poll the process as well: closing stdout must not bypass the deadline.
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(bytes),
            Some(_) => {
                return Err(format!(
                    "Git {} failed (check repository, refs, and local object availability)",
                    args.first().unwrap_or(&"command")
                ));
            }
            None if start.elapsed() < Duration::from_secs(2) => {
                thread::sleep(Duration::from_millis(10))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Git process deadline exceeded".into());
            }
        }
    }
}

pub(crate) fn resolve(repo: &Path, reference: &str) -> Result<String, String> {
    let commit = format!("{reference}^{{commit}}");
    let bytes = git(
        repo,
        &["rev-parse", "--verify", "--end-of-options", &commit],
        256,
    )?;
    let hash = String::from_utf8(bytes)
        .map_err(|_| "Invalid Git object ID")?
        .trim()
        .to_owned();
    if !(hash.len() == 40 || hash.len() == 64) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid commit ID".into());
    }
    Ok(hash)
}

pub fn review(
    request: &ReviewRequest,
    cancellation: &crate::core::Cancellation,
) -> Result<Review, String> {
    let mut base = resolve(&request.repository, &request.baseline)?;
    let head = resolve(&request.repository, &request.proposal)?;
    if request.comparison == ComparisonMode::MergeBase {
        base = String::from_utf8(git(
            &request.repository,
            &["merge-base", &base, &head],
            256,
        )?)
        .map_err(|_| "Invalid merge base")?
        .trim()
        .into();
    }
    let names = git(
        &request.repository,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--raw",
            "--no-abbrev",
            "-z",
            &base,
            &head,
            "--",
        ],
        1024 * 1024,
    )?;
    let parts: Vec<&[u8]> = names.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    if !parts.len().is_multiple_of(2) {
        return Err("Unexpected Git raw diff output".into());
    }
    let mut result = Review {
        architecture: crate::architecture::Architecture::default(),
        change_set: None,
        context: vec![],
        title: request
            .repository
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| *s != ".")
            .unwrap_or("Repository review")
            .into(),
        base,
        head,
        files: vec![],
        impacts: vec![],
        warnings: vec![],
    };
    if parts.len() / 2 > MAX_FILES {
        result.warnings.push(format!(
            "Only the first {MAX_FILES} of {} changed files are included.",
            parts.len() / 2
        ));
    }
    struct ChangedFile {
        status: String,
        path: String,
        old_oid: Option<String>,
        new_oid: Option<String>,
        old_issue: Option<&'static str>,
        new_issue: Option<&'static str>,
    }
    let mut changes = Vec::new();
    let mut object_ids = std::collections::BTreeSet::new();
    for pair in parts.chunks_exact(2).take(MAX_FILES) {
        let path = match std::str::from_utf8(pair[1]) {
            Ok(p) => p,
            Err(_) => {
                result.warnings.push("A non-UTF-8 path was omitted.".into());
                continue;
            }
        };
        if !request.scope.primary_paths.allows(path) || crate::sensitive_path(path) {
            continue;
        }
        if authorize(
            Action::Source,
            &request.principal,
            &request.scope,
            &request.current_grants,
            Some(path),
        )
        .is_err()
        {
            continue;
        }
        let header = std::str::from_utf8(pair[0]).map_err(|_| "Invalid Git raw diff header")?;
        let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 5 || !fields[0].starts_with(':') {
            return Err("Malformed Git raw diff header".into());
        }
        let status = fields[4]
            .chars()
            .next()
            .ok_or("Missing Git change status")?;
        if !matches!(status, 'A' | 'D' | 'M' | 'T' | 'U') {
            return Err("Unsupported Git change status".into());
        }
        let valid_oid = |oid: &str| {
            matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        };
        if !valid_oid(fields[2]) || !valid_oid(fields[3]) {
            return Err("Invalid object ID in Git raw diff".into());
        }
        let regular = |mode: &str| matches!(mode.trim_start_matches(':'), "100644" | "100755");
        let old_issue = (status != 'A' && !regular(fields[0]))
            .then_some("symlink or submodule content omitted");
        let new_issue = (status != 'D' && !regular(fields[1]))
            .then_some("symlink or submodule content omitted");
        let old_oid =
            (status != 'A' && old_issue.is_none()).then(|| fields[2].to_ascii_lowercase());
        let new_oid =
            (status != 'D' && new_issue.is_none()).then(|| fields[3].to_ascii_lowercase());
        if let Some(oid) = &old_oid {
            object_ids.insert(oid.clone());
        }
        if let Some(oid) = &new_oid {
            object_ids.insert(oid.clone());
        }
        changes.push(ChangedFile {
            status: status.to_string(),
            path: path.into(),
            old_oid,
            new_oid,
            old_issue,
            new_issue,
        });
    }
    let object_ids = object_ids.into_iter().collect::<Vec<_>>();
    let mut objects = std::collections::BTreeMap::new();
    for chunk in object_ids.chunks(128) {
        let values = batch_blobs(
            &request.repository,
            chunk,
            MAX_BLOB,
            64 * 1024 * 1024,
            cancellation,
        )?;
        objects.extend(chunk.iter().cloned().zip(values));
    }
    let source = |oid: Option<&String>| -> Result<String, String> {
        let Some(oid) = oid else {
            return Ok(String::new());
        };
        match objects
            .get(oid)
            .ok_or("Git batch omitted a requested object")?
        {
            BatchBlob::Data(bytes) if bytes.contains(&0) => Err("Binary content omitted".into()),
            BatchBlob::Data(bytes) => {
                String::from_utf8(bytes.clone()).map_err(|_| "Non-UTF-8 content omitted".into())
            }
            BatchBlob::Missing => Err("Git object is missing".into()),
            BatchBlob::Oversized(size) => Err(format!(
                "{size}-byte content exceeds the 512 KiB reader limit"
            )),
        }
    };
    for change in changes {
        let path = change.path;
        let status = change.status;
        let mut notes = vec![];
        if let Some(issue) = change.old_issue {
            notes.push(format!("Before: {issue}"));
        }
        if let Some(issue) = change.new_issue {
            notes.push(format!("After: {issue}"));
        }
        let before = source(change.old_oid.as_ref()).unwrap_or_else(|e| {
            notes.push(format!("Before: {e}"));
            String::new()
        });
        let after = source(change.new_oid.as_ref()).unwrap_or_else(|e| {
            notes.push(format!("After: {e}"));
            String::new()
        });
        let content_available = notes.is_empty();
        if !content_available {
            notes.push("Flow comparison unavailable: at least one revision could not be read as text. No flow or call changes are inferred for this file.".into());
        }
        let patch = git(
            &request.repository,
            &[
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-color",
                "--unified=3",
                &result.base,
                &result.head,
                "--",
                &path,
            ],
            256 * 1024,
        )
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_else(|e| {
            notes.push(e);
            String::new()
        });
        let mut file = FileReview::new(
            path.clone(),
            status,
            analyze(&path, if content_available { &before } else { "" }),
            analyze(&path, if content_available { &after } else { "" }),
            patch,
        );
        file.notes.extend(notes);
        result.files.push(file);
    }
    result.impacts = impacts(&result.files);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn oid(byte: char) -> String {
        std::iter::repeat_n(byte, 40).collect()
    }

    #[test]
    fn batch_parser_uses_exact_lengths_and_preserves_binary_bytes() {
        let first = oid('a');
        let second = oid('b');
        let bytes = format!("{first} blob 4\n").into_bytes();
        let mut output = bytes;
        output.extend_from_slice(&[0, b'\n', 255, b'x']);
        output.push(b'\n');
        output.extend_from_slice(format!("{second} missing\n").as_bytes());
        let parsed = parse_batch_output(Cursor::new(output), &[first, second], 8, 16).unwrap();
        assert!(matches!(&parsed[0], BatchBlob::Data(value) if value == &[0, b'\n', 255, b'x']));
        assert!(matches!(parsed[1], BatchBlob::Missing));
    }

    #[test]
    fn batch_parser_rejects_malformed_or_truncated_output() {
        let object = oid('c');
        let wrong = oid('d');
        assert!(
            parse_batch_output(
                Cursor::new(format!("{wrong} blob 1\nx\n")),
                std::slice::from_ref(&object),
                8,
                16,
            )
            .is_err()
        );
        assert!(
            parse_batch_output(
                Cursor::new(format!("{object} blob 3\nx\n")),
                &[object],
                8,
                16,
            )
            .is_err()
        );
    }
}
