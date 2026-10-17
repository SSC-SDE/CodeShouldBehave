use crate::{
    Action, ContextPin, Grant, PathScope, PrincipalId, RepositoryId, ScopeRecord, SecurityDomain,
    SnapshotId,
    analysis::{FileReview, analyze},
    authorize_repository,
    git::{git, resolve},
};
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Evidence {
    pub path: String,
    pub content: String,
}
pub struct Source {
    pub kind: String,
    pub name: String,
    pub repo: String,
    pub revision: String,
    pub commit: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub files: Vec<Evidence>,
    pub signals: Vec<String>,
    pub notes: Vec<String>,
}

fn paths(value: &str, required: bool) -> Result<Vec<String>, String> {
    if value == "-" && !required {
        return Ok(vec![]);
    }
    let values: Vec<String> = value.split(',').map(|v| v.trim().to_owned()).collect();
    for path in &values {
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains(['*', '?', '[', ']'])
            || path
                .trim_end_matches('/')
                .split('/')
                .any(|p| p == ".." || p == ".git" || (p == "." && path != ".") || p.is_empty())
        {
            return Err(format!(
                "Invalid scope path {path:?}: use literal repository-relative files or directories; '.' explicitly allows the whole repository"
            ));
        }
    }
    Ok(values)
}
fn matches(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    prefix == "."
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
fn allowed(path: &str, source: &Source) -> bool {
    source.include.iter().any(|p| matches(path, p))
        && !source.exclude.iter().any(|p| matches(path, p))
}

fn parse(text: &str) -> Result<Vec<Source>, String> {
    let mut sources = vec![];
    let mut names = BTreeSet::new();
    for (line, row) in text.lines().enumerate() {
        if row.trim().is_empty() || row.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = row.split('\t').collect();
        if columns.len() != 6
            || columns
                .iter()
                .any(|c| c.is_empty() || c.chars().any(char::is_control))
        {
            return Err(format!(
                "Scope line {} requires six nonempty tab-separated fields: kind, name, repository, revision, includes, excludes (use '-' for none)",
                line + 1
            ));
        }
        if !matches!(
            columns[0],
            "repository" | "internal-library" | "external-library" | "service"
        ) {
            return Err(format!("Unknown context kind: {}", columns[0]));
        }
        if !names.insert(columns[1].to_owned()) {
            return Err(format!("Duplicate context name: {}", columns[1]));
        }
        sources.push(Source {
            kind: columns[0].into(),
            name: columns[1].into(),
            repo: columns[2].into(),
            revision: columns[3].into(),
            commit: String::new(),
            include: paths(columns[4], true)?,
            exclude: paths(columns[5], false)?,
            files: vec![],
            signals: vec![],
            notes: vec![],
        });
        if sources.len() > 12 {
            return Err("At most 12 context sources may be selected".into());
        }
    }
    Ok(sources)
}

fn manifest_sources(manifest: &Path) -> Result<Vec<Source>, String> {
    let mut text = String::new();
    File::open(manifest)
        .map_err(|e| format!("Cannot open scope: {e}"))?
        .take(65537)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 65536 {
        return Err("Scope file exceeds 64 KiB".into());
    }
    parse(&text)
}

pub fn prepare_local_access(
    manifest: &Path,
    security_domain: &SecurityDomain,
    principal: &PrincipalId,
) -> Result<(Vec<ContextPin>, Vec<Grant>), String> {
    let sources = manifest_sources(manifest)?;
    let mut pins = Vec::with_capacity(sources.len());
    let mut grants = Vec::with_capacity(sources.len());
    for source in sources {
        let path = PathBuf::from(&source.repo);
        let repo = if path.is_absolute() {
            path
        } else {
            manifest.parent().unwrap_or(Path::new(".")).join(path)
        };
        let repository = RepositoryId::new(
            security_domain.clone(),
            "local-git",
            "local-context",
            source.name.clone(),
        )?;
        let commit = SnapshotId::new(
            resolve(&repo, &source.revision)
                .map_err(|e| format!("Context {}: {e}", source.name))?,
        )?;
        let paths = PathScope::new(source.include.clone(), source.exclude.clone())?;
        pins.push(ContextPin {
            name: source.name,
            repository: repository.clone(),
            commit,
            paths: paths.clone(),
        });
        grants.push(Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source]),
            paths,
        });
    }
    Ok((pins, grants))
}

