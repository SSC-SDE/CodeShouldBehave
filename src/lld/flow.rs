//! Bounded lowering of Rust syntax, with no compiler or runtime assumptions.
//!
//! Every edge is derived from AST ownership. Unsupported regions terminate at
//! an explicit unknown frontier; they do not gain an invented fallthrough.
use super::{LldBehaviorEdge, LldBehaviorNode, LldEntry, LldFlow, LldFlowRegion, semantic_tokens};
use crate::{RepositoryId, SnapshotId, core::Cancellation};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tree_sitter::Node;

pub(crate) const MAX_FLOW_NODES: usize = 200;
pub(crate) const MAX_FLOW_EDGES: usize = 400;
const MAX_FLOW_DEPTH: usize = 32;
const MAX_FLOW_BYTES: usize = 128 * 1024;
// Reserve region identities, bounded limitations, coverage counters, and the
// semantic fingerprint. Up to 49 nested regions fit the 200-node bound.
const FLOW_PAYLOAD_BUDGET: usize = MAX_FLOW_BYTES - 32 * 1024;
const MAX_NODE_TOKENS: usize = 4096;

/// A lightweight, conservative inventory for the critical analysis queue.
/// Lazy detail may refine these gaps, but an unlowered function with an
/// unsupported region must never make a catalog comparison falsely clean.
pub(crate) fn local_syntax_limitations(function: Node<'_>, source: &str) -> Vec<String> {
    let mut reasons = std::collections::BTreeSet::new();
    if function.has_error() {
        reasons.insert("Rust parse recovery prevents complete local flow".to_owned());
    }
    let mut ancestor = Some(function);
    while let Some(node) = ancestor {
        if transforming_attribute(source, node).is_some() {
            reasons
                .insert("Conditional compilation or attribute expansion is unresolved".to_owned());
        }
        ancestor = node.parent();
    }
    let Some(body) = function.child_by_field_name("body") else {
        reasons.insert("Function body is unavailable for local flow".into());
        return reasons.into_iter().collect();
    };
    let mut todo = vec![(body, 0)];
    let mut visited = 0;
    let mut estimated_nodes = 3;
    let started = Instant::now();
    while let Some((node, depth)) = todo.pop() {
        visited += 1;
        if visited > 4096 || started.elapsed() >= Duration::from_secs(2) {
            reasons.insert("Local syntax inventory reached its bounded traversal limit".into());
            break;
        }
        if depth > MAX_FLOW_DEPTH / 3 {
            // Lowering counts expression, condition, and structural dispatch
            // frames separately. The scanner cannot safely equate AST depth
            // with lowering depth, so it warns at a conservative upper bound.
            reasons.insert(
                "Local syntax may exceed the depth-32 lowering profile; detail is required".into(),
            );
            continue;
        }
        let kind = node.kind();
        if matches!(kind, "line_comment" | "block_comment") {
            continue;
        }
        if matches!(kind, "attribute_item" | "inner_attribute_item") {
            if !safe_attribute(source.get(node.byte_range()).unwrap_or("")) {
                reasons
                    .insert("Conditional compilation or attribute expansion is unresolved".into());
            }
            continue;
        }
        if matches!(
            kind,
            "compound_assignment_expr"
                | "unsafe_block"
                | "try_block"
                | "const_block"
                | "gen_block"
                | "macro_invocation"
                | "macro_definition"
                | "yield_expression"
                | "const_item"
                | "static_item"
                | "foreign_mod_item"
                | "mod_item"
        ) {
            reasons.insert(format!(
                "Unsupported {kind}: local execution/exit semantics are unresolved"
            ));
            continue;
        }
        if kind.ends_with("_expression") && !supported_expression_kind(kind) {
            reasons.insert(format!(
                "Unsupported AST construct {kind}: local continuation is unknown"
            ));
            continue;
        }
        if kind == "block"
            && named_children(node)
                .iter()
                .any(|child| child.kind() == "label")
        {
            reasons
                .insert("Labeled block exits are outside the supported loop-target profile".into());
        }
        if kind == "match_expression" {
            reasons.insert(
                "Match pattern exhaustiveness, overlap, and type feasibility are unverified".into(),
            );
        }
        if kind == "array_expression" && node.child_by_field_name("length").is_some() {
            reasons.insert("Repeat-array length and const operand semantics are outside the runtime flow profile".into());
        }
        if matches!(kind, "break_expression" | "continue_expression")
            && !has_loop_target(node, source)
        {
            reasons.insert(
                "break/continue has no resolved loop target in this executable region".into(),
            );
        }
        if kind == "let_declaration" {
            if let Some(alternative) = node.child_by_field_name("alternative") {
                if !syntactically_diverges(alternative, 0) {
                    reasons.insert(
                        "let-else fallback divergence is unverified before local detail".into(),
                    );
                }
            }
        }
        if let Some(pattern) = node.child_by_field_name("pattern") {
            if unsupported_pattern(pattern) {
                reasons.insert(
                    "Pattern expansion, const evaluation, or recovery is unresolved".into(),
                );
            }
        }
        // This is an upper-bound warning, not an invented graph. Fine-grained
        // lowering will provide actual omitted node/edge counts when requested.
        if kind.ends_with("_expression")
            || matches!(
                kind,
                "identifier"
                    | "self"
                    | "integer_literal"
                    | "float_literal"
                    | "boolean_literal"
                    | "char_literal"
                    | "string_literal"
                    | "raw_string_literal"
                    | "let_declaration"
            )
        {
            estimated_nodes += if matches!(
                kind,
                "if_expression"
                    | "while_expression"
                    | "for_expression"
                    | "loop_expression"
                    | "await_expression"
                    | "closure_expression"
            ) {
                4
            } else {
                1
            };
        }
        if estimated_nodes > MAX_FLOW_NODES {
            reasons.insert(
                "Local syntax may exceed the 200-node detail budget; lowering is required".into(),
            );
        }
        if matches!(
            kind,
            "string_literal" | "raw_string_literal" | "char_literal"
        ) && node.end_byte() - node.start_byte() > MAX_NODE_TOKENS
        {
            reasons
                .insert("A literal exceeds the per-operation semantic-token detail budget".into());
        }
        if kind == "identifier" && node.end_byte() - node.start_byte() + 10 > MAX_NODE_TOKENS {
            reasons.insert("An identifier exceeds the semantic-token detail budget".into());
        }
        if kind == "closure_expression"
            && node
                .child_by_field_name("body")
                .is_some_and(|body| body.kind() == "_")
        {
            reasons.insert(
                "Uncompiled placeholder closure body has unknown execution semantics".into(),
            );
        }
        // Patterns and type syntax are inventory, not independently executed
        // child expressions. Their expansion is separately checked above.
        todo.extend(
            named_children(node)
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1)),
        );
    }
    reasons.into_iter().take(64).collect()
}

