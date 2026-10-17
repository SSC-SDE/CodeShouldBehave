use super::*;
use crate::{SecurityDomain, lld::LldParentBinding};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tree_sitter::{Parser, Tree};

fn tree(source: &str) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}
fn first_function(node: Node<'_>) -> Node<'_> {
    if node.kind() == "function_item" {
        return node;
    }
    let mut cursor = node.walk();
    fn find(node: Node<'_>) -> Option<Node<'_>> {
        if node.kind() == "function_item" {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(found) = find(child) {
                return Some(found);
            }
        }
        None
    }
    node.named_children(&mut cursor).find_map(find).unwrap()
}
fn entry(function: Node<'_>) -> LldEntry {
    LldEntry {
        id: "lld-entry-test".into(),
        logical_key: "fixture|test|function".into(),
        name: "test".into(),
        kind: "function".into(),
        module: "fixture".into(),
        container: String::new(),
        parent_entry_id: None,
        child_entry_ids: vec![],
        path: "src/lib.rs".into(),
        blob: "f".repeat(40),
        start_byte: function.start_byte(),
        end_byte: function.end_byte(),
        start_line: function.start_position().row + 1,
        end_line: function.end_position().row + 1,
        visibility: "private".into(),
        signature: String::new(),
        signature_fingerprint: String::new(),
        body_fingerprint: String::new(),
        projections: vec![],
        call_observations: vec![],
        detail_state: "available".into(),
        evidence_ids: vec![],
        parent: LldParentBinding {
            hld_design_id: "hld-test".into(),
            component_id: None,
            candidate_component_ids: vec![],
            state: "unmapped".into(),
            mapping_basis: "fixture".into(),
            mapping_fingerprint: String::new(),
        },
        limitations: vec![],
    }
}
fn flow(source: &str) -> LldFlow {
    let tree = tree(source);
    let function = first_function(tree.root_node());
    let repository = RepositoryId::new(
        SecurityDomain::new("flow-test").unwrap(),
        "local",
        "fixture",
        "rust",
    )
    .unwrap();
    lower_flow(
        source,
        function,
        &entry(function),
        &SnapshotId::new("a".repeat(40)).unwrap(),
        &repository,
        &Cancellation::default(),
    )
    .unwrap()
}
fn nodes<'a>(graph: &'a LldFlow, operation: &str) -> Vec<&'a LldBehaviorNode> {
    graph
        .nodes
        .iter()
        .filter(|node| node.operation == operation)
        .collect()
}
fn label_node<'a>(graph: &'a LldFlow, operation: &str, label: &str) -> &'a LldBehaviorNode {
    graph
        .nodes
        .iter()
        .find(|node| node.operation == operation && node.label == label)
        .unwrap_or_else(|| panic!("missing {operation}:{label}\n{graph:#?}"))
}
fn reaches(graph: &LldFlow, from: &str, to: &str) -> bool {
    let mut todo = VecDeque::from([from]);
    let mut visited = BTreeSet::new();
    while let Some(node) = todo.pop_front() {
        if node == to {
            return true;
        }
        if visited.insert(node) {
            todo.extend(
                graph
                    .edges
                    .iter()
                    .filter(|edge| edge.from == node)
                    .map(|edge| edge.to.as_str()),
            );
        }
    }
    false
}
fn edge_projection(graph: &LldFlow) -> BTreeSet<(String, String, String)> {
    let nodes = graph
        .nodes
        .iter()
        .map(|node| (&node.id, format!("{}:{}", node.operation, node.label)))
        .collect::<BTreeMap<_, _>>();
    graph
        .edges
        .iter()
        .map(|edge| {
            (
                nodes[&edge.from].clone(),
                edge.kind.clone(),
                nodes[&edge.to].clone(),
            )
        })
        .collect()
}

#[test]
fn golden_if_returns_only_on_true_and_joins_only_the_live_else() {
    let graph = flow("fn test(x:i32)->i32 { if x > 0 { return 1; } else { 2 } }");
    assert!(graph.complete, "{:?}", graph.limitations);
    let expected = [
        ("entry:Function entry", "next", "value:x"),
        ("value:x", "next", "value:0"),
        ("value:0", "next", "operation:x > 0"),
        ("operation:x > 0", "next", "condition:x > 0"),
        ("condition:x > 0", "true", "value:1"),
        ("value:1", "next", "return:return 1"),
        ("return:return 1", "return", "exit:Function result / return"),
        ("condition:x > 0", "false", "value:2"),
        ("value:2", "next", "join:Conditional join"),
        (
            "join:Conditional join",
            "next",
            "tail_result:Implicit function tail result",
        ),
        (
            "tail_result:Implicit function tail result",
            "tail_result",
            "exit:Function result / return",
        ),
    ]
    .into_iter()
    .map(|(from, kind, to)| (from.into(), kind.into(), to.into()))
    .collect();
    assert_eq!(edge_projection(&graph), expected);
    assert!(
        graph
            .edges
            .iter()
            .all(|edge| graph.nodes.iter().any(|node| node.id == edge.from)
                && graph.nodes.iter().any(|node| node.id == edge.to))
    );
}