pub fn load(
    manifest: &Path,
    changed: &[FileReview],
    principal: &PrincipalId,
    scope: &ScopeRecord,
    current_grants: &[Grant],
) -> Result<Vec<Source>, String> {
    let mut sources = manifest_sources(manifest)?;
    if sources.len() != scope.contexts.len() {
        return Err("Context manifest does not match the immutable review scope".into());
    }
    for source in &mut sources {
        let pin = scope
            .contexts
            .iter()
            .find(|pin| pin.name == source.name)
            .ok_or_else(|| {
                format!(
                    "Context {} is not in the immutable review scope",
                    source.name
                )
            })?;
        let selected_paths = PathScope::new(source.include.clone(), source.exclude.clone())?;
        if selected_paths != pin.paths {
            return Err(format!(
                "Context {} paths do not match the immutable review scope",
                source.name
            ));
        }
        authorize_repository(
            Action::Index,
            principal,
            &pin.repository,
            current_grants,
            None,
            &pin.paths,
        )?;
        let path = PathBuf::from(&source.repo);
        let repo = if path.is_absolute() {
            path
        } else {
            manifest.parent().unwrap_or(Path::new(".")).join(path)
        };
        source.commit = resolve(&repo, &source.revision)
            .map_err(|e| format!("Context {}: {e}", source.name))?;
        if source.commit != pin.commit.as_str() {
            return Err(format!(
                "Context {} resolved commit does not match the immutable review scope",
                source.name
            ));
        }
        let mut args = vec![
            "--literal-pathspecs",
            "ls-tree",
            "-r",
            "-z",
            &source.commit,
            "--",
        ];
        // Scope is applied to tree enumeration before any blob content is read.
        if !source.include.iter().any(|p| p == ".") {
            args.extend(source.include.iter().map(String::as_str));
        }
        let tree = git(&repo, &args, 1024 * 1024)?;
        let mut total = 0;
        for entry in tree.split(|b| *b == 0).filter(|b| !b.is_empty()) {
            let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
                return Err("Invalid context Git tree".into());
            };
            let path = match std::str::from_utf8(&entry[tab + 1..]) {
                Ok(p) => p,
                Err(_) => {
                    source.notes.push("Non-UTF-8 path omitted".into());
                    continue;
                }
            };
            if !allowed(path, source) {
                continue;
            }
            authorize_repository(
                Action::Source,
                principal,
                &pin.repository,
                current_grants,
                Some(path),
                &pin.paths,
            )?;
            let meta = std::str::from_utf8(&entry[..tab])
                .map_err(|_| "Invalid Git metadata")?
                .split_whitespace()
                .collect::<Vec<_>>();
            if meta.len() != 3 || !matches!(meta[0], "100644" | "100755") || meta[1] != "blob" {
                source
                    .notes
                    .push(format!("{path}: symlink or submodule omitted"));
                continue;
            }
            if source.files.len() >= 40 {
                source
                    .notes
                    .push("Stopped at 40 files; context is incomplete".into());
                break;
            }
            let bytes = match git(&repo, &["cat-file", "blob", meta[2]], 32 * 1024) {
                Ok(b) => b,
                Err(e) => {
                    source.notes.push(format!("{path}: {e}"));
                    continue;
                }
            };
            total += bytes.len();
            if total > 256 * 1024 {
                source
                    .notes
                    .push("Stopped at 256 KiB; context is incomplete".into());
                break;
            }
            if bytes.contains(&0) {
                source.notes.push(format!("{path}: binary content omitted"));
                continue;
            }
            match String::from_utf8(bytes) {
                Ok(content) => source.files.push(Evidence {
                    path: path.into(),
                    content,
                }),
                Err(_) => source.notes.push(format!("{path}: non-UTF-8 omitted")),
            }
        }
        if source.files.is_empty() {
            source
                .notes
                .push("No readable files in this scope. No context evidence was loaded.".into());
        }
        source.signals = signals(source, changed);
    }
    Ok(sources)
}