fn supported_expression_kind(kind: &str) -> bool {
    matches!(
        kind,
        "parenthesized_expression"
            | "if_expression"
            | "match_expression"
            | "while_expression"
            | "for_expression"
            | "loop_expression"
            | "return_expression"
            | "break_expression"
            | "continue_expression"
            | "try_expression"
            | "await_expression"
            | "call_expression"
            | "binary_expression"
            | "assignment_expression"
            | "closure_expression"
            | "unit_expression"
            | "unary_expression"
            | "reference_expression"
            | "type_cast_expression"
            | "field_expression"
            | "index_expression"
            | "array_expression"
            | "tuple_expression"
            | "range_expression"
            | "struct_expression"
    )
}
fn syntactically_diverges(node: Node<'_>, depth: usize) -> bool {
    if depth > MAX_FLOW_DEPTH {
        return false;
    }
    match node.kind() {
        "return_expression" | "break_expression" | "continue_expression" => true,
        "expression_statement" | "parenthesized_expression" | "else_clause" => {
            first_executable_child(node)
                .is_some_and(|child| syntactically_diverges(child, depth + 1))
        }
        "block" => named_children(node)
            .into_iter()
            .any(|child| syntactically_diverges(child, depth + 1)),
        "if_expression" => {
            node.child_by_field_name("consequence")
                .is_some_and(|body| syntactically_diverges(body, depth + 1))
                && node
                    .child_by_field_name("alternative")
                    .is_some_and(|body| syntactically_diverges(body, depth + 1))
        }
        _ => false,
    }
}
fn has_loop_target(node: Node<'_>, source: &str) -> bool {
    let label = named_children(node)
        .into_iter()
        .find(|node| node.kind() == "label")
        .and_then(|node| source.get(node.byte_range()));
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if matches!(
            parent.kind(),
            "closure_expression" | "async_block" | "function_item"
        ) {
            return false;
        }
        if matches!(
            parent.kind(),
            "loop_expression" | "while_expression" | "for_expression"
        ) {
            let parent_label = named_children(parent)
                .into_iter()
                .find(|node| node.kind() == "label")
                .and_then(|node| source.get(node.byte_range()));
            if label.is_none() || label == parent_label {
                return true;
            }
        }
        ancestor = parent.parent();
    }
    false
}

#[derive(Clone)]
struct Port {
    from: String,
    kind: String,
    label: String,
    evidence: Option<String>,
}
impl Port {
    fn next(from: String) -> Self {
        Self {
            from,
            kind: "next".into(),
            label: String::new(),
            evidence: None,
        }
    }
    fn branch(from: String, kind: &str, label: &str, evidence: String) -> Self {
        Self {
            from,
            kind: kind.into(),
            label: label.into(),
            evidence: Some(evidence),
        }
    }
}

#[derive(Default)]
struct Branches {
    yes: Vec<Port>,
    no: Vec<Port>,
}
#[derive(Clone)]
struct Region {
    id: String,
    exit: String,
    unknown: String,
}
struct LoopTarget {
    label: Option<String>,
    header: String,
    exit: String,
    breaks: usize,
}

struct Lower<'a> {
    source: &'a str,
    occurrence: &'a str,
    cancellation: &'a Cancellation,
    started: Instant,
    graph: LldFlow,
    region: Region,
    loops: Vec<LoopTarget>,
    payload_bytes: usize,
    stopped: bool,
}

/// The complete immutable blob is supplied by the private IR handoff. This
/// routine never reads a worktree, expands a macro, compiles, or runs the code.
pub(crate) fn lower_flow(
    source: &str,
    function: Node<'_>,
    entry: &LldEntry,
    _snapshot: &SnapshotId,
    _repository: &RepositoryId,
    cancellation: &Cancellation,
) -> Result<LldFlow, String> {
    cancellation
        .checkpoint()
        .map_err(|_| "LLD flow lowering cancelled".to_owned())?;
    if source.len() > 512 * 1024 {
        return Err("LLD flow input exceeds the 512 KiB immutable-blob limit".into());
    }
    if function.start_byte() != entry.start_byte || function.end_byte() != entry.end_byte {
        return Err("LLD function AST does not match its immutable entry span".into());
    }
    let root_region = id("region", &format!("{}:root", entry.id));
    let mut lower = Lower {
        source,
        occurrence: &entry.id,
        cancellation,
        started: Instant::now(),
        graph: LldFlow {
            nodes: vec![],
            edges: vec![],
            regions: vec![],
            complete: true,
            limitations: vec![],
            omitted_nodes: 0,
            omitted_edges: 0,
            fingerprint: String::new(),
        },
        region: Region {
            id: root_region.clone(),
            exit: String::new(),
            unknown: String::new(),
        },
        loops: vec![],
        payload_bytes: 0,
        stopped: false,
    };
    let entry_node = lower.synthetic(function, "entry", "Function entry");
    let exit = lower.synthetic(function, "exit", "Function result / return");
    let unknown = lower.synthetic(function, "unknown_exit", "Unknown control frontier");
    lower.region.exit = exit.clone();
    lower.region.unknown = unknown;
    lower.graph.regions.push(LldFlowRegion {
        id: root_region,
        parent_node: None,
        kind: "function".into(),
        entry: entry_node.clone(),
        exit,
    });
    let initial = vec![Port::next(entry_node)];
    if function.has_error() {
        lower.unknown(
            function,
            initial,
            "Rust parse recovery in the selected function",
        );
    } else if let Some(attribute) = inherited_transforming_attribute(source, function) {
        lower.unknown(
            attribute,
            initial,
            "Conditional compilation or attribute expansion is unresolved",
        );
    } else if let Some(body) = function.child_by_field_name("body") {
        let pending = lower.expr(body, initial, 0)?;
        if !pending.is_empty() {
            let tail = lower.synthetic(body, "tail_result", "Implicit function tail result");
            lower.attach(pending, &tail);
            let exit = lower.region.exit.clone();
            lower.edge(&tail, &exit, "tail_result", "tail result", None);
        }
    } else {
        lower.unknown(
            function,
            initial,
            "Selected function has no retained executable body",
        );
    }
    cancellation
        .checkpoint()
        .map_err(|_| "LLD flow lowering cancelled".to_owned())?;
    // Unknown targets are reserved while lowering so an exhausted budget can
    // always end at a real node. Unused reservations are bookkeeping, not an
    // observed unknown branch, and should not create a misleading diagram box.
    let unknown_targets = lower
        .graph
        .edges
        .iter()
        .map(|edge| edge.to.clone())
        .collect::<std::collections::BTreeSet<_>>();
    lower
        .graph
        .nodes
        .retain(|node| node.operation != "unknown_exit" || unknown_targets.contains(&node.id));
    lower.graph.limitations.sort();
    lower.graph.limitations.dedup();
    lower.graph.fingerprint = graph_fingerprint(&lower.graph);
    // The running budget reserves metadata overhead. A final check protects
    // this invariant even if a future record field grows without updating it.
    if serde_json::to_vec(&lower.graph)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_FLOW_BYTES
    {
        return Err("LLD graph exceeds the 128 KiB serialized-detail limit".into());
    }
    Ok(lower.graph)
}

