use super::*;
use crate::design::{
    ConfirmedDesignContext, DesignCoverage, DesignLevel, DesignResolution, DesignReviewState,
    DesignView, design_scope_fingerprint, digest, safe_path, under,
};
use crate::{
    Cancellation, ScopeRecord,
    git::BatchBlob,
    snapshot::{ReadLimits, read_chunk},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser, Point, Tree};

const MAX_PATHS: usize = 2000;
const MAX_BLOB: usize = 512 * 1024;
const MAX_TIME: Duration = Duration::from_secs(10);

pub(crate) fn parse(source: &str, cancellation: &Cancellation) -> Result<Tree, String> {
    cancellation.checkpoint()?;
    if source.len() > MAX_BLOB {
        return Err("LLD blob exceeds 512-KiB immutable input limit".into());
    }
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| "Cannot initialize LLD Rust grammar")?;
    let started = Instant::now();
    let mut progress = |_: &tree_sitter::ParseState| {
        cancellation.is_cancelled() || started.elapsed() >= Duration::from_secs(2)
    };
    let mut input = |offset: usize, _: Point| &source.as_bytes()[offset..];
    parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or_else(|| "LLD parse cancelled or reached its two-second limit".into())
}

/// Length-prefixed AST leaves preserve literal bytes and token boundaries.
/// Comments/coordinates are omitted; whitespace inside literals is retained.
pub(crate) fn semantic_tokens(node: Node<'_>, source: &str) -> String {
    let mut result = String::new();
    let mut cursor = node.walk();
    let mut depth = 0usize;
    let mut visited = 0usize;
    loop {
        let current = cursor.node();
        visited += 1;
        if visited > 20_000 || depth > 128 || result.len() > 2 * 1024 * 1024 {
            // Exact bounded source digest prevents omitted bytes from silently
            // looking equal. Coverage separately records the traversal gap;
            // this conservative fallback does not assert normalization.
            return format!(
                "lld-normalization-limit:{}",
                digest(&[source.get(node.byte_range()).unwrap_or("")])
            );
        }
        let comment = matches!(current.kind(), "line_comment" | "block_comment");
        let leaf = current.child_count() == 0
            || matches!(
                current.kind(),
                "string_literal"
                    | "raw_string_literal"
                    | "char_literal"
                    | "byte_string_literal"
                    | "raw_byte_string_literal"
            );
        if !comment && leaf {
            if let Some(token) = source.get(current.byte_range()) {
                result.push_str(&token.len().to_string());
                result.push(':');
                result.push_str(token);
            }
        }
        if !comment && !leaf && cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return result;
            }
            depth = depth.saturating_sub(1);
        }
    }
}

