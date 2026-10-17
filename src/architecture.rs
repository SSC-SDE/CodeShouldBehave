use crate::{
    Action, Grant, PathScope, PrincipalId, ScopeRecord,
    analysis::{FileReview, analyze},
    authorize,
    git::{BatchBlob, git, resolve},
    parser::{self, SourceSpan, SyntaxReference, SyntaxSymbol},
    snapshot::{ReadLimits, read_chunk},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Default)]
pub struct FileNode {
    pub path: String,
    pub oid: String,
    pub definitions: BTreeSet<String>,
    pub calls: BTreeSet<String>,
    pub(crate) imports: BTreeSet<String>,
    pub(crate) symbols: Vec<SyntaxSymbol>,
    pub(crate) references: Vec<SyntaxReference>,
    pub(crate) parser_error_nodes: usize,
    pub(crate) heuristic_fallback: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceResolution {
    Resolved,
    Ambiguous,
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReferenceEvidence {
    pub source_path: String,
    pub name: String,
    pub span: Option<SourceSpan>,
    pub resolution: ReferenceResolution,
    pub candidates: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SymbolCorrespondenceKind {
    Exact,
    RenameCandidate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolCorrespondence {
    pub before: String,
    pub after: String,
    pub kind: SymbolCorrespondenceKind,
}

pub struct Architecture {
    pub before: Vec<FileNode>,
    pub after: Vec<FileNode>,
    pub edges: Vec<(String, String, String, &'static str)>,
    pub notes: Vec<String>,
    /// Volatile cache operations never change immutable analysis evidence.
    pub cache_diagnostics: Vec<String>,
    pub reused: usize,
    pub analyzed: usize,
    pub excluded: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub deferred: usize,
    pub enumeration_complete: bool,
    pub(crate) before_references: Vec<ReferenceEvidence>,
    pub(crate) after_references: Vec<ReferenceEvidence>,
    pub(crate) symbol_correspondence: Vec<SymbolCorrespondence>,
}
impl Default for Architecture {
    fn default() -> Self {
        Self {
            before: Vec::new(),
            after: Vec::new(),
            edges: Vec::new(),
            notes: Vec::new(),
            cache_diagnostics: Vec::new(),
            reused: 0,
            analyzed: 0,
            excluded: 0,
            unsupported: 0,
            failed: 0,
            deferred: 0,
            enumeration_complete: true,
            before_references: Vec::new(),
            after_references: Vec::new(),
            symbol_correspondence: Vec::new(),
        }
    }
}
const VERSION: &str = "flowdiff-symbols-v3-tree-sitter-rust-0.24.2-quality";

fn encode(node: &FileNode) -> Vec<u8> {
    let mut out = Vec::new();
    let mut values = std::iter::once(VERSION.to_owned())
        .chain(std::iter::once(node.oid.clone()))
        .chain(std::iter::once(node.parser_error_nodes.to_string()))
        .chain(std::iter::once(
            if node.heuristic_fallback { "1" } else { "0" }.to_owned(),
        ))
        .chain(node.definitions.iter().cloned())
        .chain(std::iter::once(String::new()))
        .chain(node.calls.iter().cloned())
        .chain(std::iter::once(String::new()))
        .chain(node.imports.iter().cloned())
        .chain(std::iter::once(String::new()))
        .collect::<Vec<_>>();
    for symbol in &node.symbols {
        values.extend([
            "symbol".into(),
            symbol.stable_key.clone(),
            symbol.correspondence_key.clone(),
            symbol.kind.clone(),
            symbol.name.clone(),
            symbol.span.start_line.to_string(),
            symbol.span.start_column.to_string(),
            symbol.span.end_line.to_string(),
            symbol.span.end_column.to_string(),
        ]);
    }
    for reference in &node.references {
        values.extend([
            "reference".into(),
            reference.name.clone(),
            reference.span.start_line.to_string(),
            reference.span.start_column.to_string(),
            reference.span.end_line.to_string(),
            reference.span.end_column.to_string(),
        ]);
    }
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}
fn decode(bytes: &[u8], oid: &str) -> Option<FileNode> {
    let mut offset = 0;
    let mut values = vec![];
    while offset < bytes.len() {
        let size = u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        offset += 4;
        let value = std::str::from_utf8(bytes.get(offset..offset.checked_add(size)?)?)
            .ok()?
            .to_owned();
        offset += size;
        values.push(value);
        if values.len() > 50000 {
            return None;
        }
    }
    if values.first()?.as_str() != VERSION || values.get(1)? != oid {
        return None;
    }
    let parser_error_nodes = values.get(2)?.parse::<usize>().ok()?;
    let heuristic_fallback = match values.get(3)?.as_str() {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    // Reject corrupt quality records rather than treating them as clean parses.
    if parser_error_nodes > 1024 * 1024 || (heuristic_fallback && parser_error_nodes > 0) {
        return None;
    }
    let definitions_end = values.iter().skip(4).position(String::is_empty)? + 4;
    let calls_start = definitions_end + 1;
    let calls_end = values.iter().skip(calls_start).position(String::is_empty)? + calls_start;
    let imports_start = calls_end + 1;
    let imports_end = values
        .iter()
        .skip(imports_start)
        .position(String::is_empty)?
        + imports_start;
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut index = imports_end + 1;
    while index < values.len() {
        match values[index].as_str() {
            "symbol" => {
                let fields = values.get(index + 1..index + 9)?;
                symbols.push(SyntaxSymbol {
                    stable_key: fields[0].clone(),
                    correspondence_key: fields[1].clone(),
                    kind: fields[2].clone(),
                    name: fields[3].clone(),
                    span: decoded_span(&fields[4..8])?,
                });
                index += 9;
            }
            "reference" => {
                let fields = values.get(index + 1..index + 6)?;
                references.push(SyntaxReference {
                    name: fields[0].clone(),
                    span: decoded_span(&fields[1..5])?,
                });
                index += 6;
            }
            _ => return None,
        }
    }
    Some(FileNode {
        oid: oid.into(),
        definitions: values[4..definitions_end].iter().cloned().collect(),
        calls: values[calls_start..calls_end].iter().cloned().collect(),
        imports: values[imports_start..imports_end].iter().cloned().collect(),
        symbols,
        references,
        parser_error_nodes,
        heuristic_fallback,
        ..FileNode::default()
    })
}

fn decoded_span(values: &[String]) -> Option<SourceSpan> {
    Some(SourceSpan {
        start_line: values.first()?.parse().ok()?,
        start_column: values.get(1)?.parse().ok()?,
        end_line: values.get(2)?.parse().ok()?,
        end_column: values.get(3)?.parse().ok()?,
    })
}

fn snapshot(
    request: &BuildRequest<'_>,
    commit: &str,
    cache: &Path,
    result: &mut Architecture,
) -> Result<Vec<FileNode>, String> {
    request.cancellation.checkpoint()?;
    let bytes = git(
        request.repo,
        &["ls-tree", "-r", "-z", commit, "--"],
        4 * 1024 * 1024,
    )?;
    let mut nodes = vec![];
    let mut pending = Vec::<(usize, String, String, std::path::PathBuf)>::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if entries.len() > 2000 {
        result.deferred += entries.len() - 2000;
        result.notes.push(format!(
            "{commit}: {} files exceed the 2,000-file reader limit and are deferred.",
            entries.len() - 2000
        ));
    }
    for entry in entries.into_iter().take(2000) {
        if nodes.len().is_multiple_of(128) {
            request.cancellation.checkpoint()?;
        }
        let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
            return Err("Invalid architecture tree".into());
        };
        let Ok(path) = std::str::from_utf8(&entry[tab + 1..]) else {
            result
                .notes
                .push("Non-UTF-8 architecture path omitted".into());
            result.failed += 1;
            continue;
        };
        if !request.paths.allows(path) || crate::sensitive_path(path) {
            result.excluded += 1;
            continue;
        }
        if authorize(
            Action::Source,
            request.principal,
            request.scope,
            request.current_grants,
            Some(path),
        )
        .is_err()
        {
            result.excluded += 1;
            continue;
        }
        let meta = std::str::from_utf8(&entry[..tab])
            .map_err(|_| "Invalid tree metadata")?
            .split_whitespace()
            .collect::<Vec<_>>();
        if meta.len() != 3 || !matches!(meta[0], "100644" | "100755") {
            result
                .notes
                .push(format!("{path}: symlink/submodule not indexed"));
            result.unsupported += 1;
            continue;
        }
        let oid = meta[2];
        if !oid.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(oid.len(), 40 | 64) {
            return Err("Invalid architecture object ID".into());
        }
        let ext = path.rsplit('.').next().unwrap_or("");
        let supported = matches!(
            ext,
            "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs"
        );
        let mut node = FileNode {
            path: path.into(),
            oid: oid.into(),
            ..FileNode::default()
        };
        if supported {
            // Syntax identities embed paths. Identical blobs copied or renamed
            // to another path require their own cached provenance.
            let cached = cache.join(format!(
                "{VERSION}-{ext}-{}-{oid}.bin",
                namespace("path", path)
            ));
            let read = fs::symlink_metadata(&cached)
                .ok()
                .filter(|m| m.is_file() && !m.file_type().is_symlink())
                .and_then(|_| {
                    let mut bytes = vec![];
                    fs::File::open(&cached)
                        .ok()?
                        .take(1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    if bytes.len() > 1024 * 1024 {
                        return None;
                    }
                    decode(&bytes, oid)
                });
            if let Some(mut hit) = read {
                hit.path = path.into();
                node = hit;
                result.reused += 1;
            } else {
                if fs::symlink_metadata(&cached)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    result
                        .notes
                        .push(format!("{path}: unsafe cache symlink rejected"));
                    result.failed += 1;
                    nodes.push(node);
                    continue;
                }
                if cached.exists() {
                    fs::remove_file(&cached)
                        .map_err(|error| format!("Cannot replace corrupt cache record: {error}"))?;
                }
                pending.push((nodes.len(), path.into(), oid.into(), cached));
            }
        } else {
            result.unsupported += 1;
        }
        nodes.push(node);
    }
    for chunk in pending.chunks(128) {
        request.cancellation.checkpoint()?;
        let object_ids = chunk
            .iter()
            .map(|(_, _, oid, _)| oid.clone())
            .collect::<Vec<_>>();
        let blobs = read_chunk(
            request.repo,
            &object_ids,
            ReadLimits::default(),
            request.cancellation,
        )?;
        for ((index, path, _, cached), blob) in chunk.iter().zip(blobs) {
            match blob {
                BatchBlob::Data(bytes) if bytes.contains(&0) => {
                    result.notes.push(format!("{path}: binary symbols omitted"));
                    result.unsupported += 1;
                }
                BatchBlob::Data(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => {
                        if path.ends_with(".rs") {
                            match parser::parse_rust(path, &text, request.cancellation) {
                                Ok(parsed) => {
                                    nodes[*index].parser_error_nodes = parsed.error_nodes;
                                    nodes[*index].definitions = parsed.definitions;
                                    nodes[*index].calls = parsed.calls;
                                    nodes[*index].imports = parsed.imports;
                                    nodes[*index].symbols = parsed.symbols;
                                    nodes[*index].references = parsed.references;
                                }
                                Err(_) => {
                                    // A cancelled parse is not reusable evidence.
                                    request.cancellation.checkpoint()?;
                                    nodes[*index].heuristic_fallback = true;
                                    let flow = analyze(path, &text);
                                    nodes[*index].definitions = flow.definitions;
                                    nodes[*index].calls = flow.calls;
                                }
                            }
                        } else {
                            let flow = analyze(path, &text);
                            nodes[*index].definitions = flow.definitions;
                            nodes[*index].calls = flow.calls;
                        }
                        result.analyzed += 1;
                        if persist_cache(cached, &encode(&nodes[*index])).is_err() {
                            result.cache_diagnostics.push(
                                "Could not persist resumable architecture cache; analysis evidence remains available".into()
                            );
                        }
                    }
                    Err(_) => {
                        result
                            .notes
                            .push(format!("{path}: non-UTF-8 symbols omitted"));
                        result.unsupported += 1;
                    }
                },
                BatchBlob::Missing => {
                    result
                        .notes
                        .push(format!("{path}: Git object is missing; indexing failed"));
                    result.failed += 1;
                }
                BatchBlob::Oversized(size) => {
                    result.notes.push(format!(
                        "{path}: {size}-byte blob exceeds the 512 KiB reader limit; deferred"
                    ));
                    result.deferred += 1;
                }
            }
        }
    }
    // Emit quality notes in tree order for both cold and warm reads. Emitting
    // them during cache lookup would reorder notes in partially warm snapshots.
    for node in &nodes {
        if node.parser_error_nodes > 0 {
            result.notes.push(format!(
                "{}: Rust parser recovered from {} syntax error nodes",
                node.path, node.parser_error_nodes
            ));
        }
        if node.heuristic_fallback {
            result.notes.push(format!(
                "{}: Rust parser unavailable; heuristic fallback used",
                node.path
            ));
        }
    }
    Ok(nodes)
}

fn persist_cache(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Cache path has no parent")?;
    let temporary = parent.join(format!(
        ".tmp-cache-{}-{}",
        std::process::id(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("record")
    ));
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary)
                .map_err(|remove| format!("Cannot replace abandoned cache temporary: {remove}"))?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|create| format!("Cannot create cache temporary: {create}"))?
        }
        Err(error) => return Err(format!("Cannot create cache temporary: {error}")),
    };
    file.write_all(bytes)
        .map_err(|error| format!("Cannot write cache temporary: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Cannot sync cache temporary: {error}"))?;
    match fs::hard_link(&temporary, path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("Cannot finalize cache record: {error}")),
    }
    fs::remove_file(&temporary)
        .map_err(|error| format!("Cannot remove cache temporary: {error}"))?;
    Ok(())
}

fn links(nodes: &[FileNode]) -> (BTreeSet<(String, String, String)>, bool) {
    let mut definitions: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in nodes {
        for name in &node.definitions {
            definitions.entry(name).or_default().push(&node.path);
        }
    }
    let mut edges = BTreeSet::new();
    for node in nodes {
        for call in &node.calls {
            if let Some(targets) = definitions.get(call.as_str()) {
                for target in targets {
                    if *target != node.path {
                        edges.insert((node.path.clone(), target.to_string(), call.clone()));
                    }
                    if edges.len() >= 3000 {
                        return (edges, true);
                    }
                }
            }
        }
    }
    (edges, false)
}
fn reference_evidence(nodes: &[FileNode]) -> Vec<ReferenceEvidence> {
    let mut definitions: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for node in nodes {
        if node.symbols.is_empty() {
            for name in &node.definitions {
                definitions
                    .entry(name)
                    .or_default()
                    .push(format!("{}:heuristic:{name}", node.path));
            }
        } else {
            for symbol in &node.symbols {
                definitions
                    .entry(&symbol.name)
                    .or_default()
                    .push(symbol.stable_key.clone());
            }
        }
    }
    let mut evidence = Vec::new();
    for node in nodes {
        if node.references.is_empty() {
            for name in &node.calls {
                evidence.push(resolved_reference(
                    &node.path,
                    name,
                    None,
                    definitions.get(name.as_str()),
                ));
            }
        } else {
            for reference in &node.references {
                evidence.push(resolved_reference(
                    &node.path,
                    &reference.name,
                    Some(reference.span.clone()),
                    definitions.get(reference.name.as_str()),
                ));
            }
        }
    }
    evidence
}

fn resolved_reference(
    source_path: &str,
    name: &str,
    span: Option<SourceSpan>,
    candidates: Option<&Vec<String>>,
) -> ReferenceEvidence {
    let candidates = candidates.cloned().unwrap_or_default();
    let resolution = match candidates.len() {
        0 => ReferenceResolution::Unresolved,
        1 => ReferenceResolution::Resolved,
        _ => ReferenceResolution::Ambiguous,
    };
    ReferenceEvidence {
        source_path: source_path.into(),
        name: name.into(),
        span,
        resolution,
        candidates,
    }
}
fn symbol_correspondence(before: &[FileNode], after: &[FileNode]) -> Vec<SymbolCorrespondence> {
    let before_by_path = before
        .iter()
        .map(|node| (node.path.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let after_by_path = after
        .iter()
        .map(|node| (node.path.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut matches = Vec::new();
    for (path, before_node) in before_by_path {
        let Some(after_node) = after_by_path.get(path) else {
            continue;
        };
        let mut matched_before = BTreeSet::new();
        let mut matched_after = BTreeSet::new();
        for (before_index, old) in before_node.symbols.iter().enumerate() {
            if let Some((after_index, new)) = after_node
                .symbols
                .iter()
                .enumerate()
                .find(|(_, new)| new.correspondence_key == old.correspondence_key)
            {
                matched_before.insert(before_index);
                matched_after.insert(after_index);
                matches.push(SymbolCorrespondence {
                    before: old.stable_key.clone(),
                    after: new.stable_key.clone(),
                    kind: SymbolCorrespondenceKind::Exact,
                });
            }
        }
        let old_unmatched = before_node
            .symbols
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched_before.contains(index))
            .collect::<Vec<_>>();
        let new_unmatched = after_node
            .symbols
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched_after.contains(index))
            .collect::<Vec<_>>();
        for (_, old) in old_unmatched {
            let old_container = old
                .correspondence_key
                .strip_suffix(&format!(":{}", old.name));
            let candidates = new_unmatched
                .iter()
                .filter(|(_, new)| {
                    new.kind == old.kind
                        && new
                            .correspondence_key
                            .strip_suffix(&format!(":{}", new.name))
                            == old_container
                })
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                let new = candidates[0].1;
                matches.push(SymbolCorrespondence {
                    before: old.stable_key.clone(),
                    after: new.stable_key.clone(),
                    kind: SymbolCorrespondenceKind::RenameCandidate,
                });
            }
        }
    }
    matches
}
fn compare_edges(result: &mut Architecture) {
    result.before_references = reference_evidence(&result.before);
    result.after_references = reference_evidence(&result.after);
    result.symbol_correspondence = symbol_correspondence(&result.before, &result.after);
    let (before, old_limited) = links(&result.before);
    let (after, new_limited) = links(&result.after);
    if old_limited || new_limited {
        result.notes.push(
            "Connections capped at 3,000 per revision; edge comparison may be incomplete.".into(),
        );
    }
    result.edges = before
        .union(&after)
        .map(|(from, to, name)| {
            (
                from.clone(),
                to.clone(),
                name.clone(),
                if !before.contains(&(from.clone(), to.clone(), name.clone())) {
                    "added"
                } else if !after.contains(&(from.clone(), to.clone(), name.clone())) {
                    "removed"
                } else {
                    "retained"
                },
            )
        })
        .collect();
}
pub struct BuildRequest<'a> {
    pub repo: &'a Path,
    pub base: &'a str,
    pub head: &'a str,
    pub cache: &'a Path,
    pub security_domain: &'a str,
    pub paths: &'a PathScope,
    pub principal: &'a PrincipalId,
    pub scope: &'a ScopeRecord,
    pub current_grants: &'a [Grant],
    pub cancellation: &'a crate::core::Cancellation,
}

pub fn build(request: &BuildRequest<'_>) -> Result<Architecture, String> {
    let base = resolve(request.repo, request.base)?;
    let head = resolve(request.repo, request.head)?;
    if fs::symlink_metadata(request.cache).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Architecture cache directory must not be a symlink".into());
    }
    fs::create_dir_all(request.cache)
        .map_err(|e| format!("Cannot create architecture cache: {e}"))?;
    let namespace = request
        .cache
        .join(namespace("domain", request.security_domain));
    if fs::symlink_metadata(&namespace).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Architecture cache namespace must not be a symlink".into());
    }
    fs::create_dir_all(&namespace)
        .map_err(|e| format!("Cannot create architecture cache namespace: {e}"))?;
    let mut result = Architecture::default();
    result.before = snapshot(request, &base, &namespace, &mut result)?;
    result.after = if base == head {
        result.before.clone()
    } else {
        snapshot(request, &head, &namespace, &mut result)?
    };
    compare_edges(&mut result);
    Ok(result)
}