impl<'a> Lower<'a> {
    fn notice(&mut self, reason: &str) {
        if !self.graph.limitations.iter().any(|value| value == reason) {
            self.graph.limitations.push(reason.into());
        }
    }
    fn gap(&mut self, reason: &str) {
        self.graph.complete = false;
        self.notice(reason);
    }
    fn checkpoint(&mut self, depth: usize) -> Result<bool, String> {
        self.cancellation
            .checkpoint()
            .map_err(|_| "LLD flow lowering cancelled".to_owned())?;
        if depth > MAX_FLOW_DEPTH {
            self.gap("AST lowering exceeded depth 32; omitted counts are lower bounds");
            return Ok(false);
        }
        if self.started.elapsed() >= Duration::from_secs(2) {
            self.stopped = true;
            self.gap("AST lowering reached its two-second limit; omitted counts are lower bounds");
        }
        Ok(!self.stopped)
    }
    fn text(&self, node: Node<'_>) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }
    fn synthetic(&mut self, syntax: Node<'_>, operation: &str, label: &str) -> String {
        self.add_node(syntax, operation, "", label, "syntactic")
    }
    fn operation(&mut self, syntax: Node<'_>, operation: &str, label: &str) -> String {
        let tokens = semantic_tokens(syntax, self.source);
        self.add_node(syntax, operation, &tokens, label, "syntactic")
    }
    fn add_node(
        &mut self,
        syntax: Node<'_>,
        operation: &str,
        tokens: &str,
        label: &str,
        resolution: &str,
    ) -> String {
        if self.stopped {
            self.graph.omitted_nodes += 1;
            return self.region.unknown.clone();
        }
        if self.graph.nodes.len() >= MAX_FLOW_NODES {
            self.graph.omitted_nodes += 1;
            self.stopped = true;
            self.gap("Local graph reached its 200-node bound; omitted counts are lower bounds");
            return self.region.unknown.clone();
        }
        if tokens.len() > MAX_NODE_TOKENS {
            self.graph.omitted_nodes += 1;
            self.gap("An operation's semantic tokens exceed 4096 bytes; the region is unavailable");
            return self.region.unknown.clone();
        }
        let occurrence = format!(
            "{}:{}:{}:{}:{}:{}",
            self.occurrence,
            self.region.id,
            syntax.start_byte(),
            syntax.end_byte(),
            operation,
            self.graph.nodes.len()
        );
        let node_id = id("node", &occurrence);
        let evidence_id = id(
            "evidence",
            &format!(
                "{}:{}:{}",
                self.occurrence,
                syntax.start_byte(),
                syntax.end_byte()
            ),
        );
        let node = LldBehaviorNode {
            id: node_id.clone(),
            region_id: self.region.id.clone(),
            ast_kind: syntax.kind().into(),
            operation: operation.into(),
            tokens: tokens.into(),
            label: bounded_label(label),
            start_byte: syntax.start_byte(),
            end_byte: syntax.end_byte(),
            start_line: syntax.start_position().row + 1,
            end_line: syntax.end_position().row + usize::from(syntax.end_position().column > 0),
            evidence_id,
            resolution: resolution.into(),
        };
        let bytes = serde_json::to_vec(&node).map_or(FLOW_PAYLOAD_BUDGET, |value| value.len());
        if self.payload_bytes + bytes > FLOW_PAYLOAD_BUDGET {
            self.graph.omitted_nodes += 1;
            self.stopped = true;
            self.gap(
                "Local graph reached its 128 KiB payload bound; omitted counts are lower bounds",
            );
            return self.region.unknown.clone();
        }
        self.payload_bytes += bytes;
        self.graph.nodes.push(node);
        node_id
    }
    fn node_evidence(&self, node: &str) -> String {
        self.graph
            .nodes
            .iter()
            .find(|value| value.id == node)
            .map(|value| value.evidence_id.clone())
            .unwrap_or_default()
    }
    fn edge(&mut self, from: &str, to: &str, kind: &str, label: &str, evidence: Option<String>) {
        if from.is_empty() || to.is_empty() || from == self.region.unknown {
            // Unknown frontiers cannot silently rejoin the normal control path.
            return;
        }
        if self.graph.edges.len() >= MAX_FLOW_EDGES {
            self.graph.omitted_edges += 1;
            self.stopped = true;
            self.gap("Local graph reached its 400-edge bound; omitted counts are lower bounds");
            return;
        }
        let edge = LldBehaviorEdge {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
            label: bounded_label(label),
            evidence_id: evidence,
        };
        let bytes = serde_json::to_vec(&edge).map_or(FLOW_PAYLOAD_BUDGET, |value| value.len());
        if self.payload_bytes + bytes > FLOW_PAYLOAD_BUDGET {
            self.graph.omitted_edges += 1;
            self.stopped = true;
            self.gap(
                "Local graph reached its 128 KiB payload bound; omitted counts are lower bounds",
            );
            return;
        }
        self.payload_bytes += bytes;
        self.graph.edges.push(edge);
    }
    fn attach(&mut self, incoming: Vec<Port>, target: &str) {
        for port in incoming {
            self.edge(&port.from, target, &port.kind, &port.label, port.evidence);
        }
    }
    fn action(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        operation: &str,
        label: &str,
    ) -> Vec<Port> {
        if incoming.is_empty() {
            return vec![];
        }
        let node = self.operation(syntax, operation, label);
        self.attach(incoming, &node);
        if node == self.region.unknown {
            vec![]
        } else {
            vec![Port::next(node)]
        }
    }
    fn unknown(&mut self, syntax: Node<'_>, incoming: Vec<Port>, reason: &str) -> Vec<Port> {
        self.gap(reason);
        let tokens = semantic_tokens(syntax, self.source);
        let tokens = if tokens.len() > MAX_NODE_TOKENS {
            ""
        } else {
            &tokens
        };
        let node = self.add_node(syntax, "unsupported_region", tokens, reason, "unknown");
        self.attach(incoming, &node);
        let target = self.region.unknown.clone();
        self.edge(
            &node,
            &target,
            "unknown_exit",
            "control continuation unknown",
            Some(self.node_evidence(&node)),
        );
        vec![]
    }

    fn expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        if incoming.is_empty() {
            // No synthetic continuation after an unconditional exit. The
            // catalog still fingerprints the complete (including dead) body.
            return Ok(vec![]);
        }
        if !self.checkpoint(depth)? {
            return Ok(self.unknown(
                syntax,
                incoming,
                "Bounded traversal omitted this AST region",
            ));
        }
        if syntax.is_error() || syntax.is_missing() {
            return Ok(self.unknown(
                syntax,
                incoming,
                "Rust parse recovery is not a supported flow region",
            ));
        }
        match syntax.kind() {
            "block" => self.block(syntax, incoming, depth + 1),
            "expression_statement" | "parenthesized_expression" => {
                match first_executable_child(syntax) {
                    Some(value) => self.expr(value, incoming, depth + 1),
                    None => Ok(self.unknown(syntax, incoming, "Expression wrapper has no executable child")),
                }
            }
            "let_declaration" => self.let_binding(syntax, incoming, depth + 1),
            "if_expression" => self.if_expr(syntax, incoming, depth + 1),
            "match_expression" => self.match_expr(syntax, incoming, depth + 1),
            "while_expression" | "for_expression" | "loop_expression" => self.loop_expr(syntax, incoming, depth + 1),
            "return_expression" => self.return_expr(syntax, incoming, depth + 1),
            "break_expression" | "continue_expression" => self.loop_exit(syntax, incoming, depth + 1),
            "try_expression" => self.try_expr(syntax, incoming, depth + 1),
            "await_expression" => self.await_expr(syntax, incoming, depth + 1),
            "call_expression" => self.call_expr(syntax, incoming, depth + 1),
            "generic_function" => {
                if let Some(function) = syntax.child_by_field_name("function") {
                    let pending = self.expr(function, incoming, depth + 1)?;
                    Ok(self.action(syntax, pending, "generic_callee", self.text(syntax)))
                } else {
                    Ok(self.unknown(syntax, incoming, "Generic call syntax has no retained receiver/callee"))
                }
            }
            "array_expression" if syntax.child_by_field_name("length").is_some() => {
                Ok(self.unknown(syntax, incoming, "Repeat-array length and const operand semantics are outside the runtime flow profile"))
            }
            "binary_expression" if matches!(operator(self.source, syntax), "&&" | "||") => {
                let branches = self.condition(syntax, incoming, depth + 1)?;
                let join = self.synthetic(syntax, "short_circuit_value", "Boolean short-circuit result");
                self.attach(branches.yes, &join);
                self.attach(branches.no, &join);
                Ok(vec![Port::next(join)])
            }
            "assignment_expression" => {
                // Rust evaluates the RHS before evaluating the assignee.
                let mut pending = incoming;
                for field in ["right", "left"] {
                    if let Some(value) = syntax.child_by_field_name(field) {
                        pending = self.expr(value, pending, depth + 1)?;
                    }
                }
                Ok(self.action(syntax, pending, "assignment", self.text(syntax)))
            }
            "compound_assignment_expr" => {
                // Primitive and overloaded compound assignment can have
                // different evaluation order. Without type analysis, do not
                // invent an ordering of side effects in its operands.
                Ok(self.unknown(syntax, incoming, "Compound-assignment operand ordering requires type/dispatch analysis"))
            }
            "closure_expression" | "async_block" | "function_item" => self.nested_region(syntax, incoming, depth + 1),
            "unsafe_block" | "try_block" | "const_block" | "gen_block" | "macro_invocation" | "macro_definition" | "yield_expression" => {
                let reason = format!("Unsupported {}: expansion, execution, or exit behavior is unknown", syntax.kind());
                Ok(self.unknown(syntax, incoming, &reason))
            }
            "empty_statement" | "line_comment" | "block_comment" => Ok(incoming),
            "identifier" | "self" | "super" | "crate" | "scoped_identifier" | "scoped_type_identifier" |
            "integer_literal" | "float_literal" | "boolean_literal" | "char_literal" | "string_literal" | "raw_string_literal" | "unit_expression" => {
                Ok(self.action(syntax, incoming, "value", self.text(syntax)))
            }
            "binary_expression" | "unary_expression" | "reference_expression" | "type_cast_expression" | "field_expression" | "index_expression" |
            "array_expression" | "tuple_expression" | "range_expression" | "struct_expression" => self.value_expr(syntax, incoming, depth + 1),
            "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item" | "impl_item" | "use_declaration" | "extern_crate_declaration" => {
                // These are declarations, not function-local runtime operations.
                Ok(self.action(syntax, incoming, "declaration", self.text(syntax)))
            }
            "const_item" | "static_item" | "foreign_mod_item" | "mod_item" => {
                Ok(self.unknown(syntax, incoming, "Nested compile-time/storage/external declaration is outside the local flow profile"))
            }
            _ => {
                let reason = format!("Unsupported AST construct {}: local continuation is unknown", syntax.kind());
                Ok(self.unknown(syntax, incoming, &reason))
            }
        }
    }