#[test]
fn early_return_has_no_fallthrough_and_does_not_connect_dead_operations() {
    let graph = flow("fn test(){ return; never_called(); }");
    assert!(graph.complete);
    assert!(nodes(&graph, "unknown_exit").is_empty());
    assert!(nodes(&graph, "call").is_empty());
    let returned = nodes(&graph, "return")[0];
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.from == returned.id)
            .map(|edge| edge.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["return"]
    );
}

#[test]
fn nested_short_circuit_golden_rhs_edges_preserve_and_or_laziness() {
    let graph = flow("fn test(){ if first() && (second() || third()) { yes(); } else { no(); } }");
    assert!(graph.complete, "{:?}", graph.limitations);
    let first = label_node(&graph, "condition", "first()");
    let second = label_node(&graph, "condition", "second()");
    let third = label_node(&graph, "condition", "third()");
    let second_callee = label_node(&graph, "value", "second");
    let third_callee = label_node(&graph, "value", "third");
    let yes = label_node(&graph, "value", "yes");
    let no = label_node(&graph, "value", "no");
    let relevant = graph
        .edges
        .iter()
        .filter(|edge| [&first.id, &second.id, &third.id].contains(&&edge.from))
        .map(|edge| (edge.from.as_str(), edge.kind.as_str(), edge.to.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        relevant,
        BTreeSet::from([
            (first.id.as_str(), "true", second_callee.id.as_str()),
            (first.id.as_str(), "false", no.id.as_str()),
            (second.id.as_str(), "true", yes.id.as_str()),
            (second.id.as_str(), "false", third_callee.id.as_str()),
            (third.id.as_str(), "true", yes.id.as_str()),
            (third.id.as_str(), "false", no.id.as_str()),
        ])
    );
    assert_eq!(nodes(&graph, "call").len(), 5);
}

#[test]
fn if_let_and_let_else_patterns_keep_success_and_resolved_else_return_separate() {
    let graph = flow(
        "fn test(v:Option<i32>)->i32 { let Some(x)=v else { return 0; }; if let Some(y)=other() { x+y } else { x } }",
    );
    assert!(graph.complete, "{:?}", graph.limitations);
    assert_eq!(nodes(&graph, "pattern_test").len(), 2);
    let pattern = nodes(&graph, "pattern_test")[0];
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.from == pattern.id)
            .map(|edge| edge.kind.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["pattern_match", "pattern_fallback"])
    );
    assert_eq!(nodes(&graph, "return").len(), 1);
    let returned = nodes(&graph, "return")[0];
    let binding = nodes(&graph, "binding")[0];
    assert!(!reaches(&graph, &returned.id, &binding.id));
}

#[test]
fn invalid_looking_let_else_fallthrough_is_unknown_without_invented_join() {
    let graph = flow("fn test(v:Option<i32>){ let Some(x)=v else { do_work(); }; use_x(x); }");
    assert!(!graph.complete);
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("fall through"))
    );
    let fallback_call = graph
        .nodes
        .iter()
        .find(|node| node.operation == "call" && node.label.contains("do_work"))
        .unwrap();
    let use_call = graph
        .nodes
        .iter()
        .find(|node| node.operation == "call" && node.label.contains("use_x"))
        .unwrap();
    assert!(!reaches(&graph, &fallback_call.id, &use_call.id));
}