fn namespace(kind: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    let mut encoded = format!("{kind}-");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
pub fn demo(files: &[FileReview]) -> Architecture {
    let mut result = Architecture::default();
    for file in files {
        result.before.push(FileNode {
            path: file.path.clone(),
            oid: "before".into(),
            definitions: file.before.definitions.clone(),
            calls: file.before.calls.clone(),
            ..FileNode::default()
        });
        result.after.push(FileNode {
            path: file.path.clone(),
            oid: "after".into(),
            definitions: file.after.definitions.clone(),
            calls: file.after.calls.clone(),
            ..FileNode::default()
        });
    }
    for (path, source) in [
        ("api/orders.rs", "fn create_order() { checkout(); }"),
        ("jobs/reconcile.rs", "fn reconcile() { charge(0); }"),
        ("web/checkout.ts", "function submit() { create_order(); }"),
    ] {
        let flow = analyze(path, source);
        let node = FileNode {
            path: path.into(),
            oid: "unchanged".into(),
            definitions: flow.definitions,
            calls: flow.calls,
            ..FileNode::default()
        };
        result.before.push(node.clone());
        result.after.push(node);
    }
    result.notes.push(
        "Illustrative architecture; arrows are name-match candidates, not resolved dependencies."
            .into(),
    );
    compare_edges(&mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "csb-syntax-cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let repo = Self(root);
            repo.git(&["init", "--template="]);
            repo.git(&["config", "user.name", "Cache fixture"]);
            repo.git(&["config", "user.email", "cache@example.invalid"]);
            repo
        }
        fn git(&self, arguments: &[&str]) -> String {
            let mut command = Command::new("git");
            command.env_clear();
            for key in ["PATH", "TMPDIR", "LANG"] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
            let result = command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .arg("-C")
                .arg(&self.0)
                .args(["-c", "core.hooksPath=/dev/null"])
                .args(arguments)
                .output()
                .unwrap();
            assert!(result.status.success(), "Git fixture setup failed");
            String::from_utf8(result.stdout).unwrap().trim().to_owned()
        }
        fn commit(&self, message: &str) -> String {
            self.git(&["add", "--all"]);
            self.git(&["commit", "-qm", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cached_identical_blob_copy_and_rename_keep_each_symbol_source_identity() {
        let repo = Repo::new();
        fs::create_dir(repo.0.join("src")).unwrap();
        let source = "fn service() {}\nfn run() { service(); }\n";
        fs::write(repo.0.join("src/original.rs"), source).unwrap();
        let base = repo.commit("original syntax");
        fs::rename(
            repo.0.join("src/original.rs"),
            repo.0.join("src/renamed.rs"),
        )
        .unwrap();
        fs::write(repo.0.join("src/copied.rs"), source).unwrap();
        let head = repo.commit("rename and identical copy");
        let principal = PrincipalId::new("cache-fixture").unwrap();
        let repository = crate::RepositoryId::new(
            crate::SecurityDomain::new("cache-fixture").unwrap(),
            "local-git",
            "fixture",
            "repo",
        )
        .unwrap();
        let paths = PathScope::all();
        let scope = ScopeRecord {
            version: crate::ScopeVersion::local_default(),
            primary_repository: repository.clone(),
            primary_paths: paths.clone(),
            contexts: vec![],
            contracts: Default::default(),
            export_allowed: false,
            model_destinations: Default::default(),
        };
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Source]),
            paths: paths.clone(),
        }];
        let cache = repo.0.join("cache");
        let cancellation = crate::Cancellation::default();
        let request = BuildRequest {
            repo: &repo.0,
            base: &base,
            head: &head,
            cache: &cache,
            security_domain: "cache-fixture",
            paths: &paths,
            principal: &principal,
            scope: &scope,
            current_grants: &grants,
            cancellation: &cancellation,
        };
        let cold = build(&request).unwrap();
        let warm = build(&request).unwrap();
        assert_eq!(cold.analyzed, 3);
        assert_eq!(warm.reused, 3);
        for result in [cold, warm] {
            assert_eq!(result.before.len(), 1);
            assert_eq!(result.after.len(), 2);
            for node in result.before.iter().chain(&result.after) {
                assert_eq!(node.symbols.len(), 2);
                for symbol in &node.symbols {
                    let prefix = format!("{}:", node.path);
                    assert!(symbol.stable_key.starts_with(&prefix));
                    assert!(symbol.correspondence_key.starts_with(&prefix));
                }
            }
        }
    }
    #[test]
    fn cache_rejects_bad_records() {
        assert!(decode(&[255, 255, 255, 255], "x").is_none());
        let n = FileNode {
            oid: "abc".into(),
            definitions: BTreeSet::from(["run".into()]),
            ..FileNode::default()
        };
        assert_eq!(
            decode(&encode(&n), "abc").unwrap().definitions,
            n.definitions
        );
        assert!(decode(&encode(&n), "wrong").is_none());
    }

    #[test]
    fn cache_quality_round_trips_and_legacy_or_corrupt_quality_is_rejected() {
        let recovered = FileNode {
            oid: "recovered".into(),
            parser_error_nodes: 3,
            ..FileNode::default()
        };
        let decoded = decode(&encode(&recovered), "recovered").unwrap();
        assert_eq!(decoded.parser_error_nodes, 3);
        assert!(!decoded.heuristic_fallback);
        let fallback = FileNode {
            oid: "fallback".into(),
            heuristic_fallback: true,
            ..FileNode::default()
        };
        assert!(
            decode(&encode(&fallback), "fallback")
                .unwrap()
                .heuristic_fallback
        );
        let contradictory = FileNode {
            heuristic_fallback: true,
            ..recovered.clone()
        };
        assert!(decode(&encode(&contradictory), "recovered").is_none());
        let oversized = FileNode {
            parser_error_nodes: usize::MAX,
            ..recovered.clone()
        };
        assert!(decode(&encode(&oversized), "recovered").is_none());

        let encoded = encode(&recovered);
        let legacy_version = b"flowdiff-symbols-v2-tree-sitter-rust-0.24.2";
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&(legacy_version.len() as u32).to_le_bytes());
        legacy.extend_from_slice(legacy_version);
        legacy.extend_from_slice(&encoded[4 + VERSION.len()..]);
        assert!(decode(&legacy, "recovered").is_none());
    }

    #[test]
    fn syntax_cache_round_trips_provenance_and_resolution_stays_snapshot_scoped() {
        let parsed_a = parser::parse_rust(
            "a.rs",
            "fn duplicate() {} fn caller() { duplicate(); missing(); }",
            &crate::core::Cancellation::default(),
        )
        .unwrap();
        let parsed_b = parser::parse_rust(
            "b.rs",
            "fn duplicate() {}",
            &crate::core::Cancellation::default(),
        )
        .unwrap();
        let make_node = |path: &str, parsed: parser::ParsedFile| FileNode {
            path: path.into(),
            oid: path.into(),
            definitions: parsed.definitions,
            calls: parsed.calls,
            imports: parsed.imports,
            symbols: parsed.symbols,
            references: parsed.references,
            parser_error_nodes: parsed.error_nodes,
            heuristic_fallback: false,
        };
        let first = make_node("a.rs", parsed_a);
        let decoded = decode(&encode(&first), "a.rs").unwrap();
        assert_eq!(decoded.symbols, first.symbols);
        assert_eq!(decoded.references, first.references);

        let evidence = reference_evidence(&[first, make_node("b.rs", parsed_b)]);
        assert!(evidence.iter().any(|reference| {
            reference.name == "duplicate"
                && reference.resolution == ReferenceResolution::Ambiguous
                && reference.candidates.len() == 2
        }));
        assert!(evidence.iter().any(|reference| {
            reference.name == "missing"
                && reference.resolution == ReferenceResolution::Unresolved
                && reference.candidates.is_empty()
        }));
    }

    #[test]
    fn correspondence_labels_unique_same_container_rename_candidates() {
        let cancellation = crate::core::Cancellation::default();
        let make = |source: &str| {
            let parsed = parser::parse_rust("api.rs", source, &cancellation).unwrap();
            FileNode {
                path: "api.rs".into(),
                oid: source.into(),
                definitions: parsed.definitions,
                calls: parsed.calls,
                imports: parsed.imports,
                symbols: parsed.symbols,
                references: parsed.references,
                parser_error_nodes: parsed.error_nodes,
                heuristic_fallback: false,
            }
        };
        let matches =
            symbol_correspondence(&[make("fn old_name() {}")], &[make("fn new_name() {}")]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, SymbolCorrespondenceKind::RenameCandidate);
    }
}