    fn block(
        &mut self,
        block: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        if named_children(block)
            .iter()
            .any(|child| child.kind() == "label")
        {
            return Ok(self.unknown(
                block,
                incoming,
                "Labeled block exits are outside the supported loop-target profile",
            ));
        }
        let mut pending = incoming;
        let mut unsupported_attribute = None;
        for child in named_children(block) {
            if matches!(child.kind(), "attribute_item" | "inner_attribute_item") {
                if !safe_attribute(self.text(child)) {
                    unsupported_attribute = Some(child);
                }
                continue;
            }
            if let Some(attribute) = unsupported_attribute.take() {
                pending = self.unknown(
                    attribute,
                    pending,
                    "Conditional compilation or attribute expansion is unresolved",
                );
            }
            pending = self.expr(child, pending, depth + 1)?;
            if pending.is_empty() {
                break;
            }
        }
        Ok(pending)
    }

    fn let_binding(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let value = syntax.child_by_field_name("value");
        let pending = match value {
            Some(value) => self.expr(value, incoming, depth + 1)?,
            None => incoming,
        };
        if pending.is_empty() {
            return Ok(vec![]);
        }
        if let Some(pattern) = syntax.child_by_field_name("pattern") {
            if unsupported_pattern(pattern) {
                return Ok(self.unknown(
                    pattern,
                    pending,
                    "Pattern expansion, const evaluation, or recovery is unresolved",
                ));
            }
        }
        if let Some(alternative) = syntax.child_by_field_name("alternative") {
            let pattern = syntax.child_by_field_name("pattern").unwrap_or(syntax);
            let test = self.operation(pattern, "pattern_test", self.text(pattern));
            self.attach(pending, &test);
            let evidence = self.node_evidence(&test);
            let successful = vec![Port::branch(
                test.clone(),
                "pattern_match",
                "pattern match",
                evidence.clone(),
            )];
            let fallback = vec![Port::branch(test, "pattern_fallback", "else", evidence)];
            let leftover = self.expr(alternative, fallback, depth + 1)?;
            if !leftover.is_empty() {
                self.unknown(
                    alternative,
                    leftover,
                    "let-else fallback appears to fall through; divergence is unverified",
                );
            }
            Ok(self.action(syntax, successful, "binding", "Bind matched pattern"))
        } else {
            Ok(self.action(syntax, pending, "binding", self.text(syntax)))
        }
    }