fn signals(source: &Source, changed: &[FileReview]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for evidence in &source.files {
        let flow = analyze(&evidence.path, &evidence.content);
        for file in changed {
            for name in flow.calls.intersection(
                &file
                    .before
                    .definitions
                    .union(&file.after.definitions)
                    .cloned()
                    .collect(),
            ) {
                let status = if !file.after.definitions.contains(name) {
                    "definition removed"
                } else if !file.before.definitions.contains(name) {
                    "definition added"
                } else {
                    "definition present in both revisions"
                };
                out.insert(format!(
                    "{} → {}: possible caller of {name}() ({status})",
                    evidence.path, file.path
                ));
            }
            for name in flow.definitions.intersection(
                &file
                    .before
                    .calls
                    .union(&file.after.calls)
                    .cloned()
                    .collect(),
            ) {
                let status = if !file.after.calls.contains(name) {
                    "call removed"
                } else if !file.before.calls.contains(name) {
                    "call added"
                } else {
                    "call retained"
                };
                out.insert(format!(
                    "{} → {}: possible dependency on {name}() ({status})",
                    file.path, evidence.path
                ));
            }
        }
    }
    out.into_iter().take(100).collect()
}

pub fn demo(changed: &[FileReview]) -> Vec<Source> {
    let samples = [
        (
            "repository",
            "Storefront",
            "src/api/",
            "src/api/order.ts",
            "export function placeOrder(cart) {\n  return checkout(cart);\n}\n",
        ),
        (
            "internal-library",
            "Inventory core",
            "src/",
            "src/stock.rs",
            "pub fn reserve(cart: &Cart) -> Result<(), Error> {\n    check_stock(cart)?;\n    Ok(())\n}\n",
        ),
        (
            "external-library",
            "Payments SDK",
            "src/",
            "src/gateway.ts",
            "export function authorize(amount) {\n  return provider.hold(amount);\n}\n",
        ),
        (
            "service",
            "Order events contract",
            "contracts/",
            "contracts/order-events.md",
            "# Order events v2\nOrderPlaced must be published only after successful payment capture.\nConsumers use receipt.id as the idempotency key.\n",
        ),
    ];
    samples
        .into_iter()
        .map(|(kind, name, include, path, content)| {
            let mut source = Source {
                kind: kind.into(),
                name: name.into(),
                repo: format!("/example/{}", name.to_lowercase().replace(' ', "-")),
                revision: "v2".into(),
                commit: "demo-snapshot".into(),
                include: vec![include.into()],
                exclude: vec![],
                files: vec![Evidence {
                    path: path.into(),
                    content: content.into(),
                }],
                signals: vec![],
                notes: vec!["Illustrative demo context; not loaded from your repositories.".into()],
            };
            source.signals = signals(&source, changed);
            source
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_are_literal_and_excludes_win() {
        let source = parse("repository\tapp\t/repo\tmain\tsrc/api\tsrc/api/private")
            .unwrap()
            .remove(0);
        assert!(allowed("src/api/public.rs", &source));
        assert!(!allowed("src/api-v2/file.rs", &source));
        assert!(!allowed("src/api/private/key.rs", &source));
    }
    #[test]
    fn rejects_implicit_or_traversing_scope() {
        for include in [
            "",
            "../secret",
            "/secret",
            "src/*",
            "src/../secret",
            ".git/config",
        ] {
            assert!(parse(&format!("service\tx\t/repo\tHEAD\t{include}\t-")).is_err());
        }
    }
}