#[test]
fn match_guards_fall_through_to_next_pattern_and_unknown_coverage_is_not_runtime_fallback() {
    let graph = flow("fn test(x:i32)->i32 { match x { 1 if allowed() => 10, 2 => 20, _ => 30 } }");
    assert!(!graph.complete);
    let patterns = nodes(&graph, "match_pattern");
    assert_eq!(patterns.len(), 3);
    let guard = label_node(&graph, "condition", "allowed()");
    assert!(graph.edges.iter().any(|edge| edge.from == guard.id
        && edge.to == patterns[1].id
        && edge.kind == "guard_fallback"));
    assert!(graph.edges.iter().any(|edge| edge.from == patterns[0].id
        && edge.to == patterns[1].id
        && edge.kind == "pattern_fallback"));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == patterns[2].id && edge.kind == "pattern_coverage_unknown")
    );
    assert_eq!(nodes(&graph, "join").len(), 1);
    assert!(!graph.nodes.iter().any(|node| node.label.contains("panic")));
}

#[test]
fn labeled_nested_loops_resolve_break_and_continue_to_the_correct_outer_loop() {
    let graph = flow(
        "fn test(mut n:i32)->i32 { 'outer: while n > 0 { loop { if skip() { continue 'outer; } break 'outer n; } } n }",
    );
    assert!(graph.complete, "{:?}", graph.limitations);
    let headers = nodes(&graph, "loop_header");
    let exits = nodes(&graph, "loop_exit");
    assert_eq!(headers.len(), 2);
    let continued = nodes(&graph, "continue")[0];
    let broken = nodes(&graph, "break")[0];
    assert!(graph.edges.iter().any(|edge| edge.from == continued.id
        && edge.to == headers[0].id
        && edge.kind == "continue"
        && edge.label == "'outer"));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == broken.id && edge.to == exits[0].id && edge.kind == "break")
    );
    let break_value = graph
        .nodes
        .iter()
        .find(|node| {
            node.operation == "value"
                && node.label == "n"
                && node.start_byte >= broken.start_byte
                && node.end_byte <= broken.end_byte
        })
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == break_value.id && edge.to == broken.id)
    );
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.from == broken.id && edge.to == exits[1].id)
    );
}

#[test]
fn for_loop_evaluates_iterable_once_and_back_edge_targets_the_next_header() {
    let graph = flow("fn test(){ for item in items() { consume(item); } }");
    assert!(graph.complete, "{:?}", graph.limitations);
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("iterator conversion"))
    );
    let header = nodes(&graph, "loop_header")[0];
    let iterable = graph
        .nodes
        .iter()
        .find(|node| node.operation == "call" && node.label.contains("items"))
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == iterable.id && edge.to == header.id)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.to == header.id && edge.kind == "loop_back")
    );
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.kind == "loop_back" && edge.to == iterable.id)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == "iterator_exhausted")
    );
}

#[test]
fn while_let_has_pattern_fallback_and_back_edge_without_inferred_iteration_count() {
    let graph = flow("fn test(){ while let Some(x)=next() { work(x); } }");
    assert!(graph.complete);
    assert_eq!(nodes(&graph, "pattern_test").len(), 1);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == "pattern_fallback")
    );
    assert!(graph.edges.iter().any(|edge| edge.kind == "loop_back"));
}

#[test]
fn question_mark_and_await_have_symbolic_residual_and_resume_edges() {
    let graph = flow("async fn test()->Result<i32,E>{ let x = request().await?; Ok(x) }");
    assert!(graph.complete, "{:?}", graph.limitations);
    let propagated = nodes(&graph, "try_propagation")[0];
    let exit = nodes(&graph, "exit")[0];
    assert!(graph.edges.iter().any(|edge| edge.from == propagated.id
        && edge.to == exit.id
        && edge.kind == "propagate_residual"
        && edge.label.contains("Try conversion unknown")));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == propagated.id && edge.kind == "try_continue")
    );
    let awaited = nodes(&graph, "await")[0];
    let resumed = nodes(&graph, "await_resume")[0];
    assert!(graph.edges.iter().any(|edge| edge.from == awaited.id
        && edge.to == resumed.id
        && edge.kind == "await_resume"));
    assert!(
        !graph
            .edges
            .iter()
            .any(|edge| edge.label.contains("Result::Err"))
    );
    assert!(
        nodes(&graph, "call")
            .iter()
            .all(|node| node.resolution == "target_and_effect_unresolved")
    );
}

