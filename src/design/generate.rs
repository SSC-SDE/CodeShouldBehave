use super::*;
use crate::{
    Cancellation,
    git::BatchBlob,
    snapshot::{ReadLimits, read_chunk},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser, Point};

const MAX_PATHS: usize = 2000;
const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_FACTS: usize = 2000;
const MAX_TIME: Duration = Duration::from_secs(10);
#[derive(Clone)]
struct Blob {
    path: String,
    oid: String,
    text: String,
}
#[derive(Clone)]
struct Contract {
    key: String,
    name: String,
    signature: String,
    start: usize,
    end: usize,
}
#[derive(Default)]
struct RustFacts {
    contracts: Vec<Contract>,
    imports: Vec<(String, usize, usize)>,
    gaps: BTreeSet<String>,
    unsupported_architectural_declarations: usize,
}
struct Builder<'a> {
    snapshot: &'a SnapshotId,
    scope: &'a ScopeRecord,
    context: &'a ConfirmedDesignContext,
    elements: BTreeMap<String, DesignElement>,
    relations: BTreeMap<String, DesignRelation>,
    evidence: BTreeMap<String, DesignEvidence>,
    document_candidates: BTreeMap<String, DesignDocumentCandidate>,
    failed_paths: BTreeSet<String>,
    coverage: DesignCoverage,
}
impl Builder<'_> {
    fn document(&mut self, path: &str, oid: &str, text: &str) {
        if !safe_path(path, self.scope)
            || kind(path) != "document"
            || SnapshotId::new(oid.into()).is_err()
        {
            self.gap("A document candidate is outside the supported pinned scope");
            return;
        }
        if self.document_candidates.len() >= 100 {
            self.coverage.deferred += 1;
            self.gap("Design document candidate inventory reached 100 entries; additional document intent remains unselected");
            return;
        }
        // Inventory the exact approved tree/blob without parsing or copying
        // document text into code facts. Its full pinned range is evidence;
        // existing source delivery still enforces 200-line/64-KiB limits.
        let blob = Blob {
            path: path.into(),
            oid: oid.into(),
            text: String::new(),
        };
        let evidence_id = self.evidence(
            &blob,
            1,
            text.lines().count().max(1),
            "documentation_candidate",
            DesignResolution::ObservedStructure,
        );
        let id = format!("document-{}", digest(&[path]));
        self.document_candidates.insert(
            id.clone(),
            DesignDocumentCandidate {
                id,
                title: path.rsplit('/').next().unwrap_or(path).into(),
                evidence_id,
            },
        );
    }
    fn gap(&mut self, text: impl Into<String>) {
        self.coverage.complete = false;
        let text = text.into();
        if self.coverage.gaps.len() < 64 && !self.coverage.gaps.contains(&text) {
            self.coverage.gaps.push(text);
        }
    }
    fn owner(&self, path: &str) -> Option<String> {
        let mut owners = Vec::new();
        let mut longest = 0;
        for component in &self.context.components {
            for root in &component.roots {
                if under(path, root) {
                    let depth = if root == "." {
                        0
                    } else {
                        root.split('/').count()
                    };
                    if depth > longest {
                        longest = depth;
                        owners.clear();
                    }
                    if depth == longest {
                        owners.push(component_id(self.scope, &component.id));
                    }
                }
            }
        }
        owners.sort();
        owners.dedup();
        if owners.len() == 1 {
            owners.pop()
        } else {
            None
        }
    }
    fn evidence(
        &mut self,
        blob: &Blob,
        start: usize,
        end: usize,
        kind: &str,
        resolution: DesignResolution,
    ) -> String {
        let id = format!(
            "evidence-{}",
            digest(&[
                self.snapshot.as_str(),
                &blob.path,
                &blob.oid,
                &start.to_string(),
                &end.to_string(),
                kind
            ])
        );
        self.evidence
            .entry(id.clone())
            .or_insert_with(|| DesignEvidence {
                id: id.clone(),
                repository: self.scope.primary_repository.clone(),
                commit: self.snapshot.clone(),
                path: blob.path.clone(),
                blob: blob.oid.clone(),
                start_line: start,
                end_line: end.max(start),
                producer_version: HLD_GENERATOR_VERSION.into(),
                relationship_type: kind.into(),
                resolution,
            });
        id
    }
    fn fact(
        &mut self,
        id: &str,
        kind: &str,
        value: String,
        evidence: String,
        resolution: DesignResolution,
    ) {
        if let Some(element) = self.elements.get_mut(id) {
            element.evidence_ids.push(evidence.clone());
            element.facts.push(DesignFact {
                kind: kind.into(),
                value,
                resolution,
                evidence_ids: vec![evidence],
            });
        }
    }
    fn relation(
        &mut self,
        from: &str,
        to: &str,
        kind: &str,
        evidence: &str,
        resolution: DesignResolution,
    ) {
        let id = format!("relation-{}", digest(&[from, to, kind]));
        let relation=self.relations.entry(id.clone()).or_insert_with(||DesignRelation{id,from:from.into(),to:to.into(),kind:kind.into(),direction:"source_to_target".into(),protocol:None,synchrony:"unknown".into(),resolution,evidence_ids:vec![],unknowns:vec!["Static dependency direction does not establish a runtime request, service, ownership or delivery guarantee".into()]});
        relation.evidence_ids.push(evidence.into());
    }
    fn element(
        &mut self,
        id: String,
        kind: DesignElementKind,
        name: String,
        owner: Option<String>,
    ) {
        self.elements.entry(id.clone()).or_insert(DesignElement {
            id,
            kind,
            name,
            component_id: owner,
            roots: vec![],
            responsibility: None,
            user_assertions: vec![],
            facts: vec![],
            evidence_ids: vec![],
            unknowns: vec![],
        });
    }
    fn manifest(&mut self, blob: &Blob, owner: &str, packages: &BTreeMap<String, String>) {
        let Ok(value) = toml::from_str::<toml::Value>(&blob.text) else {
            self.coverage.failed += 1;
            self.failed_paths.insert(blob.path.clone());
            self.gap("A selected Cargo manifest could not be parsed; no dependency facts inferred");
            return;
        };
        if let Some(name) = value
            .get("package")
            .and_then(|item| item.get("name"))
            .and_then(toml::Value::as_str)
            .filter(|name| identifier(name))
        {
            let ev = self.evidence(
                blob,
                1,
                1,
                "package_declaration",
                DesignResolution::ObservedStructure,
            );
            self.fact(
                owner,
                "package",
                name.into(),
                ev,
                DesignResolution::ObservedStructure,
            );
        }
        for table in ["dependencies", "build-dependencies"] {
            if let Some(items) = value.get(table).and_then(toml::Value::as_table) {
                for (alias, item) in items {
                    if !identifier(alias) {
                        self.gap(
                            "A selected dependency name exceeds the bounded declaration profile",
                        );
                        continue;
                    }
                    let package = item
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .filter(|value| identifier(value))
                        .unwrap_or(alias);
                    let version = item
                        .as_str()
                        .or_else(|| item.get("version").and_then(toml::Value::as_str))
                        .filter(|text| text.len() <= 128 && !text.chars().any(char::is_control));
                    let ev = self.evidence(
                        blob,
                        1,
                        1,
                        "manifest_dependency",
                        DesignResolution::ObservedStructure,
                    );
                    // Package names never establish live service instances.
                    let dependency = format!("dependency-{}", digest(&[owner, alias, table]));
                    self.element(
                        dependency.clone(),
                        DesignElementKind::Dependency,
                        format!("{package} package"),
                        Some(owner.into()),
                    );
                    self.fact(
                        &dependency,
                        "package",
                        package.into(),
                        ev.clone(),
                        DesignResolution::ObservedStructure,
                    );
                    self.fact(
                        &dependency,
                        "crate_alias",
                        alias.replace('-', "_"),
                        ev.clone(),
                        DesignResolution::ObservedStructure,
                    );
                    self.fact(
                        &dependency,
                        "dependency_table",
                        table.into(),
                        ev.clone(),
                        DesignResolution::ObservedStructure,
                    );
                    if let Some(version) = version {
                        self.fact(
                            &dependency,
                            "version_requirement",
                            version.into(),
                            ev.clone(),
                            DesignResolution::ObservedStructure,
                        );
                    }
                    if let Some(features) = item.get("features").and_then(toml::Value::as_array) {
                        let mut names = features
                            .iter()
                            .filter_map(toml::Value::as_str)
                            .filter(|name| identifier(name))
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        if names.len() != features.len() || names.len() > 100 {
                            self.gap(
                                "Dependency feature names exceed the supported declaration profile",
                            );
                        } else {
                            names.sort();
                            names.dedup();
                            self.fact(
                                &dependency,
                                "declared_features",
                                names.join(", "),
                                ev.clone(),
                                DesignResolution::ObservedStructure,
                            );
                        }
                    }
                    if let Some(default) =
                        item.get("default-features").and_then(toml::Value::as_bool)
                    {
                        self.fact(
                            &dependency,
                            "default_features",
                            default.to_string(),
                            ev.clone(),
                            DesignResolution::ObservedStructure,
                        );
                    }
                    if item.get("optional").and_then(toml::Value::as_bool) == Some(true)
                        || item.get("workspace").and_then(toml::Value::as_bool) == Some(true)
                        || item.get("git").is_some()
                    {
                        self.elements.get_mut(&dependency).unwrap().unknowns.push("Feature/workspace/Git activation is unresolved; no network fetch or feature execution occurs".into());
                        self.gap("Conditional, workspace-inherited or Git dependencies cannot be resolved completely by this profile");
                    }
                    self.relation(
                        owner,
                        &dependency,
                        "declares_package_dependency",
                        &ev,
                        DesignResolution::ObservedStructure,
                    );
                    if let Some(path) = item.get("path").and_then(toml::Value::as_str) {
                        let parent = blob.path.rsplit_once('/').map_or("", |(parent, _)| parent);
                        let target = normalize_relative(parent, path);
                        match target.filter(|root|safe_path(root,self.scope)).and_then(|root|packages.get(&root)) {
                            Some(target) if target!=owner=>self.relation(owner,target,"local_package_dependency",&ev,DesignResolution::ResolvedReference),
                            Some(_)=>{},None=>self.gap("A local package dependency has no unique confirmed owner inside selected scope"),
                        }
                    }
                }
            }
        }
        if value.get("target").is_some() || value.get("patch").is_some() {
            self.gap("Target-specific dependencies or Cargo patches are outside the current dependency-resolution profile");
        }
    }
    fn lockfile(&mut self, blob: &Blob, owner: &str) {
        let Ok(value) = toml::from_str::<toml::Value>(&blob.text) else {
            self.coverage.failed += 1;
            self.failed_paths.insert(blob.path.clone());
            self.gap("A selected Cargo lockfile could not be parsed");
            return;
        };
        let Some(packages) = value.get("package").and_then(toml::Value::as_array) else {
            self.gap("Selected Cargo lockfile package inventory is unavailable");
            return;
        };
        for package in packages.iter().take(1000) {
            let name = package
                .get("name")
                .and_then(toml::Value::as_str)
                .filter(|name| identifier(name));
            let version = package
                .get("version")
                .and_then(toml::Value::as_str)
                .filter(|version| identifier(version));
            let (Some(name), Some(version)) = (name, version) else {
                self.gap("A lockfile package has an unsupported name/version declaration");
                continue;
            };
            // Version is the observed value, not the logical ID, so a unique
            // package version update is a modified dependency. Duplicate-name
            // versions retain a qualified inventory, not resolved call edges.
            let duplicates = packages
                .iter()
                .filter(|item| item.get("name").and_then(toml::Value::as_str) == Some(name))
                .count();
            let id = format!(
                "locked-package-{}",
                digest(&[owner, name, if duplicates > 1 { version } else { "unique" }])
            );
            let ev = self.evidence(
                blob,
                1,
                1,
                "cargo_lock_resolution",
                DesignResolution::ObservedStructure,
            );
            self.element(
                id.clone(),
                DesignElementKind::Dependency,
                format!("{name} locked package"),
                Some(owner.into()),
            );
            self.fact(
                &id,
                "locked_version",
                version.into(),
                ev.clone(),
                DesignResolution::ObservedStructure,
            );
            self.relation(
                owner,
                &id,
                "records_locked_package",
                &ev,
                DesignResolution::ObservedStructure,
            );
        }
        if packages.len() > 1000 {
            self.coverage.deferred += packages.len() - 1000;
            self.gap("Selected Cargo lockfile exceeds the 1000-package inventory budget");
        }
    }
    fn rust(&mut self, blob: &Blob, owner: &str, cancel: &Cancellation) {
        let facts = match rust_facts(&blob.text, cancel) {
            Ok(facts) => facts,
            Err(_) => {
                self.coverage.failed += 1;
                self.failed_paths.insert(blob.path.clone());
                self.gap("A selected Rust source could not be parsed within the bounded profile");
                return;
            }
        };
        self.coverage.unsupported += facts.unsupported_architectural_declarations;
        for gap in facts.gaps {
            self.gap(gap);
        }
        for contract in facts.contracts {
            if self.elements.len() >= MAX_FACTS {
                self.coverage.deferred += 1;
                self.gap("HLD element budget reached; remaining declarations are deferred");
                break;
            }
            let id = format!("contract-{}", digest(&[owner, &blob.path, &contract.key]));
            let ev = self.evidence(
                blob,
                contract.start,
                contract.end,
                "rust_public_contract",
                DesignResolution::ObservedStructure,
            );
            self.element(
                id.clone(),
                DesignElementKind::Contract,
                contract.name,
                Some(owner.into()),
            );
            self.fact(
                &id,
                "public_signature",
                contract.signature,
                ev.clone(),
                DesignResolution::ObservedStructure,
            );
            self.relation(
                owner,
                &id,
                "declares_public_contract",
                &ev,
                DesignResolution::ObservedStructure,
            );
        }
        for (name, start, end) in facts.imports {
            let ev = self.evidence(
                blob,
                start,
                end,
                "rust_import",
                DesignResolution::ObservedStructure,
            );
            if matches!(
                name.as_str(),
                "crate" | "self" | "super" | "std" | "core" | "alloc"
            ) {
                continue;
            }
            let dependencies = self
                .elements
                .values()
                .filter(|item| {
                    item.kind == DesignElementKind::Dependency
                        && item.component_id.as_deref() == Some(owner)
                        && item
                            .facts
                            .iter()
                            .any(|fact| fact.kind == "crate_alias" && fact.value == name)
                })
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            if dependencies.len() == 1 {
                self.relation(
                    owner,
                    &dependencies[0],
                    "imports_declared_package",
                    &ev,
                    DesignResolution::ResolvedReference,
                );
            } else {
                self.fact(
                    owner,
                    "unresolved_import_root",
                    name,
                    ev,
                    DesignResolution::NameOnlyCandidate,
                );
                self.gap("An imported crate has no unique selected Cargo declaration; component direction remains unresolved");
            }
        }
    }
    fn deployment(&mut self, blob: &Blob, owner: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&blob.text) else {
            self.coverage.failed += 1;
            self.failed_paths.insert(blob.path.clone());
            self.gap("A selected Vercel JSON configuration could not be parsed");
            return;
        };
        if !value.is_object() {
            self.gap("A selected Vercel configuration is not a JSON object");
            return;
        }
        let id = format!("deployment-{}", digest(&[owner, &blob.path]));
        let ev = self.evidence(
            blob,
            1,
            1,
            "static_deployment_configuration",
            DesignResolution::ObservedStructure,
        );
        self.element(
            id.clone(),
            DesignElementKind::Deployment,
            "Vercel configuration".into(),
            Some(owner.into()),
        );
        self.elements.get_mut(&id).unwrap().roots.push(
            blob.path
                .rsplit_once('/')
                .map_or(".", |(parent, _)| parent)
                .into(),
        );
        self.elements.get_mut(&id).unwrap().unknowns.push("Repository configuration is not evidence of an active Vercel deployment or environment".into());
        for key in [
            "framework",
            "outputDirectory",
            "public",
            "cleanUrls",
            "trailingSlash",
        ] {
            if let Some(value) = value.get(key) {
                let bounded = value
                    .as_str()
                    .filter(|text| text.len() <= 256 && !text.chars().any(char::is_control))
                    .map(str::to_owned)
                    .or_else(|| value.as_bool().map(|flag| flag.to_string()));
                if let Some(value) = bounded {
                    self.fact(
                        &id,
                        key,
                        value,
                        ev.clone(),
                        DesignResolution::ObservedStructure,
                    );
                } else {
                    self.gap("A deployment field is outside the bounded static profile");
                }
            }
        }
        // Never return command bodies, environment variables or configuration
        // secrets. Their presence keeps architectural claims qualified.
        for key in [
            "buildCommand",
            "installCommand",
            "devCommand",
            "env",
            "build",
            "functions",
            "routes",
            "rewrites",
            "redirects",
            "headers",
            "regions",
            "crons",
        ] {
            if value.get(key).is_some() {
                self.fact(
                    &id,
                    "unresolved_configuration_field",
                    key.into(),
                    ev.clone(),
                    DesignResolution::Unknown,
                );
                self.gap("Deployment commands, runtime mappings or environment configuration remain unresolved; they are never executed or disclosed");
            }
        }
        self.relation(
            owner,
            &id,
            "has_deployment_configuration",
            &ev,
            DesignResolution::ObservedStructure,
        );
    }
    fn schema(&mut self, blob: &Blob, owner: &str) {
        let id = format!("schema-{}", digest(&[owner, &blob.path]));
        let ev = self.evidence(
            blob,
            1,
            1,
            "schema_artifact",
            DesignResolution::ObservedStructure,
        );
        self.element(
            id.clone(),
            DesignElementKind::Schema,
            format!(
                "{} schema artifact",
                blob.path.rsplit('/').next().unwrap_or("selected")
            ),
            Some(owner.into()),
        );
        // Compare normalized tokens but publish only a digest, never SQL data,
        // GraphQL literals or configuration values. Ownership is not inferred.
        let fingerprint = digest(&[&canonical(&blob.text)]);
        self.fact(
            &id,
            "schema_declaration_fingerprint",
            fingerprint,
            ev.clone(),
            DesignResolution::ObservedStructure,
        );
        self.elements.get_mut(&id).unwrap().unknowns.push("Schema compatibility, data-store instance and exclusive data ownership require reviewer verification".into());
        self.relation(
            owner,
            &id,
            "contains_schema_artifact",
            &ev,
            DesignResolution::ObservedStructure,
        );
        self.gap("Schema artifacts are fingerprinted; compatibility, active store and ownership are unresolved");
    }
}

