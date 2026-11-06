//! Scoped immutable object acquisition. No checkout, archive, patch or Contents API.
use super::*;
use crate::github_app_setup::pr_worker::{MAX_PR_BLOBS, MAX_PR_INPUT, object, object_id};
use crate::github_app_setup::{GitObject, PrWorkerRequest};
use std::collections::{BTreeMap, BTreeSet};
impl InstallationSession<'_> {
    pub fn fetch_pr_objects(
        &self,
        input: &super::super::AnalysisInput,
        policy: crate::public_report::PublicPolicy,
        context: Value,
    ) -> Result<PrWorkerRequest> {
        if input.installation_id != self.credential.installation
            || input.repository_id != self.credential.repository
            || input.app_id != self.broker.app_id
            || policy.repository.id != input.repository_id as u64
            || policy.repository.owner != self.owner
            || policy.repository.name != self.name
        {
            return Err(SetupError::Denied);
        }
        let scope = policy.validate().map_err(|_| SetupError::Denied)?;
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        let start = Instant::now();
        let mut calls = 0;
        let mut total = 0;
        let mut get = |path: String| -> Result<Value> {
            self.credential.check(super::super::now()?)?;
            if calls >= 70 || start.elapsed() > Duration::from_secs(45) {
                return Err(SetupError::Capacity);
            }
            calls += 1;
            let value = self
                .broker
                .request("GET", &path, &self.credential.token, None)?;
            total += serde_json::to_vec(&value)
                .map_err(|_| SetupError::Invalid)?
                .len();
            if total > MAX_PR_INPUT {
                return Err(SetupError::Capacity);
            }
            Ok(value)
        };
        let prefix = format!("/repos/{}/{}", self.owner, self.name);
        let mut objects = BTreeMap::new();
        let mut blobs = BTreeSet::new();
        for sha in BTreeSet::from([&input.comparison.merge_base, &input.comparison.head]) {
            if !super::super::scope::oid(sha) {
                return Err(SetupError::Invalid);
            }
            let commit = get(format!("{prefix}/git/commits/{sha}"))?;
            let raw = commit_bytes(&commit, sha)?;
            objects.insert(sha.clone(), object("commit", &raw));
            let root = oid(&commit["tree"]["sha"])?;
            let tree = get(format!("{prefix}/git/trees/{root}?recursive=1"))?;
            let rebuilt = tree_objects(&tree, &root)?;
            let entries = tree["tree"].as_array().ok_or(SetupError::Invalid)?;
            for entry in entries {
                let path = entry["path"].as_str().ok_or(SetupError::Invalid)?;
                if matches!(entry["mode"].as_str(), Some("100644" | "100755"))
                    && scope.allows(path)
                    && !crate::sensitive_path(path)
                {
                    if entry["size"].as_u64().is_none_or(|n| n > 256 * 1024) {
                        return Err(SetupError::Capacity);
                    }
                    blobs.insert(oid(&entry["sha"])?);
                }
            }
            objects.extend(rebuilt.into_iter().map(|o| (o.oid.clone(), o)));
        }
        if blobs.len() > MAX_PR_BLOBS {
            return Err(SetupError::Capacity);
        }
        for sha in blobs {
            let value = get(format!("{prefix}/git/blobs/{sha}"))?;
            if value["sha"] != sha
                || value["encoding"] != "base64"
                || value["size"].as_u64().is_none_or(|n| n > 256 * 1024)
            {
                return Err(SetupError::Invalid);
            }
            let encoded = value["content"].as_str().ok_or(SetupError::Invalid)?;
            let bytes = STANDARD
                .decode(
                    encoded
                        .bytes()
                        .filter(|b| !b.is_ascii_whitespace())
                        .collect::<Vec<_>>(),
                )
                .map_err(|_| SetupError::Invalid)?;
            if bytes.len() > 256 * 1024
                || value["size"].as_u64() != Some(bytes.len() as u64)
                || object_id("blob", &bytes) != sha
            {
                return Err(SetupError::Invalid);
            }
            objects.insert(sha, object("blob", &bytes));
        }
        self.credential.check(super::super::now()?)?;
        let request = PrWorkerRequest {
            input: input.clone(),
            policy,
            context,
            objects: objects.into_values().collect(),
        };
        if serde_json::to_vec(&request)
            .map_err(|_| SetupError::Invalid)?
            .len()
            > MAX_PR_INPUT
        {
            return Err(SetupError::Capacity);
        }
        Ok(request)
    }
}
fn identity(value: &Value) -> Result<String> {
    let name = value["name"].as_str().ok_or(SetupError::Invalid)?;
    let email = value["email"].as_str().ok_or(SetupError::Invalid)?;
    if name.len() > 512
        || email.len() > 512
        || name.contains(['\n', '\r', '\0', '<', '>'])
        || email.contains(['\n', '\r', '\0', '<', '>'])
    {
        return Err(SetupError::Invalid);
    }
    let at = utc_seconds(value["date"].as_str().ok_or(SetupError::Invalid)?)?;
    Ok(format!("{name} <{email}> {at} +0000"))
}
/// GitHub exposes signature payloads, not a generic raw-commit endpoint. Rebuild
/// the conventional signed form (or UTC unsigned form), then require its exact
/// object hash. Unrepresentable encodings/timezones/extensions fail closed.
fn commit_bytes(value: &Value, expected: &str) -> Result<Vec<u8>> {
    if value["sha"] != expected {
        return Err(SetupError::Invalid);
    }
    let root = oid(&value["tree"]["sha"])?;
    if let (Some(payload), Some(signature)) = (
        value["verification"]["payload"].as_str(),
        value["verification"]["signature"].as_str(),
    ) {
        if payload.len() + signature.len() > 256 * 1024
            || !payload.starts_with(&format!("tree {root}\n"))
        {
            return Err(SetupError::Invalid);
        }
        let end = payload.find("\n\n").ok_or(SetupError::Invalid)?;
        let header = format!(
            "gpgsig {}\n",
            signature.trim_end_matches('\n').replace('\n', "\n ")
        );
        let mut positions = vec![end + 1];
        positions.extend(payload[..end].match_indices('\n').map(|(i, _)| i + 1));
        if positions.len() > 32 {
            return Err(SetupError::Capacity);
        }
        for at in positions {
            let mut bytes = payload.as_bytes()[..at].to_vec();
            bytes.extend_from_slice(header.as_bytes());
            bytes.extend_from_slice(&payload.as_bytes()[at..]);
            if object_id("commit", &bytes) == expected {
                return Ok(bytes);
            }
        }
        return Err(SetupError::Invalid);
    }
    let parents = value["parents"]
        .as_array()
        .filter(|p| p.len() <= 16)
        .ok_or(SetupError::Invalid)?;
    let mut text = format!("tree {root}\n");
    for p in parents {
        text.push_str(&format!("parent {}\n", oid(&p["sha"])?));
    }
    text.push_str(&format!(
        "author {}\ncommitter {}\n\n{}",
        identity(&value["author"])?,
        identity(&value["committer"])?,
        value["message"]
            .as_str()
            .filter(|s| s.len() < 128 * 1024)
            .ok_or(SetupError::Invalid)?
    ));
    for candidate in [text.clone(), format!("{text}\n")] {
        if object_id("commit", candidate.as_bytes()) == expected {
            return Ok(candidate.into_bytes());
        }
    }
    Err(SetupError::Invalid)
}
fn tree_objects(value: &Value, root: &str) -> Result<Vec<GitObject>> {
    if value["sha"] != root || value["truncated"].as_bool() != Some(false) {
        return Err(SetupError::Capacity);
    }
    let entries = value["tree"]
        .as_array()
        .filter(|a| a.len() <= 2000)
        .ok_or(SetupError::Capacity)?;
    let mut expected = BTreeMap::from([(String::new(), root.to_owned())]);
    let mut groups: BTreeMap<String, Vec<(String, String, String)>> =
        BTreeMap::from([(String::new(), vec![])]);
    let mut seen = BTreeSet::new();
    for entry in entries {
        let path = entry["path"].as_str().ok_or(SetupError::Invalid)?;
        if path.len() > 256
            || path.starts_with('/')
            || path
                .split('/')
                .any(|s| s.is_empty() || matches!(s, "." | ".." | ".git"))
            || path.chars().any(char::is_control)
            || !seen.insert(path)
        {
            return Err(SetupError::Invalid);
        }
        let mode = entry["mode"].as_str().ok_or(SetupError::Invalid)?;
        let kind = entry["type"].as_str().ok_or(SetupError::Invalid)?;
        if !matches!(
            (mode, kind),
            ("040000", "tree") | ("100644" | "100755" | "120000", "blob") | ("160000", "commit")
        ) {
            return Err(SetupError::Invalid);
        }
        let sha = oid(&entry["sha"])?;
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        groups.entry(parent.into()).or_default().push((
            name.into(),
            mode.trim_start_matches('0').into(),
            sha.clone(),
        ));
        if kind == "tree" {
            expected.insert(path.into(), sha);
            groups.entry(path.into()).or_default();
        }
    }
    let mut result = vec![];
    for (path, mut entries) in groups {
        let expected = expected.get(&path).ok_or(SetupError::Invalid)?;
        entries.sort_by_key(|(name, mode, _)| {
            format!("{name}{}", if mode == "40000" { "/" } else { "" })
        });
        let mut bytes = vec![];
        for (name, mode, sha) in entries {
            bytes.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            for i in (0..40).step_by(2) {
                bytes
                    .push(u8::from_str_radix(&sha[i..i + 2], 16).map_err(|_| SetupError::Invalid)?);
            }
        }
        let obj = object("tree", &bytes);
        if &obj.oid != expected {
            return Err(SetupError::Invalid);
        }
        result.push(obj);
    }
    Ok(result)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::super::pipeline_tests::Fixture;
    use super::*;
    use std::sync::Arc;
    pub struct Responses {
        pub replies: Mutex<BTreeMap<String, Value>>,
        pub seen: Mutex<Vec<String>>,
    }
    impl Transport for Arc<Responses> {
        fn request(
            &self,
            method: &str,
            path: &str,
            token: &str,
            _: Option<&Value>,
        ) -> Result<Value> {
            assert_eq!(method, "GET");
            assert!(!token.is_empty());
            self.seen.lock().unwrap().push(path.into());
            self.replies
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or(SetupError::Unavailable)
        }
    }
    fn git(f: &Fixture, args: &[&str]) -> Vec<u8> {
        let r = std::process::Command::new("git")
            .arg("-C")
            .arg(&f.root)
            .args(args)
            .output()
            .unwrap();
        assert!(r.status.success());
        r.stdout
    }
    pub fn fixture_replies(f: &Fixture) -> BTreeMap<String, Value> {
        let mut result = BTreeMap::new();
        let prefix = "/repos/fixture/app";
        for commit in [
            &f.request.input.comparison.merge_base,
            &f.request.input.comparison.head,
        ] {
            let raw = String::from_utf8(git(f, &["cat-file", "commit", commit])).unwrap();
            let (headers, message) = raw.split_once("\n\n").unwrap();
            let tree = headers
                .lines()
                .next()
                .unwrap()
                .strip_prefix("tree ")
                .unwrap();
            let parents = headers
                .lines()
                .filter_map(|l| l.strip_prefix("parent "))
                .map(|p| json!({"sha":p}))
                .collect::<Vec<_>>();
            let who = json!({"name":"Fixture","email":"fixture@example.test","date":"2026-01-01T00:00:00Z"});
            let value = json!({"sha":commit,"tree":{"sha":tree},"parents":parents,"author":who,"committer":who,"message":message,"verification":{"payload":null,"signature":null}});
            assert_eq!(commit_bytes(&value, commit).unwrap(), raw.as_bytes());
            result.insert(format!("{prefix}/git/commits/{commit}"), value);
            let listing = String::from_utf8(git(f, &["ls-tree", "-rt", commit])).unwrap();
            let mut entries = vec![];
            for line in listing.lines() {
                let (meta, path) = line.split_once('\t').unwrap();
                let fields = meta.split_whitespace().collect::<Vec<_>>();
                let (mode, kind, oid) = (fields[0], fields[1], fields[2]);
                let mut entry = json!({"path":path,"mode":mode,"type":kind,"sha":oid});
                if kind == "blob" {
                    let bytes = git(f, &["cat-file", "blob", oid]);
                    entry["size"] = json!(bytes.len());
                    result.insert(format!("{prefix}/git/blobs/{oid}"),json!({"sha":oid,"encoding":"base64","size":bytes.len(),"content":STANDARD.encode(bytes)}));
                }
                entries.push(entry);
            }
            result.insert(
                format!("{prefix}/git/trees/{tree}?recursive=1"),
                json!({"sha":tree,"truncated":false,"tree":entries}),
            );
        }
        result
    }
    #[test]
    fn scoped_fetch_preserves_git_hashes_and_never_requests_excluded_blobs() {
        let f = Fixture::new();
        let replies = Arc::new(Responses {
            replies: Mutex::new(fixture_replies(&f)),
            seen: Mutex::new(vec![]),
        });
        let generated = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .output()
            .unwrap();
        assert!(generated.status.success());
        let mut broker =
            GitHubInstallationBroker::new(1, "client".into(), &generated.stdout).unwrap();
        broker.transport = Box::new(replies.clone());
        let _pass = broker.begin_pass(100).unwrap();
        let session = InstallationSession {
            broker: &broker,
            credential: InstallationCredential {
                token: "fixture-token-only".into(),
                installation: 2,
                repository: 3,
                expires: super::super::super::now().unwrap() + 60,
            },
            owner: "fixture".into(),
            name: "app".into(),
        };
        let request = session
            .fetch_pr_objects(
                &f.request.input,
                f.request.policy.clone(),
                f.request.context.clone(),
            )
            .unwrap();
        let expected = f
            .request
            .objects
            .iter()
            .map(|o| (&o.oid, &o.base64))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            request
                .objects
                .iter()
                .map(|o| (&o.oid, &o.base64))
                .collect::<BTreeMap<_, _>>(),
            expected
        );
        let excluded = object_id("blob", b"EXCLUDED_SOURCE_SENTINEL");
        assert!(
            !replies
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.ends_with(&excluded))
        );
        let public = super::super::super::analyze_pr_objects(&request, &f.root.join("downloaded"))
            .unwrap()
            .into_bundle()
            .unwrap();
        assert!(public.manifest.routes.hld.is_some());
        assert!(!public.manifest.routes.lld_details.is_empty());
        let tree_path = replies
            .replies
            .lock()
            .unwrap()
            .keys()
            .find(|p| p.contains("/git/trees/"))
            .unwrap()
            .clone();
        replies.replies.lock().unwrap().get_mut(&tree_path).unwrap()["truncated"] = json!(true);
        replies.seen.lock().unwrap().clear();
        assert!(
            session
                .fetch_pr_objects(
                    &f.request.input,
                    f.request.policy.clone(),
                    f.request.context.clone()
                )
                .is_err()
        );
        assert!(
            !replies
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.contains("/git/blobs/"))
        );
    }
    #[test]
    fn signed_commit_reconstruction_requires_exact_hash_and_tree() {
        let tree = "a".repeat(40);
        let payload = format!(
            "tree {tree}\nauthor Fixture <fixture@example.test> 1767225600 +0530\ncommitter Fixture <fixture@example.test> 1767225600 +0530\n\nmessage\n"
        );
        let signature = "signature fixture\nsecond line\n";
        let raw = payload.replacen(
            "\n\n",
            &format!("\ngpgsig {}\n\n", signature.trim_end().replace('\n', "\n ")),
            1,
        );
        let oid = object_id("commit", raw.as_bytes());
        let mut value = json!({"sha":oid,"tree":{"sha":tree},"verification":{"payload":payload,"signature":signature}});
        assert_eq!(commit_bytes(&value, &oid).unwrap(), raw.as_bytes());
        value["verification"]["signature"] = json!("modified");
        assert!(commit_bytes(&value, &oid).is_err());
    }
}