#[test]
fn nested_closure_and_async_returns_stay_inside_own_region_and_definition_is_not_invocation() {
    let graph =
        flow("fn test(){ let f = || { return 7; }; let a = async { return 8; }; after(); }");
    assert!(graph.complete, "{:?}", graph.limitations);
    assert_eq!(graph.regions.len(), 3);
    let root = &graph.regions[0];
    for returned in nodes(&graph, "return") {
        let region = graph
            .regions
            .iter()
            .find(|region| region.id == returned.region_id)
            .unwrap();
        assert_ne!(region.id, root.id);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.from == returned.id && edge.to == region.exit)
        );
        assert!(
            !graph
                .edges
                .iter()
                .any(|edge| edge.from == returned.id && edge.to == root.exit)
        );
    }
    for region in graph.regions.iter().skip(1) {
        let definition = region.parent_node.as_ref().unwrap();
        assert!(
            !graph
                .edges
                .iter()
                .any(|edge| edge.from == *definition && edge.to == region.entry)
        );
        assert!(!graph.edges.iter().any(|edge| edge.to == region.entry));
    }
    let after = graph
        .nodes
        .iter()
        .find(|node| node.operation == "call" && node.label.contains("after"))
        .unwrap();
    assert!(reaches(&graph, &root.entry, &after.id));
}

#[test]
fn unknown_regions_never_silently_continue_or_promote_named_calls_to_external_effects() {
    for source in [
        "fn test(){ opaque!(); after(); }",
        "fn test(){ unsafe { ptr.read(); } after(); }",
        "fn test(){ 'block: { break 'block; } after(); }",
        "#[cfg(feature=\"optional\")] fn test(){ after(); }",
        "fn test(){ x += side_effect(); after(); }",
    ] {
        let graph = flow(source);
        assert!(!graph.complete, "{source}");
        assert!(!nodes(&graph, "unsupported_region").is_empty(), "{source}");
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.operation == "call" && node.label.contains("after")),
            "{source}"
        );
        for unknown in nodes(&graph, "unknown_exit") {
            assert!(!graph.edges.iter().any(|edge| edge.from == unknown.id));
        }
    }
    let graph = flow("fn test(){ database.write(); network.send(); payment.capture(); }");
    assert_eq!(nodes(&graph, "call").len(), 3);
    assert!(
        nodes(&graph, "call")
            .iter()
            .all(|node| node.resolution == "target_and_effect_unresolved")
    );
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation.as_str(), "network" | "database" | "payment"))
    );
}

#[test]
fn assignment_rhs_is_evaluated_before_assignee() {
    let graph = flow("fn test(){ target()[index()] = value(); }");
    assert!(graph.complete, "{:?}", graph.limitations);
    let calls = nodes(&graph, "call");
    assert_eq!(calls.len(), 3);
    assert!(calls[0].label.contains("value"));
    assert!(calls[1].label.contains("target"));
    assert!(calls[2].label.contains("index"));
    assert_eq!(nodes(&graph, "assignment").len(), 1);
}

#[test]
fn formatting_comment_and_line_shift_do_not_change_projection_but_literal_bytes_do() {
    let compact = flow("fn test(x:bool){ if x { emit(\"a b\"); } else { emit(r#\"x y\"#); } }");
    let formatted = flow(
        "// heading\nfn test(x: bool) {\n // comment\n if x { emit(\"a b\"); /* inert */ }\n else { emit(r#\"x y\"#); }\n}\n",
    );
    assert_eq!(compact.fingerprint, formatted.fingerprint);
    let changed = flow("fn test(x:bool){ if x { emit(\"ab\"); } else { emit(r#\"x y\"#); } }");
    assert_ne!(compact.fingerprint, changed.fingerprint);
    assert_ne!(
        compact
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>(),
        formatted
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn repeated_operations_keep_distinct_occurrences_and_condition_operator_changes_projection() {
    let graph = flow("fn test(x:i32){ run(); run(); if x > 1 { run(); } }");
    assert_eq!(nodes(&graph, "call").len(), 3);
    assert_eq!(
        nodes(&graph, "call")
            .iter()
            .map(|node| &node.id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    assert_ne!(
        graph.fingerprint,
        flow("fn test(x:i32){ run(); run(); if x >= 1 { run(); } }").fingerprint
    );
}

#[test]
fn bounds_and_cancellation_report_incomplete_instead_of_false_clean() {
    let source = format!("fn test(){{ {} }}", "call();".repeat(300));
    let graph = flow(&source);
    assert!(!graph.complete);
    assert!(graph.nodes.len() <= MAX_FLOW_NODES);
    assert!(graph.edges.len() <= MAX_FLOW_EDGES);
    assert!(graph.omitted_nodes > 0);
    assert!(serde_json::to_vec(&graph).unwrap().len() <= MAX_FLOW_BYTES);
    let nested = format!("fn test(){{ {}true{}; }}", "(".repeat(80), ")".repeat(80));
    let graph = flow(&nested);
    assert!(!graph.complete);
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("depth 32"))
    );
    let source = "fn test(){}";
    let tree = tree(source);
    let function = first_function(tree.root_node());
    let repository = RepositoryId::new(
        SecurityDomain::new("flow-test").unwrap(),
        "local",
        "fixture",
        "rust",
    )
    .unwrap();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        lower_flow(
            source,
            function,
            &entry(function),
            &SnapshotId::new("a".repeat(40)).unwrap(),
            &repository,
            &cancellation
        )
        .unwrap_err()
        .contains("cancelled")
    );
}

#[test]
fn parse_recovery_is_explicit_and_does_not_present_a_complete_cfg() {
    let graph = flow("fn test(){ let value = ; after(); }");
    assert!(!graph.complete);
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("parse recovery"))
    );
    assert!(nodes(&graph, "call").is_empty());
}