    fn condition(
        &mut self,
        condition: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Branches, String> {
        if incoming.is_empty() {
            return Ok(Branches::default());
        }
        if !self.checkpoint(depth)? {
            self.unknown(
                condition,
                incoming,
                "Bounded traversal omitted this condition",
            );
            return Ok(Branches::default());
        }
        if condition.kind() == "parenthesized_expression" {
            if let Some(child) = first_executable_child(condition) {
                return self.condition(child, incoming, depth + 1);
            }
        }
        if condition.kind() == "binary_expression"
            && matches!(operator(self.source, condition), "&&" | "||")
        {
            let Some(left) = condition.child_by_field_name("left") else {
                self.unknown(
                    condition,
                    incoming,
                    "Short-circuit expression is missing its left operand",
                );
                return Ok(Branches::default());
            };
            let Some(right) = condition.child_by_field_name("right") else {
                self.unknown(
                    condition,
                    incoming,
                    "Short-circuit expression is missing its right operand",
                );
                return Ok(Branches::default());
            };
            let lhs = self.condition(left, incoming, depth + 1)?;
            if operator(self.source, condition) == "&&" {
                let rhs = self.condition(right, lhs.yes, depth + 1)?;
                let mut no = lhs.no;
                no.extend(rhs.no);
                return Ok(Branches { yes: rhs.yes, no });
            }
            let rhs = self.condition(right, lhs.no, depth + 1)?;
            let mut yes = lhs.yes;
            yes.extend(rhs.yes);
            return Ok(Branches { yes, no: rhs.no });
        }
        if condition.kind() == "let_chain" {
            let mut yes = incoming;
            let mut no = vec![];
            for child in named_children(condition) {
                let branch = self.condition(child, yes, depth + 1)?;
                yes = branch.yes;
                no.extend(branch.no);
            }
            return Ok(Branches { yes, no });
        }
        if condition.kind() == "let_condition" {
            let value = condition.child_by_field_name("value");
            let pending = match value {
                Some(value) => self.expr(value, incoming, depth + 1)?,
                None => incoming,
            };
            let pattern = condition
                .child_by_field_name("pattern")
                .unwrap_or(condition);
            if pending.is_empty() {
                return Ok(Branches::default());
            }
            if unsupported_pattern(pattern) {
                self.unknown(
                    pattern,
                    pending,
                    "Pattern expansion, const evaluation, or recovery is unresolved",
                );
                return Ok(Branches::default());
            }
            let test = self.operation(pattern, "pattern_test", self.text(pattern));
            self.attach(pending, &test);
            let evidence = self.node_evidence(&test);
            return Ok(Branches {
                yes: vec![Port::branch(
                    test.clone(),
                    "pattern_match",
                    "pattern match",
                    evidence.clone(),
                )],
                no: vec![Port::branch(
                    test,
                    "pattern_fallback",
                    "pattern fallback",
                    evidence,
                )],
            });
        }
        let evaluated = self.expr(condition, incoming, depth + 1)?;
        if evaluated.is_empty() {
            return Ok(Branches::default());
        }
        let test = self.operation(condition, "condition", self.text(condition));
        self.attach(evaluated, &test);
        if test == self.region.unknown {
            return Ok(Branches::default());
        }
        let evidence = self.node_evidence(&test);
        Ok(Branches {
            yes: vec![Port::branch(test.clone(), "true", "true", evidence.clone())],
            no: vec![Port::branch(test, "false", "false", evidence)],
        })
    }

    fn if_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let Some(condition) = syntax.child_by_field_name("condition") else {
            return Ok(self.unknown(syntax, incoming, "if expression has no retained condition"));
        };
        let branches = self.condition(condition, incoming, depth + 1)?;
        let mut endings = match syntax.child_by_field_name("consequence") {
            Some(body) => self.expr(body, branches.yes, depth + 1)?,
            None => self.unknown(
                syntax,
                branches.yes,
                "if expression has no retained consequence",
            ),
        };
        let alternative = syntax
            .child_by_field_name("alternative")
            .and_then(first_executable_child);
        let other = match alternative {
            Some(body) => self.expr(body, branches.no, depth + 1)?,
            None => branches.no,
        };
        endings.extend(other);
        if endings.is_empty() {
            return Ok(vec![]);
        }
        let join = self.synthetic(syntax, "join", "Conditional join");
        self.attach(endings, &join);
        Ok(vec![Port::next(join)])
    }

