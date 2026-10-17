use super::*;

fn escape(text: &str) -> String {
    text.chars()
        .take(160)
        .map(|ch| match ch {
            '"' => "&quot;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '&' => "&amp;".into(),
            '\n' | '\r' => " ".into(),
            '[' | ']' | '{' | '}' | '`' => format!("&#{};", ch as u32),
            _ => ch.to_string(),
        })
        .collect()
}
pub fn render_lld_mermaid(detail: &LldFunctionDetail) -> String {
    let mut output = String::from("flowchart TD\n");
    if detail.graph.nodes.is_empty() {
        output.push_str(&format!(
            "  declaration[\"{} · {}\"]\n",
            escape(&detail.entry.kind),
            escape(&detail.entry.name)
        ));
        for (i, child) in detail.declarations.iter().take(100).enumerate() {
            output.push_str(&format!(
                "  c{i}[\"{} · {}\"]\n  declaration --> c{i}\n",
                escape(&child.kind),
                escape(&child.name)
            ));
        }
        output.push_str("  note[\"Declaration structure only · select a function for local flow\"]\n  declaration -.-> note\n");
        return output;
    }
    for (i, node) in detail.graph.nodes.iter().enumerate() {
        let label = escape(&format!("{} · {}", node.operation, node.label));
        if matches!(
            node.operation.as_str(),
            "condition" | "pattern" | "short_circuit" | "try_branch"
        ) {
            output.push_str(&format!("  n{i}{{\"{label}\"}}\n"));
        } else {
            output.push_str(&format!("  n{i}[\"{label}\"]\n"));
        }
    }
    for edge in &detail.graph.edges {
        let from = detail
            .graph
            .nodes
            .iter()
            .position(|node| node.id == edge.from);
        let to = detail
            .graph
            .nodes
            .iter()
            .position(|node| node.id == edge.to);
        if let (Some(from), Some(to)) = (from, to) {
            let label = escape(if edge.label.is_empty() {
                &edge.kind
            } else {
                &edge.label
            });
            output.push_str(&format!("  n{from} -->|\"{label}\"| n{to}\n"));
        }
    }
    output.push_str("  classDef unknown fill:#30271b,stroke:#b78d46,color:#eee3cf\n");
    for (i, node) in detail.graph.nodes.iter().enumerate() {
        if node.operation == "unsupported" || node.resolution == "unknown" {
            output.push_str(&format!("  class n{i} unknown\n"));
        }
    }
    output
}
pub fn render_lld_markdown(detail: &LldFunctionDetail) -> String {
    let mut output = format!(
        "# {}\n\n{}\n\n- Entry: `{}`\n- Catalog: `{}`\n- HLD parent: `{}` / `{}` ({})\n- Source: `{}` lines {}–{} · blob `{}`\n- Behavior fingerprint: `{}`\n- Runtime validation: unavailable\n\n```mermaid\n{}```\n\n",
        detail.entry.name,
        detail.label,
        detail.entry.id,
        detail.catalog_id,
        detail.entry.parent.hld_design_id,
        detail
            .entry
            .parent
            .component_id
            .as_deref()
            .unwrap_or("unmapped"),
        detail.entry.parent.state,
        detail.entry.path,
        detail.entry.start_line,
        detail.entry.end_line,
        detail.entry.blob,
        detail.behavior_fingerprint,
        detail.mermaid
    );
    output.push_str("Call expressions and assignment syntax do not establish network, database, transaction, durable state, or business effects.\n\n");
    if !detail.coverage.gaps.is_empty() {
        output.push_str("## Coverage gaps\n\n");
        for gap in &detail.coverage.gaps {
            output.push_str("- ");
            output.push_str(gap);
            output.push('\n');
        }
    }
    output
}