pub fn generate_hld(
    repository: &Path,
    snapshot: &SnapshotId,
    scope: &ScopeRecord,
    context: &ConfirmedDesignContext,
    cancellation: &Cancellation,
) -> Result<DesignVersion, String> {
    context.validate(scope)?;
    cancellation.checkpoint()?;
    let started = Instant::now();
    let mut builder = Builder {
        snapshot,
        scope,
        context,
        elements: BTreeMap::new(),
        relations: BTreeMap::new(),
        evidence: BTreeMap::new(),
        document_candidates: BTreeMap::new(),
        failed_paths: BTreeSet::new(),
        coverage: DesignCoverage {
            profile: HLD_PROFILE.into(),
            enumeration_complete: true,
            indexed: 0,
            unsupported: 0,
            failed: 0,
            deferred: 0,
            complete: true,
            gaps: vec![],
        },
    };
    for component in &context.components {
        let id = component_id(scope, &component.id);
        let mut roots = component.roots.clone();
        roots.sort();
        roots.dedup();
        builder.elements.insert(id.clone(),DesignElement{id,kind:DesignElementKind::Component,name:component.name.clone(),component_id:Some(component.id.clone()),roots,
            responsibility:Some(component.role.clone()),user_assertions:vec![format!("Confirmed onboarding role: {}",component.role)],facts:vec![],evidence_ids:vec![],
            unknowns:vec!["Confirmed role and boundary express user interpretation; business responsibilities, runtime ownership and deployed service state are not established".into()]});
    }
    let tree = cancelable_tree(repository, snapshot, cancellation);
    let entries = match tree {
        Ok(bytes) => {
            let (entries, unresolved) = parse_tree(&bytes, scope);
            if unresolved > 0 {
                builder.coverage.unsupported += unresolved;
                builder.gap("Selected symlinks, submodules or unsupported tree entries are not followed; their architecture is unresolved");
            }
            entries
        }
        Err(error) => {
            if cancellation.is_cancelled() {
                return Err(error);
            }
            builder.coverage.enumeration_complete = false;
            builder.coverage.failed += 1;
            builder.gap("Pinned tree enumeration is unavailable or exceeds its bounded budget; no complete denominator is claimed");
            vec![]
        }
    };
    let mut blobs = Vec::new();
    let mut remaining = MAX_BYTES;
    if entries.len() > MAX_PATHS {
        builder.coverage.enumeration_complete = false;
        builder.coverage.deferred += entries.len() - MAX_PATHS;
        builder
            .gap("Selected tree exceeds the 2000-path HLD budget; omitted paths are not analyzed");
    }
    let bounded_entries = entries.iter().take(MAX_PATHS).collect::<Vec<_>>();
    let documents = bounded_entries
        .iter()
        .filter(|entry| kind(&entry.0) == "document")
        .collect::<Vec<_>>();
    let selected_documents = documents
        .iter()
        .take(100)
        .map(|entry| entry.0.as_str())
        .collect::<BTreeSet<_>>();
    if documents.len() > 100 {
        builder.coverage.deferred += documents.len() - 100;
        builder.gap("Design document candidate inventory reached 100 entries; additional document intent remains unselected");
    }
    for (chunk_index, entries) in bounded_entries.chunks(32).enumerate() {
        cancellation.checkpoint()?;
        if started.elapsed() > MAX_TIME || remaining == 0 {
            builder.coverage.deferred += bounded_entries.len().saturating_sub(chunk_index * 32);
            builder.gap("HLD ten-second or ten-MiB read budget reached; remaining source evidence is unknown");
            break;
        }
        let selected = entries
            .iter()
            .filter(|entry| {
                kind(&entry.0) != "ignore"
                    && (kind(&entry.0) != "document"
                        || selected_documents.contains(entry.0.as_str()))
            })
            .collect::<Vec<_>>();
        let ids = selected
            .iter()
            .map(|entry| entry.1.clone())
            .collect::<Vec<_>>();
        let results = read_chunk(
            repository,
            &ids,
            ReadLimits {
                objects_per_chunk: 32,
                bytes_per_object: 512 * 1024,
                bytes_per_chunk: remaining,
            },
            cancellation,
        );
        match results {
            Err(error) => {
                if cancellation.is_cancelled() {
                    return Err(error);
                }
                builder.coverage.failed += selected.len();
                builder.gap(
                    "Selected source objects are unavailable or exceed the bounded read budget",
                );
            }
            Ok(results) => {
                for (entry, result) in selected.into_iter().zip(results) {
                    match result {
                        BatchBlob::Data(bytes) => {
                            remaining = remaining.saturating_sub(bytes.len());
                            match String::from_utf8(bytes) {
                                Ok(text) => blobs.push(Blob {
                                    path: entry.0.clone(),
                                    oid: entry.1.clone(),
                                    text,
                                }),
                                Err(_) => {
                                    builder.coverage.failed += 1;
                                    builder.gap("Selected source is not UTF-8 and is not analyzed");
                                }
                            }
                        }
                        _ => {
                            builder.coverage.failed += 1;
                            builder.gap(
                                "Selected source is missing or exceeds the 512-KiB blob limit",
                            );
                        }
                    }
                }
            }
        }
    }
    let packages = blobs
        .iter()
        .filter(|blob| kind(&blob.path) == "cargo")
        .filter_map(|blob| {
            builder.owner(&blob.path).map(|owner| {
                (
                    blob.path
                        .rsplit_once('/')
                        .map_or(".", |(parent, _)| parent)
                        .to_owned(),
                    owner,
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    // Manifests establish selected package identity before imports are resolved.
    for blob in blobs.iter().filter(|blob| kind(&blob.path) == "cargo") {
        cancellation.checkpoint()?;
        if let Some(owner) = builder.owner(&blob.path) {
            builder.manifest(blob, &owner, &packages);
        } else {
            builder.gap("A selected manifest has no unique confirmed component owner");
        }
    }
    for (blob_index, blob) in blobs.iter().enumerate() {
        cancellation.checkpoint()?;
        if started.elapsed() > MAX_TIME {
            builder.coverage.deferred += blobs.len() - blob_index;
            builder.gap("HLD generation reached its ten-second processing budget");
            break;
        }
        if kind(&blob.path) == "document" {
            builder.document(&blob.path, &blob.oid, &blob.text);
            builder.coverage.indexed += 1;
            continue;
        }
        let Some(owner) = builder.owner(&blob.path) else {
            builder.coverage.failed += 1;
            builder.gap("A selected source path has no unique confirmed component owner; no boundary is invented");
            continue;
        };
        let ev = builder.evidence(
            blob,
            1,
            1,
            "component_source_presence",
            DesignResolution::ObservedStructure,
        );
        builder.fact(
            &owner,
            "source_presence",
            blob.path.clone(),
            ev,
            DesignResolution::ObservedStructure,
        );
        match kind(&blob.path) {
            "rust" => builder.rust(blob, &owner, cancellation),
            "cargo" => {}
            "cargo_lock" => builder.lockfile(blob, &owner),
            "vercel" => builder.deployment(blob, &owner),
            "schema" => builder.schema(blob, &owner),
            "unsupported" => {
                builder.coverage.unsupported += 1;
                builder.gap("Selected non-Rust source or unsupported deployment/manifest semantics cannot be analyzed completely");
            }
            _ => {}
        }
        if kind(&blob.path) != "unsupported" && !builder.failed_paths.contains(&blob.path) {
            builder.coverage.indexed += 1;
        }
    }
    for component in &context.components {
        let id = component_id(scope, &component.id);
        if builder.elements[&id].evidence_ids.is_empty() {
            builder.gap("A confirmed component has no observed selected source or manifest at this exact commit");
        }
    }
    // Preserve accepted hosting and integration decisions as interpretation;
    // never reuse an old blob as evidence at a new observed commit.
    for mapping in &context.providers {
        let owner = component_id(scope, &mapping.component_id);
        if let Some(element) = builder.elements.get_mut(&owner) {
            element.user_assertions.push(format!(
                "Selected static hosting context: {} at {}",
                mapping.provider, mapping.source_root
            ));
            element.unknowns.push(format!(
                "Recorded hosting verification {:?} belongs to accepted context commit {}. Current deployment state and environment remain unverified",
                mapping.verification,context.accepted_commit.as_str()
            ));
        }
    }
    for integration in &context.integrations {
        let owner = component_id(scope, &integration.component_id);
        if let Some(element) = builder.elements.get_mut(&owner) {
            element.user_assertions.push(format!(
                "Selected integration context: {} ({})",
                integration.name, integration.kind
            ));
        }
    }
    for gap in &context.gaps {
        builder.gap(gap.clone());
    }
    for element in builder.elements.values_mut() {
        if element.kind == DesignElementKind::Component {
            let mut presence = element
                .facts
                .iter()
                .filter(|fact| fact.kind == "source_presence")
                .flat_map(|fact| fact.evidence_ids.iter().cloned())
                .collect::<Vec<_>>();
            presence.sort();
            presence.dedup();
            presence.truncate(16);
            element.facts.retain(|fact| fact.kind != "source_presence");
            if !presence.is_empty() {
                element.facts.push(DesignFact {
                    kind: "source_presence".into(),
                    value: "Selected source or manifest observed at this exact commit".into(),
                    resolution: DesignResolution::ObservedStructure,
                    evidence_ids: presence,
                });
            }
            element.evidence_ids = element
                .facts
                .iter()
                .flat_map(|fact| fact.evidence_ids.iter().cloned())
                .collect();
        }
        element.evidence_ids.sort();
        element.evidence_ids.dedup();
        element
            .facts
            .sort_by(|a, b| (&a.kind, &a.value).cmp(&(&b.kind, &b.value)));
        element.facts.dedup();
        element.user_assertions.sort();
    }
    for relation in builder.relations.values_mut() {
        relation.evidence_ids.sort();
        relation.evidence_ids.dedup();
    }
    let referenced = builder
        .elements
        .values()
        .flat_map(|element| element.evidence_ids.iter())
        .chain(
            builder
                .relations
                .values()
                .flat_map(|relation| relation.evidence_ids.iter()),
        )
        .chain(
            builder
                .document_candidates
                .values()
                .map(|candidate| &candidate.evidence_id),
        )
        .cloned()
        .collect::<BTreeSet<_>>();
    builder.evidence.retain(|id, _| referenced.contains(id));
    builder.coverage.gaps.sort();
    builder.coverage.gaps.dedup();
    let scope_fingerprint = design_scope_fingerprint(scope);
    let mut components = context.components.clone();
    for component in &mut components {
        component.roots.sort();
        component.roots.dedup();
    }
    components.sort_by(|left, right| left.id.cmp(&right.id));
    let mut providers = context.providers.clone();
    providers.sort_by(|left, right| {
        (&left.component_id, &left.provider, &left.source_root).cmp(&(
            &right.component_id,
            &right.provider,
            &right.source_root,
        ))
    });
    let mut integrations = context.integrations.clone();
    integrations.sort_by(|left, right| left.id.cmp(&right.id));
    let mut gaps = context.gaps.clone();
    gaps.sort();
    let context_semantics=serde_json::to_string(&serde_json::json!({"components":components,"providers":providers,"integrations":integrations,"gaps":gaps})).map_err(|_|"Cannot encode HLD context")?;
    let input_id = digest(&[
        HLD_GENERATOR_VERSION,
        snapshot.as_str(),
        scope.version.as_str(),
        &scope_fingerprint,
        &context.session_id,
        &context.session_revision.to_string(),
        &context.input_fingerprint,
        &context.proposal_fingerprint,
        &context_semantics,
    ]);
    let id = format!("design-{input_id}");
    let mut version = DesignVersion {
        level: DesignLevel::Hld,
        lld: None,
        id,
        generator_version: HLD_GENERATOR_VERSION.into(),
        repository: scope.primary_repository.clone(),
        snapshot: snapshot.clone(),
        scope_version: scope.version.clone(),
        scope_fingerprint,
        onboarding_session_id: context.session_id.clone(),
        onboarding_revision: context.session_revision,
        onboarding_input_fingerprint: context.input_fingerprint.clone(),
        onboarding_proposal_fingerprint: context.proposal_fingerprint.clone(),
        review_state: if builder.coverage.complete {
            DesignReviewState::AwaitingConfirmation
        } else {
            DesignReviewState::Partial
        },
        view: DesignView::Observed,
        elements: builder.elements.into_values().collect(),
        relations: builder.relations.into_values().collect(),
        evidence: builder.evidence.into_values().collect(),
        sections: vec![],
        coverage: builder.coverage,
        constraints: vec![],
        correspondence: vec![],
        document_candidates: builder.document_candidates.into_values().collect(),
        decision_records: vec![],
        mermaid: String::new(),
        markdown: String::new(),
    };
    version.sections = sections(&version);
    version.mermaid = render_hld_mermaid(&version);
    version.markdown = render_hld_markdown(&version);
    // Durable storage has a one-MiB envelope. Keep full component boundaries,
    // but explicitly defer excess facts; unsupported/omitted facts never
    // become a no-change result. Evidence is pruned only with its facts.
    for _ in 0..40 {
        if serde_json::to_vec(&version)
            .map_err(|_| "Cannot encode HLD")?
            .len()
            <= 900 * 1024
        {
            break;
        }
        version.coverage.complete = false;
        version.review_state = DesignReviewState::Partial;
        let gap="HLD storage budget reached; excess derived declarations and their evidence are explicitly deferred".to_string();
        if !version.coverage.gaps.contains(&gap) {
            version.coverage.gaps.push(gap);
        }
        let remove = version
            .elements
            .iter()
            .rev()
            .filter(|element| element.kind != DesignElementKind::Component)
            .take(50)
            .map(|element| element.id.clone())
            .collect::<BTreeSet<_>>();
        if remove.is_empty() {
            if version.document_candidates.is_empty() {
                return Err(
                    "Confirmed component metadata exceeds the bounded HLD storage profile".into(),
                );
            }
            let count = version.document_candidates.len().min(10);
            version
                .document_candidates
                .truncate(version.document_candidates.len() - count);
            version.coverage.deferred += count;
        }
        version.coverage.deferred += remove.len();
        version
            .elements
            .retain(|element| !remove.contains(&element.id));
        version
            .relations
            .retain(|relation| !remove.contains(&relation.from) && !remove.contains(&relation.to));
        let retained = version
            .elements
            .iter()
            .flat_map(|element| element.evidence_ids.iter())
            .chain(
                version
                    .relations
                    .iter()
                    .flat_map(|relation| relation.evidence_ids.iter()),
            )
            .chain(
                version
                    .document_candidates
                    .iter()
                    .map(|candidate| &candidate.evidence_id),
            )
            .cloned()
            .collect::<BTreeSet<_>>();
        version
            .evidence
            .retain(|evidence| retained.contains(&evidence.id));
        version.sections = sections(&version);
        version.mermaid = render_hld_mermaid(&version);
        version.markdown = render_hld_markdown(&version);
    }
    let normalized=serde_json::to_string(&serde_json::json!({"elements":version.elements,"relations":version.relations,"evidence":version.evidence,"coverage":version.coverage,"document_candidates":version.document_candidates})).map_err(|_|"Cannot encode normalized HLD facts")?;
    version.id = format!("design-{}", digest(&[&input_id, &normalized]));
    version.markdown = render_hld_markdown(&version);
    validate_design_scope(&version, scope)?;
    if serde_json::to_vec(&version)
        .map_err(|_| "Cannot encode HLD")?
        .len()
        > 900 * 1024
    {
        return Err("HLD artifact exceeds its bounded storage limit".into());
    }
    cancellation.checkpoint()?;
    Ok(version)
}

pub(crate) fn component_id(scope: &ScopeRecord, id: &str) -> String {
    format!(
        "component-{}",
        digest(&[
            scope.primary_repository.security_domain().as_str(),
            scope.primary_repository.provider(),
            scope.primary_repository.owner(),
            scope.primary_repository.name(),
            id
        ])
    )
}
fn kind(path: &str) -> &'static str {
    if path.ends_with(".rs") {
        "rust"
    } else if path.rsplit('/').next() == Some("Cargo.toml") {
        "cargo"
    } else if path.rsplit('/').next() == Some("Cargo.lock") {
        "cargo_lock"
    } else if path.rsplit('/').next() == Some("vercel.json") {
        "vercel"
    } else if path.ends_with(".sql")
        || path.ends_with(".graphql")
        || path.ends_with(".proto")
        || path.ends_with(".prisma")
    {
        "schema"
    } else if [".md", ".mdx", ".txt"]
        .iter()
        .any(|extension| path.to_ascii_lowercase().ends_with(extension))
    {
        "document"
    } else if [
        ".js", ".jsx", ".ts", ".tsx", ".py", ".go", ".java", ".kt", ".cs", ".rb", ".php", ".cpp",
        ".c", ".toml", ".yaml", ".yml",
    ]
    .iter()
    .any(|ending| path.ends_with(ending))
        || path.rsplit('/').next() == Some("package.json")
    {
        "unsupported"
    } else {
        "ignore"
    }
}
pub(crate) fn parse_tree(bytes: &[u8], scope: &ScopeRecord) -> (Vec<(String, String)>, usize) {
    let mut entries = Vec::new();
    let mut unresolved = 0;
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        let Ok(path) = std::str::from_utf8(&entry[tab + 1..]) else {
            continue;
        };
        if !safe_path(path, scope) {
            continue;
        }
        let Ok(header) = std::str::from_utf8(&entry[..tab]) else {
            continue;
        };
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() == 3
            && matches!(fields[0], "100644" | "100755")
            && fields[1] == "blob"
            && SnapshotId::new(fields[2].into()).is_ok()
        {
            entries.push((path.into(), fields[2].into()));
        } else if fields.get(1) == Some(&"commit") || kind(path) != "ignore" {
            unresolved += 1;
        }
    }
    entries.sort();
    (entries, unresolved)
}
pub(crate) fn cancelable_tree(
    repository: &Path,
    snapshot: &SnapshotId,
    cancel: &Cancellation,
) -> Result<Vec<u8>, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
    };
    let mut child = Command::new("git")
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
        ])
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-r", "-z", snapshot.as_str(), "--"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start HLD Git reader")?;
    let stdout = child.stdout.take().ok_or("HLD Git output unavailable")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() || started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("HLD tree read cancelled or exceeded five seconds".into());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(bytes)) if bytes.len() <= 2 * 1024 * 1024 => loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => return Ok(bytes),
                    Ok(Some(_)) => return Err("HLD tree unavailable".into()),
                    Ok(None)
                        if started.elapsed() < Duration::from_secs(5) && !cancel.is_cancelled() =>
                    {
                        thread::sleep(Duration::from_millis(10))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("HLD tree process unavailable".into());
                    }
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("HLD tree exceeds bounded metadata output".into());
            }
        }
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
fn normalize_relative(parent: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.contains('\\') || target.len() > 256 {
        return None;
    }
    let mut parts = if parent.is_empty() {
        vec![]
    } else {
        parent.split('/').collect::<Vec<_>>()
    };
    for part in target.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            ".git" => return None,
            _ => parts.push(part),
        }
    }
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}
fn canonical(text: &str) -> String {
    let mut output = String::new();
    let mut whitespace = false;
    let mut quote = None;
    let mut escaped = false;
    for character in text.chars() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if character == '\\' {
                escaped = true;
                continue;
            }
            if character == delimiter {
                quote = None;
                output.push_str("<literal>");
            }
            continue;
        }
        if character == '"' {
            quote = Some(character);
            continue;
        }
        if character.is_whitespace() {
            whitespace = true;
            continue;
        }
        if whitespace
            && output
                .chars()
                .last()
                .is_some_and(|previous| previous.is_alphanumeric() || previous == '_')
            && (character.is_alphanumeric() || character == '_')
        {
            output.push(' ');
        }
        whitespace = false;
        output.push(character);
    }
    output
}
fn rust_facts(source: &str, cancel: &Cancellation) -> Result<RustFacts, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| "Cannot load Rust HLD parser")?;
    let started = Instant::now();
    let mut progress = |_: &tree_sitter::ParseState| {
        cancel.is_cancelled() || started.elapsed() > Duration::from_secs(2)
    };
    let mut input = |offset: usize, _: Point| &source.as_bytes()[offset..];
    let tree = parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or("HLD Rust parser cancelled or timed out")?;
    cancel.checkpoint()?;
    if tree.root_node().has_error() {
        return Err("HLD Rust contains syntax recovery".into());
    }
    let mut result = RustFacts::default();
    walk_rust(
        tree.root_node(),
        source,
        &mut Vec::new(),
        false,
        &mut result,
        cancel,
        0,
    )?;
    Ok(result)
}
fn walk_rust(
    node: Node<'_>,
    source: &str,
    scope: &mut Vec<String>,
    in_test: bool,
    result: &mut RustFacts,
    cancel: &Cancellation,
    depth: usize,
) -> Result<(), String> {
    cancel.checkpoint()?;
    if depth > 128 {
        return Err("HLD Rust syntax nesting exceeds the 128-level limit".into());
    }
    let text = &source[node.byte_range()];
    let test = in_test
        || (node.kind() == "attribute_item" && canonical(text) == "#[cfg(test)]")
        || test_attribute(node, source);
    if test {
        return Ok(());
    }
    if node.kind() == "attribute_item" && text.contains("cfg") {
        result.gaps.insert("Conditional Rust declarations are not feature-evaluated; relevant architecture remains qualified".into());
    }
    if node.kind() == "attribute_item"
        && [
            "derive",
            "serde",
            "route",
            "get(",
            "post(",
            "put(",
            "delete(",
            "async_trait",
            "repr(",
        ]
        .iter()
        .any(|name| text.contains(name))
    {
        result.gaps.insert("Rust procedural/contract attributes are not expanded or evaluated; their architectural effects remain unresolved".into());
    }
    let public = node.child_by_field_name("visibility_modifier").is_some() || {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .any(|child| child.kind() == "visibility_modifier")
    };
    let name = node
        .child_by_field_name("name")
        .map(|node| &source[node.byte_range()]);
    if node.kind() == "union_item" {
        if public {
            result.unsupported_architectural_declarations += 1;
            result.gaps.insert("Public Rust union declarations are outside the architectural contract profile; representation and exposed fields remain unresolved".into());
        }
        // A union's fields/representation must not become independent public
        // contracts. Private unions without public architectural evidence stay
        // in the LLD profile; do not infer a high-level delta from their bodies.
        return Ok(());
    }
    if matches!(
        node.kind(),
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "mod_item"
    ) && public
    {
        if let Some(name) = name {
            let signature = declaration_signature(node, source);
            if signature.len() > 4096 || result.contracts.len() >= 1000 {
                result
                    .gaps
                    .insert("Public declaration exceeds bounded HLD contract inventory".into());
            } else {
                result.contracts.push(Contract {
                    key: format!("{}:{}:{name}", scope.join("::"), node.kind()),
                    name: name.into(),
                    signature,
                    start: node.start_position().row + 1,
                    end: node.end_position().row + 1,
                });
            }
        }
    }
    if node.kind() == "use_declaration" {
        let statement = canonical(text);
        if public {
            if statement.len() <= 256 {
                result.contracts.push(Contract {
                    key: format!("{}:public_reexport:{statement}", scope.join("::")),
                    name: statement.clone(),
                    signature: statement.clone(),
                    start: node.start_position().row + 1,
                    end: node.end_position().row + 1,
                });
            } else {
                result
                    .gaps
                    .insert("Public re-export exceeds the bounded HLD declaration profile".into());
            }
        }
        let argument = statement
            .strip_prefix("pub use")
            .or_else(|| statement.strip_prefix("use"))
            .unwrap_or(&statement);
        let root = argument
            .trim()
            .trim_start_matches(':')
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .next()
            .unwrap_or("");
        if identifier(root) {
            result.imports.push((
                root.into(),
                node.start_position().row + 1,
                node.end_position().row + 1,
            ));
        } else {
            result.gaps.insert("Grouped or unresolved Rust imports require resolution outside this bounded HLD profile".into());
        }
    }
    // Function implementations and private method bodies are LLD. Do not
    // inflate HLD semantic changes with their internal calls or literals.
    if matches!(node.kind(), "function_item" | "function_signature_item") {
        return Ok(());
    }
    if node.kind() == "macro_invocation" {
        result.gaps.insert(
            "Item-level macros may create architectural declarations and are not expanded".into(),
        );
        return Ok(());
    }
    let enters = matches!(node.kind(), "mod_item" | "impl_item" | "trait_item");
    if enters {
        if let Some(name) = name {
            scope.push(name.into());
        } else if node.kind() == "impl_item" {
            let header = node
                .child_by_field_name("body")
                .map_or(text, |body| &source[node.start_byte()..body.start_byte()]);
            scope.push(canonical(header));
        } else {
            scope.push(node.kind().into());
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_rust(child, source, scope, test, result, cancel, depth + 1)?;
    }
    if enters {
        scope.pop();
    }
    Ok(())
}
fn test_attribute(node: Node<'_>, source: &str) -> bool {
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() != "attribute_item" {
            break;
        }
        let text = &source[sibling.byte_range()];
        if text.contains("cfg(test)") {
            return true;
        }
        previous = sibling.prev_named_sibling();
    }
    false
}
fn declaration_signature(node: Node<'_>, source: &str) -> String {
    if matches!(node.kind(), "const_item" | "static_item") {
        if let Some(value) = node.child_by_field_name("value") {
            return format!(
                "{} = initializer:{}",
                canonical(&source[node.start_byte()..value.start_byte()]),
                digest(&[&source[value.byte_range()]])
            );
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        let header = &source[node.start_byte()..body.start_byte()];
        if matches!(node.kind(), "function_item" | "mod_item") {
            return canonical(header);
        }
        if node.kind() == "trait_item" {
            let mut parts = vec![canonical(header)];
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                if child.kind() != "attribute_item" {
                    parts.push(declaration_signature(child, source));
                }
            }
            return format!("{}{{{}}}", parts.remove(0), parts.join(";"));
        }
    }
    canonical(&source[node.byte_range()])
}
fn sections(version: &DesignVersion) -> Vec<DesignSection> {
    let ids = |kind: DesignElementKind| {
        version
            .elements
            .iter()
            .filter(|item| item.kind == kind)
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    };
    let relations = |kind: &str| {
        version
            .relations
            .iter()
            .filter(|item| item.kind == kind)
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    };
    vec![
        DesignSection{id:"identity".into(),title:"Identity and coverage".into(),element_ids:vec![],relation_ids:vec![],unknowns:version.coverage.gaps.clone()},
        DesignSection{id:"system_context".into(),title:"System context".into(),element_ids:ids(DesignElementKind::Dependency),relation_ids:relations("local_package_dependency"),unknowns:vec!["Actors, users and live external systems need evidence or explicit user input".into()]},
        DesignSection{id:"components".into(),title:"Components and boundaries".into(),element_ids:ids(DesignElementKind::Component),relation_ids:vec![],unknowns:vec!["User-confirmed component roles do not prove business responsibility or live service independence".into()]},
        DesignSection{id:"interactions".into(),title:"Interactions and contracts".into(),element_ids:ids(DesignElementKind::Contract),relation_ids:version.relations.iter().filter(|item|item.kind.contains("dependency")||item.kind.contains("imports")).map(|item|item.id.clone()).collect(),unknowns:vec!["Static dependency arrows do not establish runtime call direction, protocol or sync/async behavior".into()]},
        DesignSection{id:"data_ownership".into(),title:"Data ownership".into(),element_ids:ids(DesignElementKind::Schema),relation_ids:relations("contains_schema_artifact"),unknowns:vec!["Schema presence or a data client does not establish exclusive ownership, an active store or consistency guarantees".into()]},
        DesignSection{id:"deployment".into(),title:"Deployment topology".into(),element_ids:ids(DesignElementKind::Deployment),relation_ids:relations("has_deployment_configuration"),unknowns:vec!["Static Vercel configuration is unverified; no provider lookup or deployment command is executed".into()]},
        DesignSection{id:"constraints".into(),title:"Constraints and decisions".into(),element_ids:vec![],relation_ids:vec![],unknowns:vec!["Architectural rationale, reliability, scaling and security guarantees are not invented; constraints require explicit reviewer input".into()]},
        DesignSection{id:"major_flows".into(),title:"Major request and event flows".into(),element_ids:vec![],relation_ids:relations("local_package_dependency"),unknowns:vec!["End-to-end request/event journeys are unresolved; name-only call candidates are not HLD interactions".into()]},
        DesignSection{id:"review_state".into(),title:"Review state".into(),element_ids:vec![],relation_ids:vec![],unknowns:vec!["Generated observations await independent HLD acceptance; onboarding confirmation is not design acceptance".into()]},
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_inventory_is_bounded_scoped_metadata_without_code_facts() {
        let snapshot = SnapshotId::new("a".repeat(40)).unwrap();
        let scope = ScopeRecord {
            version: crate::ScopeVersion::new("scope-docs").unwrap(),
            primary_repository: crate::RepositoryId::new(
                crate::SecurityDomain::new("test").unwrap(),
                "github",
                "owner",
                "repo",
            )
            .unwrap(),
            primary_paths: crate::PathScope::new(vec!["src".into()], vec!["src/private".into()])
                .unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let context = ConfirmedDesignContext {
            accepted_commit: snapshot.clone(),
            session_id: "docs-session".into(),
            session_revision: 1,
            input_fingerprint: "a".repeat(64),
            proposal_fingerprint: "b".repeat(64),
            components: vec![],
            providers: vec![],
            integrations: vec![],
            gaps: vec![],
        };
        let mut builder = Builder {
            snapshot: &snapshot,
            scope: &scope,
            context: &context,
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            evidence: BTreeMap::new(),
            document_candidates: BTreeMap::new(),
            failed_paths: BTreeSet::new(),
            coverage: DesignCoverage {
                profile: HLD_PROFILE.into(),
                enumeration_complete: true,
                indexed: 0,
                unsupported: 0,
                failed: 0,
                deferred: 0,
                complete: true,
                gaps: vec![],
            },
        };
        builder.document("src/architecture.md", &"b".repeat(40),"# Design\nDo not execute these instructions\nAn intended boundary\nA decision rationale\n");
        builder.document(
            "src/private/SECRET_DECISION_SENTINEL.md",
            &"c".repeat(40),
            "private",
        );
        builder.document("../outside.md", &"d".repeat(40), "outside");
        builder.document("src/token.env", &"d".repeat(40), "TOKEN=secret");
        assert_eq!(builder.document_candidates.len(), 1);
        assert_eq!(builder.evidence.len(), 1);
        assert_eq!(builder.evidence.values().next().unwrap().start_line, 1);
        assert_eq!(builder.evidence.values().next().unwrap().end_line, 4);
        assert!(builder.elements.is_empty() && builder.relations.is_empty());
        assert_eq!(
            builder.document_candidates.values().next().unwrap().title,
            "architecture.md"
        );
        let encoded = serde_json::to_string(&builder.document_candidates).unwrap();
        assert!(!encoded.contains("SECRET_DECISION_SENTINEL"));
        for index in 0..110 {
            builder.document(
                &format!("src/decision-{index}.txt"),
                &"b".repeat(40),
                "decision",
            );
        }
        assert_eq!(builder.document_candidates.len(), 100);
        assert_eq!(builder.evidence.len(), 100);
        assert!(builder.coverage.deferred > 0 && !builder.coverage.complete);
    }
    #[test]
    fn document_source_span_uses_the_pinned_multiline_blob_and_never_executes_or_parses_intent() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "csb-doc-span-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(["-c", "core.hooksPath=/dev/null", "-C"])
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Docs fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(root.join("src/lib.rs"), "pub fn entry()->u32 { 1 }\n").unwrap();
        std::fs::write(root.join("docs/decision.md"),"# Intended boundary\nA proposed responsibility\n<script>DOC_INSTRUCTION_SENTINEL</script>\nReviewer rationale\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "Pinned docs"]);
        let snapshot = SnapshotId::new(git(&["rev-parse", "HEAD"])).unwrap();
        let scope = ScopeRecord {
            version: crate::ScopeVersion::new("docs-pinned-scope").unwrap(),
            primary_repository: crate::RepositoryId::new(
                crate::SecurityDomain::new("docs-fixture").unwrap(),
                "github",
                "owner",
                "repo",
            )
            .unwrap(),
            primary_paths: crate::PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let component = crate::ComponentProposal {
            id: "engine".into(),
            name: "Engine".into(),
            role: "library".into(),
            repository_index: 0,
            roots: vec![".".into()],
            stack_candidates: vec![],
            signals: vec![],
            evidence: vec![],
            confidence: crate::EvidenceConfidence::UserSupplied,
            contradictions: vec![],
            decision: crate::ComponentDecision::Corrected,
        };
        let context = ConfirmedDesignContext {
            accepted_commit: snapshot.clone(),
            session_id: "docs-pinned-session".into(),
            session_revision: 1,
            input_fingerprint: "a".repeat(64),
            proposal_fingerprint: "b".repeat(64),
            components: vec![component],
            providers: vec![],
            integrations: vec![],
            gaps: vec![],
        };
        let before =
            generate_hld(&root, &snapshot, &scope, &context, &Cancellation::default()).unwrap();
        let doc = &before.document_candidates[0];
        let evidence = before
            .evidence
            .iter()
            .find(|evidence| evidence.id == doc.evidence_id)
            .unwrap();
        assert_eq!((evidence.start_line, evidence.end_line), (1, 4));
        assert_eq!(evidence.commit, snapshot);
        assert_eq!(evidence.blob, git(&["rev-parse", "HEAD:docs/decision.md"]));
        assert!(before.decision_records.is_empty());
        assert!(!before.markdown.contains("DOC_INSTRUCTION_SENTINEL"));
        assert!(
            before
                .elements
                .iter()
                .flat_map(|element| &element.facts)
                .all(|fact| !fact.value.contains("DOC_INSTRUCTION_SENTINEL"))
        );
        std::fs::write(
            root.join("docs/decision.md"),
            "Different mutable worktree document\n",
        )
        .unwrap();
        let after =
            generate_hld(&root, &snapshot, &scope, &context, &Cancellation::default()).unwrap();
        assert_eq!(before, after);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rust_contract_profile_ignores_private_bodies_and_default_trait_bodies() {
        let before = "pub trait Gateway { fn capture(&self)->i32 { 1 } }\npub fn pay()->i32 { helper() }\nfn helper()->i32 { 1 }\n";
        let after = "pub trait Gateway { fn capture(&self) -> i32 { 20 } }\npub fn pay() -> i32 { helper()+3 }\nfn helper()->i32 { println!(\"private\"); 80 }\n";
        let facts = |source| {
            rust_facts(source, &Cancellation::default())
                .unwrap()
                .contracts
                .into_iter()
                .map(|item| (item.key, item.signature))
                .collect::<Vec<_>>()
        };
        assert_eq!(facts(before), facts(after));
        assert_ne!(facts(before), facts(&after.replace("-> i32", "-> u64")));
    }
    #[test]
    fn test_only_imports_are_not_service_dependencies() {
        let facts = rust_facts(
            "#[cfg(test)] mod tests { use sqlx::Pool; pub trait Fake {} }\nuse reqwest::Client;",
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(facts.imports.len(), 1);
        assert_eq!(facts.imports[0].0, "reqwest");
        assert!(facts.contracts.is_empty());
    }
    #[test]
    fn cancelled_and_recovered_syntax_never_produces_complete_facts() {
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(rust_facts("pub struct S;", &cancel).is_err());
        assert!(rust_facts("pub fn broken( {", &Cancellation::default()).is_err());
    }
    #[test]
    fn same_named_methods_keep_distinct_type_scopes() {
        let facts=rust_facts("pub struct A; pub struct B; impl A {pub fn same()->i32 {1}} impl B {pub fn same()->i32 {2}}",&Cancellation::default()).unwrap();
        let keys = facts
            .contracts
            .iter()
            .filter(|item| item.name == "same")
            .map(|item| &item.key)
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), 2);
    }
    #[test]
    fn deeply_nested_syntax_is_bounded_before_stack_exhaustion() {
        let source = format!(
            "{}pub struct Deep;{}",
            "mod nested {".repeat(130),
            "}".repeat(130)
        );
        assert!(rust_facts(&source, &Cancellation::default()).is_err());
    }
    #[test]
    fn public_reexports_are_contracts_even_when_import_root_is_internal() {
        let before = rust_facts("pub use crate::Old;", &Cancellation::default()).unwrap();
        let after = rust_facts("pub use crate::New;", &Cancellation::default()).unwrap();
        assert_eq!(before.contracts.len(), 1);
        assert_ne!(before.contracts[0].signature, after.contracts[0].signature);
    }
    #[test]
    fn public_union_field_changes_are_incomplete_and_old_hld_profiles_remain_readable() {
        struct Temporary(std::path::PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = Temporary(std::env::temp_dir().join(format!(
            "csb-public-union-hld-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )));
        std::fs::create_dir_all(root.0.join("src")).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(["-c", "core.hooksPath=/dev/null", "-C"])
                .arg(&root.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Union HLD fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        let commit = |source: &str| {
            std::fs::write(root.0.join("src/lib.rs"), source).unwrap();
            git(&["add", "."]);
            git(&["commit", "--no-gpg-sign", "-q", "-m", "Pinned union source"]);
            SnapshotId::new(git(&["rev-parse", "HEAD"])).unwrap()
        };
        let before_commit = commit(
            "pub union RawBits { pub integer: u32, pub floating: f32 }\npub fn stable()->u32 { 1 }\n",
        );
        let scope = ScopeRecord {
            version: ScopeVersion::new("public-union-scope").unwrap(),
            primary_repository: RepositoryId::new(
                crate::SecurityDomain::new("fixture").unwrap(),
                "github",
                "test",
                "public-union",
            )
            .unwrap(),
            primary_paths: crate::PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let context = ConfirmedDesignContext {
            accepted_commit: before_commit.clone(),
            session_id: "public-union-context".into(),
            session_revision: 1,
            input_fingerprint: "a".repeat(64),
            proposal_fingerprint: "b".repeat(64),
            components: vec![crate::ComponentProposal {
                id: "library".into(),
                name: "Library".into(),
                role: "library".into(),
                repository_index: 0,
                roots: vec![".".into()],
                stack_candidates: vec![],
                signals: vec![],
                evidence: vec![],
                confidence: crate::EvidenceConfidence::UserSupplied,
                contradictions: vec![],
                decision: crate::ComponentDecision::Corrected,
            }],
            providers: vec![],
            integrations: vec![],
            gaps: vec![],
        };
        let before = generate_hld(
            &root.0,
            &before_commit,
            &scope,
            &context,
            &Cancellation::default(),
        )
        .unwrap();
        let after_commit = commit(
            "pub union RawBits { pub integer: u64, pub floating: f64 }\npub fn stable()->u32 { 1 }\n",
        );
        let after = generate_hld(
            &root.0,
            &after_commit,
            &scope,
            &context,
            &Cancellation::default(),
        )
        .unwrap();
        for design in [&before, &after] {
            assert!(!design.coverage.complete);
            assert_eq!(design.coverage.unsupported, 1);
            assert!(
                design
                    .coverage
                    .gaps
                    .iter()
                    .any(|gap| gap.contains("Public Rust union"))
            );
            assert!(
                design
                    .elements
                    .iter()
                    .filter(|element| element.kind == DesignElementKind::Contract)
                    .all(|element| element.name == "stable")
            );
        }
        assert_eq!(
            compare_hld(&before, &after).unwrap().outcome,
            DesignOutcome::Incomplete
        );
        assert_eq!(before.generator_version, "csb-hld-rust-static-v3");
        // Reading immutable v2 history does not relabel it as the v3 profile.
        let mut legacy = before.clone();
        legacy.generator_version = "csb-hld-rust-static-v2".into();
        for evidence in &mut legacy.evidence {
            evidence.producer_version = legacy.generator_version.clone();
        }
        validate_design_scope(&legacy, &scope).unwrap();
        let decoded: DesignVersion =
            serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(decoded, legacy);
        assert!(compare_hld(&decoded, &after).is_err());
    }
    #[test]
    fn private_union_fields_do_not_become_hld_contracts_or_public_coverage_gaps() {
        let before = rust_facts(
            "union PrivateBits { pub field: u32 }\npub fn stable()->u32 { 1 }",
            &Cancellation::default(),
        )
        .unwrap();
        let after = rust_facts(
            "union PrivateBits { pub field: u64 }\npub fn stable()->u32 { 1 }",
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(before.unsupported_architectural_declarations, 0);
        assert_eq!(after.unsupported_architectural_declarations, 0);
        assert!(before.gaps.is_empty() && after.gaps.is_empty());
        assert_eq!(
            before
                .contracts
                .iter()
                .map(|item| (&item.key, &item.signature))
                .collect::<Vec<_>>(),
            after
                .contracts
                .iter()
                .map(|item| (&item.key, &item.signature))
                .collect::<Vec<_>>()
        );
        assert!(before.contracts.iter().all(|item| item.name == "stable"));
    }
    #[test]
    fn relative_package_paths_cannot_escape_repository() {
        assert_eq!(
            normalize_relative("services/api", "../../shared"),
            Some("shared".into())
        );
        assert!(normalize_relative("", "../outside").is_none());
        assert!(normalize_relative("api", "/outside").is_none());
    }
}
