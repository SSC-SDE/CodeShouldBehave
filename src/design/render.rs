use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn markdown_escape(text: &str) -> String {
    let mut output = String::new();
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '`' => output.push_str("&#96;"),
            '[' => output.push_str("&#91;"),
            ']' => output.push_str("&#93;"),
            '\\' => output.push_str("&#92;"),
            '*' => output.push_str("&#42;"),
            '_' => output.push_str("&#95;"),
            '|' => output.push_str("&#124;"),
            '#' => output.push_str("&#35;"),
            c if c.is_control() => output.push(' '),
            c => output.push(c),
        }
    }
    output
}
fn mermaid_escape(text: &str) -> String {
    text.chars()
        .take(140)
        .map(|character| match character {
            '&' => "&amp;".into(),
            '"' => "&quot;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '[' => "&#91;".into(),
            ']' => "&#93;".into(),
            '{' => "&#123;".into(),
            '}' => "&#125;".into(),
            '`' => "&#96;".into(),
            c if c.is_control() => " ".into(),
            c => c.to_string(),
        })
        .collect()
}
pub(crate) fn node_id(id: &str) -> String {
    format!("n{}", digest(&[id]))
}
fn visible(element: &DesignElement) -> bool {
    matches!(
        element.kind,
        DesignElementKind::Component
            | DesignElementKind::Dependency
            | DesignElementKind::Deployment
            | DesignElementKind::Schema
    )
}
pub fn render_hld_mermaid(version: &DesignVersion) -> String {
    let mut output = String::from("flowchart LR\n");
    let nodes = version
        .elements
        .iter()
        .filter(|element| visible(element))
        .take(200)
        .collect::<Vec<_>>();
    let ids = nodes
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    for element in nodes {
        output.push_str(&format!(
            "  {}[\"{}\"]\n",
            node_id(&element.id),
            mermaid_escape(&element.name)
        ));
    }
    for relation in version
        .relations
        .iter()
        .filter(|item| ids.contains(item.from.as_str()) && ids.contains(item.to.as_str()))
    {
        output.push_str(&format!(
            "  {} -->|\"{}\"| {}\n",
            node_id(&relation.from),
            mermaid_escape(&relation.kind.replace('_', " ")),
            node_id(&relation.to)
        ));
    }
    if version
        .elements
        .iter()
        .filter(|element| visible(element))
        .count()
        > 200
    {
        output.push_str(
            "  omitted[\"Diagram limited to 200 elements; inspect the authorized inventory\"]\n",
        );
    }
    output
}
pub fn render_hld_markdown(version: &DesignVersion) -> String {
    let mut output = format!(
        "# High-Level Design\n\nCommit: {}  \nDesign: {}  \nGenerator: {}  \nScope: {}  \nState: {:?}\n\nThe static profile is {}. User assertions remain separate from source observations. This is not runtime verification.\n\n```mermaid\n{}```\n",
        markdown_escape(version.snapshot.as_str()),
        markdown_escape(&version.id),
        markdown_escape(&version.generator_version),
        markdown_escape(version.scope_version.as_str()),
        version.review_state,
        markdown_escape(&version.coverage.profile),
        version.mermaid
    );
    for section in &version.sections {
        output.push_str(&format!("\n## {}\n\n", markdown_escape(&section.title)));
        for id in &section.element_ids {
            if let Some(element) = version.elements.iter().find(|element| &element.id == id) {
                output.push_str(&format!("### {}\n\n", markdown_escape(&element.name)));
                if let Some(responsibility) = &element.responsibility {
                    output.push_str(&format!(
                        "User-confirmed role: {}\n\n",
                        markdown_escape(responsibility)
                    ));
                }
                for fact in &element.facts {
                    output.push_str(&format!(
                        "- {}: {} ({:?}; evidence {})\n",
                        markdown_escape(&fact.kind),
                        markdown_escape(&fact.value),
                        fact.resolution,
                        fact.evidence_ids
                            .iter()
                            .map(|id| markdown_escape(id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                for assertion in &element.user_assertions {
                    output.push_str(&format!(
                        "- User assertion: {}\n",
                        markdown_escape(assertion)
                    ));
                }
                for unknown in &element.unknowns {
                    output.push_str(&format!("- Unknown: {}\n", markdown_escape(unknown)));
                }
            }
        }
        for unknown in &section.unknowns {
            output.push_str(&format!("- Unknown: {}\n", markdown_escape(unknown)));
        }
        if section.id == "constraints" {
            for decision in &version.decision_records {
                output.push_str(&format!("\n### Selected document decision {}\n\nComponent: {}  \nReviewer intent: {}  \nVerification: requires reviewer verification; document text is not executed or treated as code facts.  \nEvidence: {}\n",
                    markdown_escape(&decision.id),markdown_escape(&decision.component_id),markdown_escape(&decision.intent),
                    decision.evidence_ids.iter().map(|id|markdown_escape(id)).collect::<Vec<_>>().join(", ")));
            }
        }
    }
    if !version.document_candidates.is_empty() {
        output.push_str("\n## Available document references\n\nThese are scoped file candidates, not selected architectural intent. Only an explicit reviewer document decision adds intent; document text remains untrusted source content.\n\n");
        for candidate in &version.document_candidates {
            output.push_str(&format!(
                "- {}: {} (evidence {})\n",
                markdown_escape(&candidate.id),
                markdown_escape(&candidate.title),
                markdown_escape(&candidate.evidence_id)
            ));
        }
    }
    output.push_str("\n## Source evidence\n\n");
    for evidence in &version.evidence {
        output.push_str(&format!(
            "- {}: {}:{}-{}, commit {}, blob {}, {} ({:?})\n",
            markdown_escape(&evidence.id),
            markdown_escape(&evidence.path),
            evidence.start_line,
            evidence.end_line,
            markdown_escape(evidence.commit.as_str()),
            markdown_escape(&evidence.blob),
            markdown_escape(&evidence.relationship_type),
            evidence.resolution
        ));
    }
    output
}
pub(crate) fn render_delta_mermaid(
    baseline: &DesignVersion,
    proposal: &DesignVersion,
    changes: &[DesignChange],
    impacts: &BTreeMap<String, BTreeSet<String>>,
) -> String {
    let mut elements = baseline
        .elements
        .iter()
        .chain(&proposal.elements)
        .filter(|element| visible(element))
        .map(|element| (element.id.as_str(), element))
        .collect::<BTreeMap<_, _>>();
    let classes = changes
        .iter()
        .filter_map(|change| {
            change
                .element_id
                .as_ref()
                .map(|id| (id.as_str(), change.kind.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut output = String::from("flowchart LR\n");
    let mut shown = BTreeSet::new();
    for (id, element) in elements.iter_mut().take(200) {
        shown.insert(*id);
        output.push_str(&format!(
            "  {}[\"{}\"]\n",
            node_id(id),
            mermaid_escape(&format!(
                "{}{}",
                if classes.get(id).copied() == Some("uncertain") {
                    "? "
                } else {
                    ""
                },
                element.name
            ))
        ));
        if let Some(kind) = classes.get(id) {
            output.push_str(&format!(
                "  class {} {}\n",
                node_id(id),
                if *kind == "removed" {
                    "removed"
                } else if *kind == "added" {
                    "added"
                } else if *kind == "uncertain" {
                    "uncertain"
                } else {
                    "modified"
                }
            ));
        } else if impacts.contains_key(*id) {
            output.push_str(&format!("  class {} impacted\n", node_id(id)));
        }
    }
    let mut relations = BTreeMap::new();
    for relation in baseline.relations.iter().chain(&proposal.relations) {
        relations.insert(&relation.id, relation);
    }
    for relation in relations
        .values()
        .filter(|item| shown.contains(item.from.as_str()) && shown.contains(item.to.as_str()))
    {
        let removed = changes.iter().any(|change| {
            change.kind == "removed" && change.relation_id.as_ref() == Some(&relation.id)
        });
        let uncertain = changes.iter().any(|change| {
            change.kind == "uncertain" && change.relation_id.as_ref() == Some(&relation.id)
        });
        output.push_str(&format!(
            "  {} {}|\"{}{}\"| {}\n",
            node_id(&relation.from),
            if removed || uncertain { "-.->" } else { "-->" },
            if removed {
                "removed: "
            } else if uncertain {
                "? unresolved change: "
            } else {
                ""
            },
            mermaid_escape(&relation.kind.replace('_', " ")),
            node_id(&relation.to)
        ));
    }
    output.push_str("  classDef added fill:#12382a,stroke:#55d99d,color:#f0f4fa\n  classDef removed fill:#44212a,stroke:#ef8392,color:#f0f4fa\n  classDef modified fill:#43351e,stroke:#e4bc66,color:#f0f4fa\n  classDef uncertain fill:#30213f,stroke:#b79bea,color:#f0f4fa\n  classDef impacted fill:#192b43,stroke:#9bbaff,color:#f0f4fa\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_labels_cannot_inject_mermaid_or_markdown() {
        let malicious = "x\"] --> evil[\"<script>alert(1)</script>```\nclick x \"javascript:evil\"";
        let escaped = mermaid_escape(malicious);
        assert!(!escaped.contains('\n'));
        assert!(!escaped.contains('"'));
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('`'));
        let markdown = markdown_escape(malicious);
        assert!(!markdown.contains('<'));
        assert!(!markdown.contains('`'));
        assert!(!markdown.contains('\n'));
    }
}