#[test]
fn lightweight_catalog_inventory_does_not_hide_unsupported_lazy_detail() {
    for source in [
        "fn test(){ opaque!(); }",
        "#[cfg(feature=\"x\")] fn test(){}",
        "fn test(){ unsafe { work(); } }",
        "fn test(){ try { work()?; } }",
        "fn test(){ 'block: { break 'block; } }",
        "fn test(){ break 'missing; }",
        "fn test(){ let v=async { continue; }; }",
        "fn test(x:i32){ match x { _=>() } }",
        "fn test(v:Option<i32>){ let Some(x)=v else { work(); }; }",
    ] {
        let tree = tree(source);
        assert!(
            !local_syntax_limitations(first_function(tree.root_node()), source).is_empty(),
            "{source}"
        );
    }
    let source =
        "fn test(v:Option<i32>)->i32{ let Some(x)=v else { return 0; }; if x>2 { x } else { 2 } }";
    let tree = tree(source);
    assert!(local_syntax_limitations(first_function(tree.root_node()), source).is_empty());
}

#[test]
fn inherited_cfg_or_attribute_expansion_makes_nested_function_flow_unknown() {
    for source in [
        "#[cfg(feature=\"x\")] mod conditional { fn test(){ work(); } }",
        "#[transform] impl Owner { fn test(){ work(); } }",
    ] {
        let graph = flow(source);
        assert!(!graph.complete);
        assert!(nodes(&graph, "call").is_empty());
        assert!(!nodes(&graph, "unsupported_region").is_empty());
    }
}

#[test]
fn generic_method_callee_evaluates_its_receiver_before_the_outer_call() {
    let graph = flow("fn test(){ receiver().send::<Message>(value()); }");
    assert!(graph.complete, "{:?}", graph.limitations);
    let calls = nodes(&graph, "call");
    assert_eq!(calls.len(), 3);
    assert!(calls[0].label.contains("receiver"));
    assert!(calls[1].label.contains("value"));
    assert!(calls[2].label.contains("send::<Message>"));
    assert!(reaches(&graph, &calls[0].id, &calls[2].id));
    assert!(nodes(&graph, "generic_callee").iter().all(|node| reaches(
        &graph,
        &calls[0].id,
        &node.id
    )));
}

#[test]
fn repeat_array_const_length_does_not_become_an_invented_runtime_call_sequence() {
    let source = "fn test(){ let items=[element(); length()]; after(); }";
    let graph = flow(source);
    assert!(!graph.complete);
    assert!(nodes(&graph, "call").is_empty());
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("Repeat-array"))
    );
    let tree = tree(source);
    assert!(
        local_syntax_limitations(first_function(tree.root_node()), source)
            .iter()
            .any(|reason| reason.contains("Repeat-array"))
    );
}

#[test]
fn serialized_payload_exhaustion_retains_a_bounded_partial_graph() {
    let literal = "x".repeat(2000);
    let source = format!(
        "fn test(){{ {} }}",
        format!("emit(\"{literal}\");").repeat(40)
    );
    let graph = flow(&source);
    assert!(!graph.complete);
    assert!(
        graph
            .limitations
            .iter()
            .any(|reason| reason.contains("128 KiB"))
    );
    assert!(graph.omitted_nodes > 0 || graph.omitted_edges > 0);
    assert!(serde_json::to_vec(&graph).unwrap().len() <= MAX_FLOW_BYTES);
    assert!(graph.nodes.len() < MAX_FLOW_NODES);
    assert!(!nodes(&graph, "unknown_exit").is_empty());
}