fn node_text(source: &str, node: Node<'_>) -> String {
    source.get(node.byte_range()).unwrap_or("").into()
}
fn compact(text: &str, max: usize) -> String {
    let value = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.len() <= max {
        value
    } else {
        let mut end = max.saturating_sub(3);
        while !value.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        value[..end].to_string() + "…"
    }
}
pub(super) fn parent_binding(hld: &DesignVersion, path: &str) -> LldParentBinding {
    let mut candidates = BTreeSet::new();
    let mut specificity = 0;
    for component in hld
        .elements
        .iter()
        .filter(|item| item.kind == crate::design::DesignElementKind::Component)
    {
        for root in &component.roots {
            if !under(path, root) {
                continue;
            }
            let depth = if root == "." {
                0
            } else {
                root.split('/').count()
            };
            if depth > specificity {
                specificity = depth;
                candidates.clear();
            }
            if depth == specificity {
                candidates.insert(component.id.clone());
            }
        }
    }
    let candidates = candidates.into_iter().collect::<Vec<_>>();
    let state = match candidates.len() {
        0 => "unmapped",
        1 => "matched",
        _ => "ambiguous",
    };
    LldParentBinding { hld_design_id: hld.id.clone(), component_id: (candidates.len() == 1).then(|| candidates[0].clone()), mapping_fingerprint: digest(&[&hld.id, path, state, &candidates.join("|")]), candidate_component_ids: candidates, state: state.into(), mapping_basis: "Authorized most-specific confirmed file containment; business ownership is not inferred".into() }
}
struct Extractor<'a> {
    catalog: &'a mut LldCatalog,
    hld: &'a DesignVersion,
    path: &'a str,
    blob: &'a str,
    source: &'a str,
    cancellation: &'a Cancellation,
    started: Instant,
}
impl Extractor<'_> {
    fn evidence(&mut self, node: Node<'_>, kind: &str) -> String {
        let id = format!(
            "lld-evidence-{}",
            digest(&[
                self.catalog.snapshot.as_str(),
                self.catalog.scope_version.as_str(),
                self.path,
                self.blob,
                &node.start_byte().to_string(),
                &node.end_byte().to_string(),
                kind
            ])
        );
        self.catalog.evidence.push(DesignEvidence {
            id: id.clone(),
            repository: self.catalog.repository.clone(),
            commit: self.catalog.snapshot.clone(),
            path: self.path.into(),
            blob: self.blob.into(),
            start_line: node.start_position().row + 1,
            end_line: (node.end_position().row + usize::from(node.end_position().column > 0))
                .max(node.start_position().row + 1),
            producer_version: LLD_GENERATOR_VERSION.into(),
            relationship_type: kind.into(),
            resolution: DesignResolution::ObservedStructure,
        });
        id
    }
    fn entry(
        &mut self,
        node: Node<'_>,
        module: &str,
        container: &str,
        parent: Option<&str>,
        kind: &str,
        name: String,
    ) -> Option<String> {
        if self.catalog.entries.len() >= MAX_LLD_ENTRIES {
            self.catalog.coverage.deferred += 1;
            self.catalog.coverage.structure_complete = false;
            return None;
        }
        let logical_key = format!("{module}|{container}|{kind}|{name}");
        if logical_key.len() > 4096 {
            self.catalog.coverage.deferred += 1;
            self.catalog.coverage.structure_complete = false;
            return None;
        }
        let id = format!(
            "lld-entry-{}",
            digest(&[
                self.catalog.repository.security_domain().as_str(),
                self.catalog.repository.provider(),
                self.catalog.repository.owner(),
                self.catalog.repository.name(),
                self.catalog.snapshot.as_str(),
                self.catalog.scope_version.as_str(),
                self.path,
                self.blob,
                &node.start_byte().to_string(),
                &node.end_byte().to_string(),
                kind
            ])
        );
        let body = node.child_by_field_name("body");
        let mut signature_tokens = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if body.is_none_or(|body| child.id() != body.id()) {
                signature_tokens.push_str(&semantic_tokens(child, self.source));
            }
        }
        // Schema/config modules have no Rust children; fingerprint exact bytes.
        if signature_tokens.is_empty() {
            signature_tokens = semantic_tokens(node, self.source);
        }
        let signature_end = body.map_or(node.end_byte(), |body| body.start_byte());
        let signature = compact(
            self.source
                .get(node.start_byte()..signature_end)
                .unwrap_or(""),
            4096,
        );
        let visibility = node
            .named_children(&mut node.walk())
            .find(|child| child.kind() == "visibility_modifier")
            .map(|child| compact(&node_text(self.source, child), 128))
            .unwrap_or_else(|| "private".into());
        let evidence_id = self.evidence(node, "lld_declaration");
        let mut projections = BTreeMap::<String, Vec<String>>::new();
        if let Some(body) = body {
            projection(body, self.source, &mut projections, 0);
        }
        let projections = projections
            .into_iter()
            .map(|(kind, values)| LldProjection {
                kind,
                count: values.len(),
                fingerprint: digest(&values.iter().map(String::as_str).collect::<Vec<_>>()),
            })
            .collect();
        let mut call_observations = vec![];
        if let Some(body) = body {
            calls(body, self.source, &mut call_observations, 0);
        }
        let mut limitations = vec![];
        if node.has_error() {
            limitations.push(
                "Syntax recovery/missing nodes prevent complete declaration or local flow claims"
                    .into(),
            );
            self.catalog.coverage.structure_complete = false;
        }
        if kind == "function" {
            let local_gaps = super::flow::local_syntax_limitations(node, self.source);
            if !local_gaps.is_empty() {
                self.catalog.coverage.structure_complete = false;
                self.catalog.coverage.unsupported += 1;
            }
            limitations.extend(local_gaps);
        }
        if signature_tokens.contains("lld-normalization-limit:")
            || body.is_some_and(|body| {
                semantic_tokens(body, self.source).starts_with("lld-normalization-limit:")
            })
        {
            limitations.push("Semantic normalization reached its depth/node/token bound; exact source digest retained as conservative fallback".into());
            self.catalog.coverage.structure_complete = false;
        }
        let mut previous = node.prev_named_sibling();
        while let Some(attribute) = previous.filter(|item| item.kind() == "attribute_item") {
            let text = node_text(self.source, attribute);
            if text.contains("cfg") || text.contains("derive") {
                limitations.push("Conditional compilation or generated derive implementations are not expanded or selected".into());
                self.catalog.coverage.structure_complete = false;
            }
            previous = attribute.prev_named_sibling();
        }
        let parent_binding = parent_binding(self.hld, self.path);
        if parent_binding.state != "matched" {
            limitations.push(format!(
                "HLD parent file containment is {}",
                parent_binding.state
            ));
        }
        self.catalog.entries.push(LldEntry {
            id: id.clone(),
            logical_key,
            name,
            kind: kind.into(),
            module: module.into(),
            container: container.into(),
            parent_entry_id: parent.map(str::to_owned),
            child_entry_ids: vec![],
            path: self.path.into(),
            blob: self.blob.into(),
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: (node.end_position().row + usize::from(node.end_position().column > 0))
                .max(node.start_position().row + 1),
            visibility,
            signature,
            signature_fingerprint: digest(&[&signature_tokens]),
            body_fingerprint: digest(&[&body
                .map(|body| semantic_tokens(body, self.source))
                .unwrap_or_default()]),
            projections,
            call_observations,
            detail_state: if kind == "function" && body.is_some() {
                "available"
            } else if kind == "function" {
                "declaration_only"
            } else {
                "structure_available"
            }
            .into(),
            evidence_ids: vec![evidence_id],
            parent: parent_binding,
            limitations,
        });
        if let Some(parent) = parent {
            if let Some(item) = self
                .catalog
                .entries
                .iter_mut()
                .find(|item| item.id == parent)
            {
                item.child_entry_ids.push(id.clone());
            }
        }
        Some(id)
    }
    fn visit(
        &mut self,
        node: Node<'_>,
        module: &str,
        container: &str,
        parent: Option<&str>,
        depth: usize,
    ) -> Result<(), String> {
        self.cancellation.checkpoint()?;
        if self.started.elapsed() > MAX_TIME {
            self.catalog.coverage.structure_complete = false;
            self.catalog.coverage.deferred += 1;
            return Ok(());
        }
        if depth > 128 {
            self.catalog.coverage.structure_complete = false;
            self.catalog.coverage.deferred += 1;
            return Ok(());
        }
        if node.kind() == "union_item" {
            // Union representation, active fields and unsafe reads require
            // a separate supported profile. Preserve a source-backed frontier
            // instead of attaching its fields to the enclosing file/type.
            let name = node
                .child_by_field_name("name")
                .map(|name| compact(&node_text(self.source, name), 256))
                .unwrap_or_else(|| "unnamed union".into());
            let id = self.entry(
                node,
                module,
                container,
                parent,
                "unsupported_declaration",
                name,
            );
            let gap = "Union declaration semantics and field ownership are outside the supported structural profile; nested fields are not indexed under another owner";
            if let Some(entry) =
                id.and_then(|id| self.catalog.entries.iter_mut().find(|entry| entry.id == id))
            {
                entry.detail_state = "unsupported".into();
                entry.limitations.push(gap.into());
            }
            self.catalog.coverage.unsupported += 1;
            self.catalog.coverage.structure_complete = false;
            if self.catalog.coverage.gaps.len() < 64 {
                self.catalog.coverage.gaps.push(gap.into());
            }
            return Ok(());
        }
        let mut current_module = module.to_string();
        let mut current_container = container.to_string();
        let mut current_parent = parent.map(str::to_owned);
        let kind = match node.kind() {
            "mod_item" => Some("module"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "trait_item" => Some("trait"),
            "function_item" | "function_signature_item" => Some("function"),
            "type_item" => Some("type_alias"),
            "const_item" => Some("constant"),
            "static_item" => Some("static"),
            "field_declaration" => Some("field"),
            "parameter" | "self_parameter" => Some("parameter"),
            "enum_variant" => Some("variant"),
            "use_declaration" => Some("import"),
            _ => None,
        };
        if node.kind() == "impl_item" {
            let type_name = node
                .child_by_field_name("type")
                .map(|item| semantic_tokens(item, self.source))
                .unwrap_or_else(|| "unknown-self".into());
            let trait_name = node
                .child_by_field_name("trait")
                .map(|item| semantic_tokens(item, self.source));
            current_container = match trait_name {
                Some(trait_name) => format!("impl<{trait_name}>for<{type_name}>"),
                None => format!("impl<{type_name}>"),
            };
        }
        if let Some(kind) = kind {
            let name = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("pattern"))
                .map(|name| compact(&node_text(self.source, name), 256))
                .unwrap_or_else(|| compact(&node_text(self.source, node), 256));
            current_parent = self.entry(node, module, container, parent, kind, name.clone());
            if kind == "module" {
                current_module = format!("{module}::{name}");
            }
            if matches!(kind, "struct" | "enum" | "trait" | "function") {
                current_container = format!("{container}::{kind}<{name}>");
            }
        }
        // Call/closure expression subtrees are not declarations; their local
        // behavior is lowered lazily with separate executable region scopes.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(
                child,
                &current_module,
                &current_container,
                current_parent.as_deref(),
                depth + 1,
            )?;
        }
        Ok(())
    }
}
fn projection(
    node: Node<'_>,
    source: &str,
    output: &mut BTreeMap<String, Vec<String>>,
    depth: usize,
) {
    if depth > 128 {
        return;
    }
    let kind = match node.kind() {
        "if_expression" | "while_expression" => Some("condition"),
        "match_arm" => Some("pattern"),
        "return_expression" | "break_expression" | "continue_expression" | "try_expression" => {
            Some("exit")
        }
        "call_expression" => Some("call"),
        "await_expression" => Some("await"),
        "assignment_expression" | "compound_assignment_expr" | "let_declaration" => {
            Some("assignment")
        }
        "loop_expression" | "for_expression" => Some("loop"),
        "macro_invocation" | "unsafe_block" => Some("unsupported"),
        _ => None,
    };
    if let Some(kind) = kind {
        let selected = if kind == "condition" {
            node.child_by_field_name("condition").unwrap_or(node)
        } else if kind == "call" {
            node.child_by_field_name("function").unwrap_or(node)
        } else if kind == "pattern" {
            node.child_by_field_name("pattern").unwrap_or(node)
        } else {
            node
        };
        let values = output.entry(kind.into()).or_default();
        if values.len() < 200 {
            values.push(digest(&[&semantic_tokens(selected, source)]));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        projection(child, source, output, depth + 1);
    }
}
fn calls(node: Node<'_>, source: &str, output: &mut Vec<String>, depth: usize) {
    if depth > 128 || output.len() >= 200 {
        return;
    }
    if node.kind() == "call_expression" {
        if let Some(function) = node.child_by_field_name("function") {
            output.push(compact(&node_text(source, function), 256));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        calls(child, source, output, depth + 1);
    }
}

pub fn generate_lld(
    repository: &Path,
    snapshot: &SnapshotId,
    scope: &ScopeRecord,
    context: &ConfirmedDesignContext,
    hld: &DesignVersion,
    cancellation: &Cancellation,
) -> Result<LldSnapshot, String> {
    context.validate(scope)?;
    crate::design::validate_design_scope(hld, scope)?;
    if hld.snapshot != *snapshot || hld.level != DesignLevel::Hld {
        return Err("LLD requires its exact authorized observed HLD parent snapshot".into());
    }
    let started = Instant::now();
    let mut catalog = LldCatalog {
        id: String::new(),
        generator_version: LLD_GENERATOR_VERSION.into(),
        parser_version: LLD_IR_VERSION.into(),
        profile: LLD_PROFILE.into(),
        repository: scope.primary_repository.clone(),
        snapshot: snapshot.clone(),
        scope_version: scope.version.clone(),
        scope_fingerprint: design_scope_fingerprint(scope),
        onboarding_session_id: context.session_id.clone(),
        onboarding_revision: context.session_revision,
        onboarding_input_fingerprint: context.input_fingerprint.clone(),
        onboarding_proposal_fingerprint: context.proposal_fingerprint.clone(),
        hld_design_id: hld.id.clone(),
        entries: vec![],
        evidence: vec![],
        coverage: LldCoverage::default(),
    };
    let mut inputs = vec![];
    let entries = match crate::design::generate::cancelable_tree(repository, snapshot, cancellation)
    {
        Ok(bytes) => {
            let (entries, unsupported) = crate::design::generate::parse_tree(&bytes, scope);
            catalog.coverage.unsupported += unsupported;
            if unsupported > 0 {
                catalog.coverage.structure_complete = false;
                catalog.coverage.gaps.push(
                    "Selected symlinks/submodules/nonregular entries are not followed".into(),
                );
            }
            entries
        }
        Err(error) => {
            if cancellation.is_cancelled() {
                return Err(error);
            }
            catalog.coverage.inventory_complete = false;
            catalog.coverage.failed += 1;
            catalog
                .coverage
                .gaps
                .push("Pinned tree enumeration unavailable or exceeds its bound".into());
            vec![]
        }
    };
    if entries.len() > MAX_PATHS {
        catalog.coverage.inventory_complete = false;
        catalog.coverage.deferred += entries.len() - MAX_PATHS;
        catalog
            .coverage
            .gaps
            .push("LLD inventory exceeded its 2000-path bound".into());
    }
    let mut remaining = MAX_LLD_INPUT_BYTES;
    let mut packages = BTreeMap::new();
    for chunk in entries
        .iter()
        .take(MAX_PATHS)
        .collect::<Vec<_>>()
        .chunks(32)
    {
        cancellation.checkpoint()?;
        if started.elapsed() > MAX_TIME || remaining == 0 {
            catalog.coverage.structure_complete = false;
            catalog.coverage.deferred += chunk.len();
            catalog.coverage.gaps.push("LLD ten-second/two-MiB immutable-input budget reached; unretained source is unavailable".into());
            break;
        }
        let selected = chunk
            .iter()
            .filter(|(path, _)| {
                path.ends_with(".rs")
                    || path.ends_with("Cargo.toml")
                    || path.ends_with("Cargo.lock")
                    || path.ends_with("vercel.json")
                    || [".sql", ".graphql", ".proto", ".prisma"]
                        .iter()
                        .any(|extension| path.ends_with(extension))
            })
            .collect::<Vec<_>>();
        let ids = selected
            .iter()
            .map(|(_, blob)| blob.clone())
            .collect::<Vec<_>>();
        let blobs = match read_chunk(
            repository,
            &ids,
            ReadLimits {
                objects_per_chunk: 32,
                bytes_per_object: MAX_BLOB,
                bytes_per_chunk: remaining,
            },
            cancellation,
        ) {
            Ok(blobs) => blobs,
            Err(error) => {
                if cancellation.is_cancelled() {
                    return Err(error);
                }
                catalog.coverage.structure_complete = false;
                catalog.coverage.failed += selected.len();
                catalog
                    .coverage
                    .gaps
                    .push("Selected source objects unavailable or bounded read exceeded".into());
                continue;
            }
        };
        for ((path, blob), result) in selected.into_iter().zip(blobs) {
            let BatchBlob::Data(bytes) = result else {
                catalog.coverage.failed += 1;
                catalog.coverage.structure_complete = false;
                catalog.coverage.gaps.push(format!(
                    "{path}: source missing, binary, or exceeds 512-KiB"
                ));
                continue;
            };
            remaining = remaining.saturating_sub(bytes.len());
            let Ok(source) = String::from_utf8(bytes) else {
                catalog.coverage.failed += 1;
                catalog.coverage.structure_complete = false;
                catalog
                    .coverage
                    .gaps
                    .push(format!("{path}: non-UTF-8 source unavailable"));
                continue;
            };
            if path.ends_with("Cargo.toml") {
                if let Ok(value) = toml::from_str::<toml::Value>(&source) {
                    if let Some(name) = value
                        .get("package")
                        .and_then(|item| item.get("name"))
                        .and_then(toml::Value::as_str)
                    {
                        packages.insert(
                            path.rsplit_once('/')
                                .map_or(".", |(parent, _)| parent)
                                .to_owned(),
                            compact(name, 128),
                        );
                    }
                }
            }
            let input = LldSyntaxInput {
                id: format!(
                    "lld-input-{}",
                    digest(&[
                        snapshot.as_str(),
                        scope.version.as_str(),
                        path,
                        blob,
                        LLD_IR_VERSION
                    ])
                ),
                version: LLD_IR_VERSION.into(),
                path: path.clone(),
                blob: blob.clone(),
                source_fingerprint: digest(&[&source]),
                source,
            };
            inputs.push(input);
        }
    }
    for input in &inputs {
        cancellation.checkpoint()?;
        if started.elapsed() > MAX_TIME {
            catalog.coverage.structure_complete = false;
            catalog.coverage.deferred += 1;
            catalog
                .coverage
                .gaps
                .push("LLD declaration extraction exceeded ten-second generation budget".into());
            continue;
        }
        if !input.path.ends_with(".rs") {
            if catalog.entries.len() >= MAX_LLD_ENTRIES {
                catalog.coverage.structure_complete = false;
                catalog.coverage.deferred += 1;
                continue;
            }
            let is_schema = [".sql", ".graphql", ".proto", ".prisma"]
                .iter()
                .any(|extension| input.path.ends_with(extension));
            let artifact_kind = if is_schema { "schema" } else { "configuration" };
            let parent = parent_binding(hld, &input.path);
            let id = format!(
                "lld-artifact-{}",
                digest(&[
                    snapshot.as_str(),
                    scope.version.as_str(),
                    &input.path,
                    &input.blob
                ])
            );
            let ev = format!("lld-evidence-{}", digest(&[&id, "schema"]));
            catalog.evidence.push(DesignEvidence {
                id: ev.clone(),
                repository: catalog.repository.clone(),
                commit: snapshot.clone(),
                path: input.path.clone(),
                blob: input.blob.clone(),
                start_line: 1,
                end_line: input.source.lines().count().max(1),
                producer_version: LLD_GENERATOR_VERSION.into(),
                relationship_type: format!("{artifact_kind}_artifact"),
                resolution: DesignResolution::ObservedStructure,
            });
            catalog.entries.push(LldEntry { id, logical_key: format!("{artifact_kind}|{}", input.path), name: input.path.rsplit('/').next().unwrap_or(&input.path).into(), kind: artifact_kind.into(), module: input.path.clone(), container: String::new(), parent_entry_id: None, child_entry_ids: vec![], path: input.path.clone(), blob: input.blob.clone(), start_byte: 0, end_byte: input.source.len(), start_line: 1, end_line: input.source.lines().count().max(1), visibility: "artifact".into(), signature: format!("Pinned {artifact_kind} artifact; runtime effect/compatibility unknown"), signature_fingerprint: input.source_fingerprint.clone(), body_fingerprint: input.source_fingerprint.clone(), projections: vec![], call_observations: vec![], detail_state: "structure_available".into(), evidence_ids: vec![ev], parent, limitations: vec![format!("{artifact_kind} artifact bytes are observed; runtime effects, compatibility and execution remain unknown")] });
            if is_schema {
                catalog.coverage.unsupported += 1;
                catalog.coverage.structure_complete = false;
            }
            continue;
        }
        let tree = match parse(&input.source, cancellation) {
            Ok(tree) => tree,
            Err(error) => {
                if cancellation.is_cancelled() {
                    return Err(error);
                }
                catalog.coverage.failed += 1;
                catalog.coverage.structure_complete = false;
                catalog.coverage.gaps.push(format!(
                    "{}: parser unavailable or bounded timeout",
                    input.path
                ));
                continue;
            }
        };
        let package = packages
            .iter()
            .filter(|(root, _)| under(&input.path, root))
            .max_by_key(|(root, _)| root.len())
            .map(|(_, name)| name.as_str())
            .unwrap_or("unresolved-package");
        // Keep file identity explicit; compiler module/path/cfg resolution is
        // unavailable. Inline module names extend this qualified syntax key.
        let module = format!("{package}::{}", input.path);
        let mut extractor = Extractor {
            catalog: &mut catalog,
            hld,
            path: &input.path,
            blob: &input.blob,
            source: &input.source,
            cancellation,
            started,
        };
        let parent = extractor.entry(
            tree.root_node(),
            &module,
            "",
            None,
            "file",
            input.path.clone(),
        );
        extractor.visit(tree.root_node(), &module, "", parent.as_deref(), 0)?;
        if tree.root_node().has_error() {
            catalog.coverage.structure_complete = false;
            catalog.coverage.failed += 1;
            catalog.coverage.gaps.push(format!(
                "{}: syntax recovery or missing AST nodes",
                input.path
            ));
        }
        catalog.coverage.indexed += 1;
    }
    let selected_source_extensions = [
        ".js", ".jsx", ".ts", ".tsx", ".py", ".go", ".java", ".c", ".cpp",
    ];
    for (path, _) in entries.iter().take(MAX_PATHS) {
        if selected_source_extensions
            .iter()
            .any(|extension| path.ends_with(extension))
        {
            catalog.coverage.unsupported += 1;
            catalog.coverage.structure_complete = false;
        }
    }
    if catalog.coverage.unsupported > 0 {
        catalog.coverage.gaps.push(
            "Non-Rust source/schema semantics are outside the supported local Rust profile".into(),
        );
    }
    if catalog.coverage.deferred > 0 {
        catalog
            .coverage
            .gaps
            .push("Bounded inventory/declaration/input omissions prevent complete result".into());
    }
    catalog.entries.sort_by(|a, b| {
        (&a.path, a.start_byte, &a.kind, &a.id).cmp(&(&b.path, b.start_byte, &b.kind, &b.id))
    });
    catalog.evidence.sort_by(|a, b| a.id.cmp(&b.id));
    catalog.evidence.dedup_by(|a, b| a.id == b.id);
    catalog.coverage.gaps.sort();
    catalog.coverage.gaps.dedup();
    catalog.coverage.gaps.truncate(64);
    // The private review contains two catalogs, their retained inputs and HLD
    // evidence. Bound each catalog independently; omit facts explicitly rather
    // than making an ordinary source review fail at the outer artifact limit.
    while serde_json::to_vec(&catalog)
        .map_err(|_| "Cannot encode bounded LLD catalog")?
        .len()
        > MAX_LLD_CATALOG_BYTES - 1024
    {
        if catalog.entries.is_empty() {
            return Err("LLD catalog metadata exceeds its one-MiB storage bound".into());
        }
        let remove = catalog.entries.len().min(25);
        catalog.entries.truncate(catalog.entries.len() - remove);
        catalog.coverage.deferred += remove;
        catalog.coverage.structure_complete = false;
        let gap = "LLD one-MiB catalog budget reached; excess declarations/evidence are explicitly deferred".to_string();
        if !catalog.coverage.gaps.contains(&gap) {
            catalog.coverage.gaps.push(gap);
        }
        let retained = catalog
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<BTreeSet<_>>();
        for entry in &mut catalog.entries {
            entry.child_entry_ids.retain(|id| retained.contains(id));
        }
        let evidence = catalog
            .entries
            .iter()
            .flat_map(|entry| entry.evidence_ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        catalog.evidence.retain(|item| evidence.contains(&item.id));
    }
    catalog.id = format!(
        "lld-catalog-{}",
        digest(&[
            LLD_GENERATOR_VERSION,
            LLD_IR_VERSION,
            LLD_PROFILE,
            snapshot.as_str(),
            scope.version.as_str(),
            &catalog.scope_fingerprint,
            &hld.id,
            &serde_json::to_string(&catalog.entries)
                .map_err(|_| "Cannot encode LLD declarations")?,
            &serde_json::to_string(&catalog.coverage).map_err(|_| "Cannot encode LLD coverage")?
        ])
    );
    let result = LldSnapshot { catalog, inputs };
    validate_snapshot(&result, scope)?;
    Ok(result)
}

fn validate_snapshot(snapshot: &LldSnapshot, scope: &ScopeRecord) -> Result<(), String> {
    let catalog = &snapshot.catalog;
    if catalog.repository != scope.primary_repository
        || catalog.scope_version != scope.version
        || catalog.scope_fingerprint != design_scope_fingerprint(scope)
        || catalog.entries.len() > MAX_LLD_ENTRIES
        || snapshot.inputs.len() > MAX_PATHS
        || snapshot
            .inputs
            .iter()
            .map(|input| input.source.len())
            .sum::<usize>()
            > MAX_LLD_INPUT_BYTES
        || serde_json::to_vec(catalog)
            .map_err(|_| "Cannot encode LLD catalog")?
            .len()
            > MAX_LLD_CATALOG_BYTES
        || catalog.profile != LLD_PROFILE
        || catalog.parser_version != LLD_IR_VERSION
        || catalog.generator_version != LLD_GENERATOR_VERSION
    {
        return Err("Invalid bounded immutable LLD snapshot identity".into());
    }
    for input in &snapshot.inputs {
        if !safe_path(&input.path, scope)
            || input.version != LLD_IR_VERSION
            || input.source.len() > MAX_BLOB
            || input.source_fingerprint != digest(&[&input.source])
            || SnapshotId::new(input.blob.clone()).is_err()
            || input.id
                != format!(
                    "lld-input-{}",
                    digest(&[
                        catalog.snapshot.as_str(),
                        scope.version.as_str(),
                        &input.path,
                        &input.blob,
                        LLD_IR_VERSION
                    ])
                )
        {
            return Err("LLD syntax input is not its authorized immutable source".into());
        }
    }
    for entry in &catalog.entries {
        let input = snapshot
            .inputs
            .iter()
            .find(|input| input.path == entry.path && input.blob == entry.blob)
            .ok_or("LLD entry input is unavailable")?;
        if entry.start_byte > entry.end_byte
            || entry.end_byte > input.source.len()
            || !input.source.is_char_boundary(entry.start_byte)
            || !input.source.is_char_boundary(entry.end_byte)
            || entry.start_line == 0
            || entry.end_line < entry.start_line
            || entry.end_line > input.source.lines().count().max(1)
            || entry.parent.hld_design_id != catalog.hld_design_id
            || entry.evidence_ids.is_empty()
            || entry.evidence_ids.iter().any(|id| {
                !catalog.evidence.iter().any(|evidence| {
                    evidence.id == *id
                        && evidence.path == entry.path
                        && evidence.blob == entry.blob
                        && evidence.commit == catalog.snapshot
                        && evidence.repository == catalog.repository
                })
            })
        {
            return Err("LLD declaration is outside its retained source evidence".into());
        }
    }
    let ids = catalog
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    let evidence_ids = catalog
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    let input_ids = snapshot
        .inputs
        .iter()
        .map(|input| input.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != catalog.entries.len()
        || evidence_ids.len() != catalog.evidence.len()
        || input_ids.len() != snapshot.inputs.len()
        || catalog.evidence.len() > MAX_LLD_ENTRIES * 2
        || catalog.coverage.gaps.len() > 64
    {
        return Err("Duplicate/unbounded LLD catalog identities".into());
    }
    for entry in &catalog.entries {
        if entry.id.len() > 128
            || entry.name.len() > 256
            || entry.signature.len() > 4096
            || entry.logical_key.len() > 4096
            || entry.child_entry_ids.len() > MAX_LLD_ENTRIES
            || entry
                .parent_entry_id
                .as_ref()
                .is_some_and(|id| !ids.contains(id.as_str()))
            || entry
                .child_entry_ids
                .iter()
                .any(|id| !ids.contains(id.as_str()))
            || entry.limitations.len() > 64
        {
            return Err("LLD declaration metadata or hierarchy is invalid".into());
        }
    }
    for evidence in &catalog.evidence {
        let input = snapshot
            .inputs
            .iter()
            .find(|input| input.path == evidence.path && input.blob == evidence.blob)
            .ok_or("LLD evidence source input unavailable")?;
        if evidence.repository != catalog.repository
            || evidence.commit != catalog.snapshot
            || evidence.start_line == 0
            || evidence.end_line < evidence.start_line
            || evidence.end_line > input.source.lines().count().max(1)
            || evidence.producer_version != LLD_GENERATOR_VERSION
            || evidence.id.len() > 128
        {
            return Err("LLD evidence is outside immutable catalog snapshot".into());
        }
    }
    let expected_id = format!(
        "lld-catalog-{}",
        digest(&[
            LLD_GENERATOR_VERSION,
            LLD_IR_VERSION,
            LLD_PROFILE,
            catalog.snapshot.as_str(),
            scope.version.as_str(),
            &catalog.scope_fingerprint,
            &catalog.hld_design_id,
            &serde_json::to_string(&catalog.entries)
                .map_err(|_| "Cannot encode LLD declarations")?,
            &serde_json::to_string(&catalog.coverage).map_err(|_| "Cannot encode LLD coverage")?
        ])
    );
    if catalog.id != expected_id {
        return Err("LLD catalog immutable projection checksum mismatch".into());
    }
    Ok(())
}

pub fn build_lld_detail(
    snapshot: &LldSnapshot,
    entry_id: &str,
    cancellation: &Cancellation,
) -> Result<LldFunctionDetail, String> {
    cancellation.checkpoint()?;
    let entry = snapshot
        .catalog
        .entries
        .iter()
        .find(|entry| entry.id == entry_id)
        .ok_or("LLD selected entry unavailable")?
        .clone();
    let input = snapshot
        .inputs
        .iter()
        .find(|input| input.path == entry.path && input.blob == entry.blob)
        .ok_or("LLD exact immutable syntax input unavailable")?;
    if input.source_fingerprint != digest(&[&input.source]) || input.version != LLD_IR_VERSION {
        return Err("LLD immutable syntax checksum/version mismatch".into());
    }
    super::blob::verify(&input.blob, &input.source, cancellation)?;
    let tree = if entry.kind == "function" {
        Some(parse(&input.source, cancellation)?)
    } else {
        None
    };
    fn find<'a>(node: Node<'a>, entry: &LldEntry) -> Option<Node<'a>> {
        let mut pending = vec![(node, 0)];
        while let Some((node, depth)) = pending.pop() {
            if depth > 128 {
                return None;
            }
            if node.start_byte() == entry.start_byte
                && node.end_byte() == entry.end_byte
                && matches!(node.kind(), "function_item" | "function_signature_item")
            {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.start_byte() <= entry.start_byte && child.end_byte() >= entry.end_byte {
                    pending.push((child, depth + 1));
                }
            }
        }
        None
    }
    let function = tree
        .as_ref()
        .and_then(|tree| find(tree.root_node(), &entry));
    let mut coverage = snapshot.catalog.coverage.clone();
    let graph = if entry.kind == "function"
        && function.is_some_and(|function| function.child_by_field_name("body").is_some())
    {
        super::flow::lower_flow(
            &input.source,
            function.unwrap(),
            &entry,
            &snapshot.catalog.snapshot,
            &snapshot.catalog.repository,
            cancellation,
        )?
    } else {
        LldFlow { nodes: vec![], edges: vec![], regions: vec![], complete: false, limitations: vec!["Selected declaration has structural detail only; a function body is required for local control flow".into()], omitted_nodes: 0, omitted_edges: 0, fingerprint: entry.signature_fingerprint.clone() }
    };
    coverage.local_flow = if entry.kind == "function" {
        if graph.complete {
            "complete_supported_profile"
        } else {
            "incomplete"
        }
    } else {
        "not_applicable_structure"
    }
    .into();
    coverage.gaps.extend(graph.limitations.iter().cloned());
    coverage.gaps.extend(entry.limitations.iter().cloned());
    coverage.gaps.sort();
    coverage.gaps.dedup();
    coverage.gaps.truncate(64);
    let mut evidence = snapshot
        .catalog
        .evidence
        .iter()
        .filter(|evidence| entry.evidence_ids.contains(&evidence.id))
        .cloned()
        .collect::<Vec<_>>();
    for node in &graph.nodes {
        evidence.push(DesignEvidence {
            id: node.evidence_id.clone(),
            repository: snapshot.catalog.repository.clone(),
            commit: snapshot.catalog.snapshot.clone(),
            path: entry.path.clone(),
            blob: entry.blob.clone(),
            start_line: node.start_line.max(1),
            end_line: node.end_line.max(node.start_line).max(1),
            producer_version: LLD_GENERATOR_VERSION.into(),
            relationship_type: "lld_syntax_span".into(),
            resolution: DesignResolution::ObservedStructure,
        });
    }
    evidence.sort_by(|a, b| a.id.cmp(&b.id));
    evidence.dedup_by(|a, b| a.id == b.id);
    let behavior_fingerprint = digest(&[
        LLD_PROFILE,
        &entry.signature_fingerprint,
        &entry.body_fingerprint,
        &graph.fingerprint,
    ]);
    let id = format!(
        "lld-detail-{}",
        digest(&[&snapshot.catalog.id, &entry.id, &behavior_fingerprint])
    );
    let all_declarations = snapshot
        .catalog
        .entries
        .iter()
        .filter(|item| entry.child_entry_ids.contains(&item.id))
        .cloned()
        .collect::<Vec<_>>();
    let mut declarations = all_declarations
        .iter()
        .take(100)
        .cloned()
        .collect::<Vec<_>>();
    while serde_json::to_vec(&declarations)
        .map_err(|_| "Cannot encode LLD children")?
        .len()
        > 128 * 1024
    {
        declarations.pop();
    }
    if declarations.len() < all_declarations.len() {
        coverage.deferred += all_declarations.len() - declarations.len();
        coverage.structure_complete = false;
        coverage.gaps.push(format!("{} direct child declarations omitted by the 100-entry/128-KiB detail bound; catalog pagination retains available inventory",all_declarations.len()-declarations.len()));
    }
    let mut detail = LldFunctionDetail {
        id,
        catalog_id: snapshot.catalog.id.clone(),
        declarations,
        entry,
        profile: LLD_PROFILE.into(),
        label: if function.is_some() {
            LLD_LABEL
        } else {
            "Static declaration structure"
        }
        .into(),
        graph,
        evidence,
        behavior_fingerprint,
        coverage,
        mermaid: String::new(),
        markdown: String::new(),
    };
    detail.mermaid = render_lld_mermaid(&detail);
    detail.markdown = render_lld_markdown(&detail);
    if serde_json::to_vec(&detail.graph)
        .map_err(|_| "Cannot encode LLD graph")?
        .len()
        > 128 * 1024
    {
        return Err("LLD graph exceeds its 128-KiB detail budget".into());
    }
    Ok(detail)
}

pub fn lld_source(snapshot: &LldSnapshot, evidence: &DesignEvidence) -> Result<String, String> {
    if evidence.repository != snapshot.catalog.repository
        || evidence.commit != snapshot.catalog.snapshot
    {
        return Err("LLD source evidence identity mismatch".into());
    }
    let input = snapshot
        .inputs
        .iter()
        .find(|input| input.path == evidence.path && input.blob == evidence.blob)
        .ok_or("LLD exact source unavailable")?;
    if input.source_fingerprint != digest(&[&input.source])
        || evidence.start_line == 0
        || evidence.end_line < evidence.start_line
    {
        return Err("LLD source checksum/span invalid".into());
    }
    super::blob::verify(&input.blob, &input.source, &Cancellation::default())?;
    let source = input
        .source
        .lines()
        .skip(evidence.start_line - 1)
        .take((evidence.end_line - evidence.start_line + 1).min(200))
        .collect::<Vec<_>>()
        .join("\n");
    if source.len() > 64 * 1024 {
        return Err("LLD source exceeds its 64-KiB preview bound".into());
    }
    Ok(source)
}
pub fn lld_detail_design(
    snapshot: &LldSnapshot,
    detail: &LldFunctionDetail,
) -> Result<DesignVersion, String> {
    if detail.catalog_id != snapshot.catalog.id
        || !snapshot
            .catalog
            .entries
            .iter()
            .any(|entry| entry == &detail.entry)
    {
        return Err("LLD detail does not belong to selected immutable catalog".into());
    }
    let catalog = &snapshot.catalog;
    Ok(DesignVersion {
        level: DesignLevel::Lld,
        lld: Some(LldDesignData {
            catalog_id: catalog.id.clone(),
            entry_id: detail.entry.id.clone(),
            logical_key: detail.entry.logical_key.clone(),
            behavior_fingerprint: detail.behavior_fingerprint.clone(),
            parent: detail.entry.parent.clone(),
            detail: detail.clone(),
            invariants: vec![],
        }),
        id: detail.id.clone(),
        generator_version: LLD_GENERATOR_VERSION.into(),
        repository: catalog.repository.clone(),
        snapshot: catalog.snapshot.clone(),
        scope_version: catalog.scope_version.clone(),
        scope_fingerprint: catalog.scope_fingerprint.clone(),
        onboarding_session_id: catalog.onboarding_session_id.clone(),
        onboarding_revision: catalog.onboarding_revision,
        onboarding_input_fingerprint: catalog.onboarding_input_fingerprint.clone(),
        onboarding_proposal_fingerprint: catalog.onboarding_proposal_fingerprint.clone(),
        review_state: if detail.graph.complete && catalog.coverage.structure_complete {
            DesignReviewState::AwaitingConfirmation
        } else {
            DesignReviewState::Partial
        },
        view: DesignView::Observed,
        elements: vec![],
        relations: vec![],
        evidence: detail.evidence.clone(),
        sections: vec![],
        coverage: DesignCoverage {
            profile: LLD_PROFILE.into(),
            enumeration_complete: catalog.coverage.inventory_complete,
            indexed: detail.graph.nodes.len(),
            unsupported: catalog.coverage.unsupported,
            failed: catalog.coverage.failed,
            deferred: catalog.coverage.deferred + detail.graph.omitted_nodes,
            complete: detail.graph.complete && catalog.coverage.structure_complete,
            gaps: detail.coverage.gaps.clone(),
        },
        constraints: vec![],
        correspondence: vec![],
        document_candidates: vec![],
        decision_records: vec![],
        mermaid: detail.mermaid.clone(),
        markdown: detail.markdown.clone(),
    })
}

/// Validate private retained IR against the already authorized immutable HLD
/// parent and current intersected scope before any catalog/detail/source work.
pub fn validate_lld_snapshot(
    snapshot: &LldSnapshot,
    hld: &DesignVersion,
    scope: &ScopeRecord,
) -> Result<(), String> {
    crate::design::validate_design_scope(hld, scope)?;
    if hld.level != DesignLevel::Hld
        || hld.snapshot != snapshot.catalog.snapshot
        || hld.id != snapshot.catalog.hld_design_id
        || hld.onboarding_session_id != snapshot.catalog.onboarding_session_id
        || hld.onboarding_revision != snapshot.catalog.onboarding_revision
        || hld.onboarding_input_fingerprint != snapshot.catalog.onboarding_input_fingerprint
        || hld.onboarding_proposal_fingerprint != snapshot.catalog.onboarding_proposal_fingerprint
    {
        return Err("LLD parent snapshot identity mismatch".into());
    }
    validate_snapshot(snapshot, scope)?;
    for entry in &snapshot.catalog.entries {
        if entry.parent != parent_binding(hld, &entry.path) {
            return Err("LLD parent containment differs from exact authorized HLD".into());
        }
    }
    Ok(())
}