    fn match_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let Some(value) = syntax.child_by_field_name("value") else {
            return Ok(self.unknown(
                syntax,
                incoming,
                "match expression has no retained scrutinee",
            ));
        };
        let Some(body) = syntax.child_by_field_name("body") else {
            return Ok(self.unknown(syntax, incoming, "match expression has no retained arms"));
        };
        let mut fallback = self.expr(value, incoming, depth + 1)?;
        let mut endings = vec![];
        for arm in named_children(body)
            .into_iter()
            .filter(|node| node.kind() == "match_arm")
        {
            if let Some(attribute) = named_children(arm).into_iter().find(|child| {
                matches!(child.kind(), "attribute_item" | "inner_attribute_item")
                    && !safe_attribute(self.text(*child))
            }) {
                fallback = self.unknown(
                    attribute,
                    fallback,
                    "Conditional or expanded match arm is unresolved",
                );
                break;
            }
            let Some(pattern) = arm.child_by_field_name("pattern") else {
                fallback = self.unknown(arm, fallback, "Match arm has no retained pattern");
                break;
            };
            if unsupported_pattern(pattern) {
                fallback = self.unknown(
                    pattern,
                    fallback,
                    "Pattern expansion, const evaluation, or recovery is unresolved",
                );
                break;
            }
            let test = self.operation(pattern, "match_pattern", self.text(pattern));
            self.attach(fallback, &test);
            let evidence = self.node_evidence(&test);
            let mut selected = vec![Port::branch(
                test.clone(),
                "pattern_match",
                "pattern match",
                evidence.clone(),
            )];
            fallback = vec![Port::branch(
                test,
                "pattern_fallback",
                "next pattern (overlap/type unknown)",
                evidence,
            )];
            if let Some(guard) = pattern.child_by_field_name("condition") {
                let guarded = self.condition(guard, selected, depth + 1)?;
                selected = guarded.yes;
                fallback.extend(guarded.no.into_iter().map(|port| Port {
                    kind: "guard_fallback".into(),
                    label: "guard false → next arm".into(),
                    ..port
                }));
            }
            match arm.child_by_field_name("value") {
                Some(body) => endings.extend(self.expr(body, selected, depth + 1)?),
                None => {
                    self.unknown(arm, selected, "Match arm has no retained executable body");
                }
            }
        }
        self.gap("Match pattern exhaustiveness, overlap, and type feasibility are unverified");
        // This is an unresolved analysis frontier, not an invented executable
        // fallback arm or a claim that a panic occurs for unmatched inputs.
        let unknown = self.region.unknown.clone();
        for port in fallback {
            self.edge(
                &port.from,
                &unknown,
                "pattern_coverage_unknown",
                "unmatched coverage unknown",
                port.evidence,
            );
        }
        if endings.is_empty() {
            return Ok(vec![]);
        }
        let join = self.synthetic(syntax, "join", "Match-arm join");
        self.attach(endings, &join);
        Ok(vec![Port::next(join)])
    }

    fn loop_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let mut pending = incoming;
        if syntax.kind() == "for_expression" {
            if let Some(value) = syntax.child_by_field_name("value") {
                pending = self.expr(value, pending, depth + 1)?;
            }
            self.notice("for-loop iterator conversion, next dispatch, iteration count, and termination are unresolved");
        }
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let header = self.synthetic(syntax, "loop_header", "Loop header");
        self.attach(pending, &header);
        let exit = self.synthetic(syntax, "loop_exit", "Loop exit");
        let label = named_children(syntax)
            .into_iter()
            .find(|child| child.kind() == "label")
            .map(|child| self.text(child).to_owned());
        self.loops.push(LoopTarget {
            label,
            header: header.clone(),
            exit: exit.clone(),
            breaks: 0,
        });
        let (body_incoming, has_normal_exit) = match syntax.kind() {
            "while_expression" => {
                if let Some(condition) = syntax.child_by_field_name("condition") {
                    let branches =
                        self.condition(condition, vec![Port::next(header.clone())], depth + 1)?;
                    let normal_exit = !branches.no.is_empty();
                    self.attach(branches.no, &exit);
                    (branches.yes, normal_exit)
                } else {
                    (
                        self.unknown(
                            syntax,
                            vec![Port::next(header.clone())],
                            "while loop has no retained condition",
                        ),
                        false,
                    )
                }
            }
            "for_expression" => {
                let next =
                    self.synthetic(syntax, "iterator_next", "Iterator next (dispatch unknown)");
                self.edge(&header, &next, "next", "", None);
                let evidence = self.node_evidence(&next);
                self.edge(
                    &next,
                    &exit,
                    "iterator_exhausted",
                    "iterator exhausted",
                    Some(evidence.clone()),
                );
                let items = vec![Port::branch(next, "iterator_item", "next item", evidence)];
                let binding = syntax.child_by_field_name("pattern").unwrap_or(syntax);
                let body_input = if unsupported_pattern(binding) {
                    self.unknown(
                        binding,
                        items,
                        "Iterator pattern expansion, const evaluation, or recovery is unresolved",
                    )
                } else {
                    self.action(binding, items, "iteration_binding", self.text(binding))
                };
                (body_input, true)
            }
            _ => (vec![Port::next(header.clone())], false),
        };
        let back = match syntax.child_by_field_name("body") {
            Some(body) => self.expr(body, body_incoming, depth + 1)?,
            None => self.unknown(syntax, body_incoming, "Loop has no retained body"),
        };
        for port in back {
            self.edge(
                &port.from,
                &header,
                "loop_back",
                "next iteration (count unknown)",
                port.evidence,
            );
        }
        let target = self.loops.pop().expect("loop target was pushed");
        if has_normal_exit || target.breaks > 0 {
            Ok(vec![Port::next(exit)])
        } else {
            Ok(vec![])
        }
    }

    fn return_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let mut pending = incoming;
        if let Some(value) = first_executable_child(syntax) {
            pending = self.expr(value, pending, depth + 1)?;
        }
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let returned = self.operation(syntax, "return", self.text(syntax));
        self.attach(pending, &returned);
        let exit = self.region.exit.clone();
        self.edge(
            &returned,
            &exit,
            "return",
            "return from this region",
            Some(self.node_evidence(&returned)),
        );
        Ok(vec![])
    }

    fn loop_exit(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let children = named_children(syntax);
        let label = children
            .iter()
            .find(|child| child.kind() == "label")
            .map(|child| self.text(*child));
        let target = self
            .loops
            .iter()
            .rposition(|target| label.is_none() || target.label.as_deref() == label);
        let Some(target_index) = target else {
            return Ok(self.unknown(
                syntax,
                incoming,
                "break/continue label does not resolve to a loop in this executable region",
            ));
        };
        let mut pending = incoming;
        for child in children.into_iter().filter(|child| child.kind() != "label") {
            pending = self.expr(child, pending, depth + 1)?;
        }
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let is_break = syntax.kind() == "break_expression";
        let operation = if is_break { "break" } else { "continue" };
        let jump = self.operation(syntax, operation, self.text(syntax));
        self.attach(pending, &jump);
        let target = if is_break {
            self.loops[target_index].breaks += 1;
            self.loops[target_index].exit.clone()
        } else {
            self.loops[target_index].header.clone()
        };
        self.edge(
            &jump,
            &target,
            operation,
            label.unwrap_or("nearest loop"),
            Some(self.node_evidence(&jump)),
        );
        Ok(vec![])
    }

    fn try_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let Some(operand) = first_executable_child(syntax) else {
            return Ok(self.unknown(syntax, incoming, "Try propagation is missing its operand"));
        };
        let pending = self.expr(operand, incoming, depth + 1)?;
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let question = self.operation(
            syntax,
            "try_propagation",
            "? : Try operand / residual unresolved",
        );
        self.attach(pending, &question);
        let evidence = self.node_evidence(&question);
        let exit = self.region.exit.clone();
        self.edge(
            &question,
            &exit,
            "propagate_residual",
            "propagate residual (Try conversion unknown)",
            Some(evidence.clone()),
        );
        self.notice("? propagation uses unresolved Try/FromResidual behavior; it does not imply Result::Err");
        Ok(vec![Port::branch(
            question,
            "try_continue",
            "continue",
            evidence,
        )])
    }

    fn await_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let Some(operand) = first_executable_child(syntax) else {
            return Ok(self.unknown(syntax, incoming, "Await expression is missing its operand"));
        };
        let pending = self.expr(operand, incoming, depth + 1)?;
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let awaited = self.operation(syntax, "await", ".await : possible suspension boundary");
        self.attach(pending, &awaited);
        let resumed = self.synthetic(
            syntax,
            "await_resume",
            "Resume continuation (scheduling unknown)",
        );
        self.edge(
            &awaited,
            &resumed,
            "await_resume",
            "resume; polling/cancellation unknown",
            Some(self.node_evidence(&awaited)),
        );
        self.notice(
            "Await scheduling, polling, cancellation, and external result values are unresolved",
        );
        Ok(vec![Port::next(resumed)])
    }

    fn call_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let mut pending = incoming;
        if let Some(function) = syntax.child_by_field_name("function") {
            pending = self.expr(function, pending, depth + 1)?;
        }
        if let Some(arguments) = syntax.child_by_field_name("arguments") {
            for argument in named_children(arguments) {
                if matches!(argument.kind(), "attribute_item" | "inner_attribute_item") {
                    if !safe_attribute(self.text(argument)) {
                        pending = self.unknown(
                            argument,
                            pending,
                            "Conditional/expanded call argument is unresolved",
                        );
                    }
                } else {
                    pending = self.expr(argument, pending, depth + 1)?;
                }
            }
        }
        if pending.is_empty() {
            return Ok(vec![]);
        }
        let call_name = syntax
            .child_by_field_name("function")
            .map(|value| self.text(value))
            .unwrap_or("unknown callee");
        let label = format!("Call {call_name} · target/effect unresolved");
        let tokens = semantic_tokens(syntax, self.source);
        let call = self.add_node(
            syntax,
            "call",
            &tokens,
            &label,
            "target_and_effect_unresolved",
        );
        self.attach(pending, &call);
        self.notice("Observed call syntax does not resolve dispatch, prove a callee, or identify an external effect");
        if call == self.region.unknown {
            Ok(vec![])
        } else {
            Ok(vec![Port::next(call)])
        }
    }

    fn value_expr(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        let mut pending = incoming;
        let values: Vec<Node<'_>> = match syntax.kind() {
            "binary_expression" => ["left", "right"]
                .into_iter()
                .filter_map(|field| syntax.child_by_field_name(field))
                .collect(),
            "reference_expression" | "type_cast_expression" | "field_expression" => {
                syntax.child_by_field_name("value").into_iter().collect()
            }
            "struct_expression" => {
                let mut values = vec![];
                if let Some(body) = syntax.child_by_field_name("body") {
                    for field in named_children(body) {
                        if named_children(field).iter().any(|child| {
                            matches!(child.kind(), "attribute_item" | "inner_attribute_item")
                                && !safe_attribute(self.text(*child))
                        }) {
                            return Ok(self.unknown(
                                field,
                                pending,
                                "Conditional/expanded struct field initializer is unresolved",
                            ));
                        }
                        match field.kind() {
                            "field_initializer" => {
                                if let Some(value) = field.child_by_field_name("value") {
                                    values.push(value);
                                }
                            }
                            "shorthand_field_initializer" | "base_field_initializer" => {
                                if let Some(value) = first_executable_child(field) {
                                    values.push(value);
                                }
                            }
                            _ => {
                                return Ok(self.unknown(
                                    field,
                                    pending,
                                    "Struct initializer is outside the supported local profile",
                                ));
                            }
                        }
                    }
                }
                values
            }
            _ => named_children(syntax)
                .into_iter()
                .filter(|child| {
                    !matches!(
                        child.kind(),
                        "attribute_item"
                            | "inner_attribute_item"
                            | "mutable_specifier"
                            | "type_identifier"
                            | "primitive_type"
                    )
                })
                .collect(),
        };
        if named_children(syntax).iter().any(|child| {
            matches!(child.kind(), "attribute_item" | "inner_attribute_item")
                && !safe_attribute(self.text(*child))
        }) {
            return Ok(self.unknown(
                syntax,
                pending,
                "Conditional/expanded expression child is unresolved",
            ));
        }
        for value in values {
            pending = self.expr(value, pending, depth + 1)?;
        }
        Ok(self.action(syntax, pending, "operation", self.text(syntax)))
    }

    fn nested_region(
        &mut self,
        syntax: Node<'_>,
        incoming: Vec<Port>,
        depth: usize,
    ) -> Result<Vec<Port>, String> {
        if self.graph.nodes.len() + 4 > MAX_FLOW_NODES
            || self.payload_bytes + 8192 > FLOW_PAYLOAD_BUDGET
        {
            return Ok(self.unknown(
                syntax,
                incoming,
                "Nested region cannot fit the remaining local graph budget",
            ));
        }
        let operation = if syntax.kind() == "function_item" {
            "function_definition"
        } else {
            "region_definition"
        };
        let kind = match syntax.kind() {
            "closure_expression" => "closure",
            "async_block" => "async",
            _ => "nested_function",
        };
        let definition = self.operation(
            syntax,
            operation,
            &format!("Define {kind}; invocation/captures unresolved"),
        );
        self.attach(incoming, &definition);
        if definition == self.region.unknown {
            return Ok(vec![]);
        }
        self.notice("Nested closure/async/function region is a definition, not evidence of invocation or callback ordering");
        let parent_region = self.region.clone();
        let parent_loops = std::mem::take(&mut self.loops);
        self.region = Region {
            id: id(
                "region",
                &format!("{}:{}:{}", self.occurrence, syntax.start_byte(), kind),
            ),
            exit: String::new(),
            unknown: String::new(),
        };
        let entry = self.synthetic(syntax, "entry", &format!("{kind} entry (when invoked)"));
        let exit = self.synthetic(syntax, "exit", &format!("{kind} result / return"));
        let unknown = self.synthetic(
            syntax,
            "unknown_exit",
            "Unknown nested-region control frontier",
        );
        self.region.exit = exit.clone();
        self.region.unknown = unknown;
        self.graph.regions.push(LldFlowRegion {
            id: self.region.id.clone(),
            parent_node: Some(definition.clone()),
            kind: kind.into(),
            entry: entry.clone(),
            exit: exit.clone(),
        });
        let body = syntax.child_by_field_name("body").or_else(|| {
            named_children(syntax)
                .into_iter()
                .find(|child| child.kind() == "block")
        });
        if let Some(body) = body {
            let endings = self.expr(body, vec![Port::next(entry)], depth + 1)?;
            for port in endings {
                self.edge(
                    &port.from,
                    &exit,
                    "tail_result",
                    "nested region tail result",
                    port.evidence,
                );
            }
        } else {
            self.unknown(
                syntax,
                vec![Port::next(entry)],
                "Nested executable region has no retained body",
            );
        }
        self.region = parent_region;
        self.loops = parent_loops;
        Ok(vec![Port::next(definition)])
    }
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| !matches!(child.kind(), "line_comment" | "block_comment"))
        .collect()
}
fn first_executable_child(node: Node<'_>) -> Option<Node<'_>> {
    named_children(node).into_iter().find(|child| {
        !matches!(
            child.kind(),
            "label" | "attribute_item" | "inner_attribute_item"
        )
    })
}
fn operator<'a>(source: &'a str, node: Node<'_>) -> &'a str {
    node.child_by_field_name("operator")
        .and_then(|operator| source.get(operator.byte_range()))
        .unwrap_or("")
}
fn safe_attribute(text: &str) -> bool {
    let tag = text
        .trim_start_matches('#')
        .trim_start_matches('!')
        .trim_start_matches('[')
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    matches!(
        tag,
        "allow"
            | "warn"
            | "deny"
            | "forbid"
            | "doc"
            | "inline"
            | "cold"
            | "must_use"
            | "deprecated"
            | "track_caller"
            | "test"
            | "ignore"
            | "should_panic"
    )
}
fn unsupported_pattern(node: Node<'_>) -> bool {
    let mut pending = vec![(node, 0)];
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_FLOW_DEPTH
            || node.is_error()
            || node.is_missing()
            || matches!(node.kind(), "macro_invocation" | "const_block")
        {
            return true;
        }
        pending.extend(
            named_children(node)
                .into_iter()
                .map(|node| (node, depth + 1)),
        );
    }
    false
}
fn transforming_attribute<'a>(source: &str, node: Node<'a>) -> Option<Node<'a>> {
    let mut previous = node.prev_named_sibling();
    while let Some(attribute) = previous {
        if matches!(attribute.kind(), "attribute_item" | "inner_attribute_item") {
            if !safe_attribute(source.get(attribute.byte_range()).unwrap_or("")) {
                return Some(attribute);
            }
        } else if !matches!(attribute.kind(), "line_comment" | "block_comment") {
            break;
        }
        previous = attribute.prev_named_sibling();
    }
    None
}
fn inherited_transforming_attribute<'a>(source: &str, node: Node<'a>) -> Option<Node<'a>> {
    let mut ancestor = Some(node);
    while let Some(node) = ancestor {
        if let Some(attribute) = transforming_attribute(source, node) {
            return Some(attribute);
        }
        ancestor = node.parent();
    }
    None
}
fn bounded_label(label: &str) -> String {
    let compact = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut result: String = compact.chars().take(180).collect();
    if compact.chars().count() > 180 {
        result.push('…');
    }
    result
}
fn id(kind: &str, value: &str) -> String {
    format!("lld-{kind}-{}", hex_digest(value.as_bytes()))
}
fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stable semantic projection excludes immutable occurrence IDs, source
/// coordinates, display labels, and layout. Repeated operations keep their
/// ordered graph occurrences; literal bytes survive token normalization.
fn graph_fingerprint(graph: &LldFlow) -> String {
    let indexes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (&node.id, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let regions = graph
        .regions
        .iter()
        .enumerate()
        .map(|(index, region)| (&region.id, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let nodes = graph
        .nodes
        .iter()
        .map(|node| {
            (
                regions.get(&node.region_id).copied(),
                &node.ast_kind,
                &node.operation,
                &node.tokens,
                &node.resolution,
            )
        })
        .collect::<Vec<_>>();
    let edges = graph
        .edges
        .iter()
        .map(|edge| {
            (
                indexes.get(&edge.from).copied(),
                indexes.get(&edge.to).copied(),
                &edge.kind,
                &edge.label,
            )
        })
        .collect::<Vec<_>>();
    let region_projection = graph
        .regions
        .iter()
        .map(|region| {
            (
                &region.kind,
                region
                    .parent_node
                    .as_ref()
                    .and_then(|node| indexes.get(node).copied()),
                indexes.get(&region.entry).copied(),
                indexes.get(&region.exit).copied(),
            )
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&(
        nodes,
        edges,
        region_projection,
        graph.complete,
        graph.omitted_nodes,
        graph.omitted_edges,
    ))
    .unwrap_or_default();
    hex_digest(&encoded)
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
