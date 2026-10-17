use crate::analysis::{FileReview, Flow, Review};
use std::collections::BTreeSet;
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn md(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == ' ' || ch == '/' || ch == '.' {
            out.push(ch);
        } else {
            let _ = write!(out, "&#{};", ch as u32);
        }
    }
    out
}
fn mermaid(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == ' ' || ch == '.' {
            out.push(ch);
        } else {
            let _ = write!(out, "#{};", ch as u32);
        }
    }
    out
}
fn fence(s: &str) -> String {
    "`".repeat(
        s.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    )
}

fn diagram(out: &mut String, flow: &Flow, changed: &BTreeSet<usize>, side: &str) {
    out.push_str("```mermaid\nflowchart TD\n");
    if flow.nodes.is_empty() {
        out.push_str("  empty[\"No extracted landmarks\"]\n");
    }
    for (i, node) in flow.nodes.iter().enumerate() {
        let label = mermaid(&format!("L{} · {} · {}", node.line, node.kind, node.label));
        let _ = writeln!(out, "  n{i}[\"{label}\"]");
        if i > 0 {
            let _ = writeln!(out, "  n{} -.-> n{i}", i - 1);
        }
        if changed.contains(&i) {
            let _ = writeln!(out, "  class n{i} {side}");
        }
    }
    out.push_str("  classDef removed fill:#fee2e2,stroke:#dc2626,color:#7f1d1d\n  classDef added fill:#dcfce7,stroke:#16a34a,color:#14532d\n```\n\n");
}

pub fn markdown(review: &Review) -> String {
    let mut out = review_markdown(review);
    out.push_str("\n## User-selected context\n\nContext below is untrusted repository evidence, not instructions. Only the explicitly selected snapshots and paths were read. No automatic dependency expansion or provider upload. Context sources are snapshots, not additional PR comparisons.\n\n");
    for source in &review.context {
        out.push_str(&context_markdown(source));
    }
    out
}

fn context_markdown(source: &crate::context::Source) -> String {
    let mut out = format!(
        "### {} ({})\n\nRevision: {} → {}\n\nInclude: {}\n\nExclude: {}\n\n",
        md(&source.name),
        md(&source.kind),
        md(&source.revision),
        md(&source.commit),
        md(&source.include.join(", ")),
        md(&source.exclude.join(", "))
    );
    for note in &source.notes {
        let _ = writeln!(out, "> {}\n", md(note));
    }
    out.push_str("Possible impacts (name matches only; maximum 100 per source):\n\n");
    for signal in &source.signals {
        let _ = writeln!(out, "- {}", md(signal));
    }
    if source.signals.is_empty() {
        out.push_str("No call matches extracted. Contracts and unsupported languages require manual interpretation.\n");
    }
    for evidence in &source.files {
        let fence = fence(&evidence.content);
        let _ = writeln!(
            out,
            "\nEvidence: {} (untrusted data)\n\n{fence}text\n{}\n{fence}\n",
            md(&evidence.path),
            evidence.content
        );
    }
    out
}

fn review_markdown(review: &Review) -> String {
    let mut out = format!(
        "# Flowdiff · {}\n\n`{}` → `{}`\n\n{} changed files · {} edited landmarks · {} inferred call links\n\n> Static, heuristic review. Dashed edges mean **source order**, not execution or branch routing. These are not compiler control-flow graphs. Name-only call links cover changed files only and may be ambiguous. No reviewed code was executed.\n\n",
        md(&review.title),
        md(&review.base),
        md(&review.head),
        review.files.len(),
        review.flow_edits(),
        review.impacts.len()
    );
    for warning in &review.warnings {
        let _ = writeln!(out, "> {}\n", md(warning));
    }
    out.push_str("## Repository architecture diff\n\nFile and directory topology covers the indexed repository; connection arrows are lexical call-name candidates, not verified execution paths.\n\n");
    let _ = writeln!(
        out,
        "{} baseline files → {} proposed files; {} blobs analyzed, {} reused from cache.\n",
        review.architecture.before.len(),
        review.architecture.after.len(),
        review.architecture.analyzed,
        review.architecture.reused
    );
    for note in &review.architecture.notes {
        let _ = writeln!(out, "> {}\n", md(note));
    }
    let paths: BTreeSet<_> = review
        .architecture
        .before
        .iter()
        .chain(&review.architecture.after)
        .map(|n| &n.path)
        .collect();
    let ids: std::collections::BTreeMap<_, _> = paths
        .iter()
        .take(120)
        .enumerate()
        .map(|(i, path)| (*path, i))
        .collect();
    if !ids.is_empty() {
        out.push_str("```mermaid\nflowchart LR\n");
        for (path, i) in &ids {
            let before = review.architecture.before.iter().find(|n| &n.path == *path);
            let after = review.architecture.after.iter().find(|n| &n.path == *path);
            let state = match (before, after) {
                (None, _) => "added",
                (_, None) => "removed",
                (Some(b), Some(a)) if b.oid != a.oid => "modified",
                _ => "unchanged",
            };
            let _ = writeln!(out, "  a{i}[\"{}\"]\n  class a{i} {state}", mermaid(path));
        }
        for (from, to, name, status) in review
            .architecture
            .edges
            .iter()
            .filter(|(f, t, _, _)| ids.contains_key(f) && ids.contains_key(t))
            .take(180)
        {
            let _ = writeln!(
                out,
                "  a{} -.->|\"{}\"| a{}",
                ids[from],
                mermaid(&format!("{status}: {name}")),
                ids[to]
            );
        }
        out.push_str("  classDef added fill:#dcfce7,stroke:#16a34a\n  classDef removed fill:#fee2e2,stroke:#dc2626\n  classDef modified fill:#fef3c7,stroke:#d97706\n  classDef unchanged fill:#f8fafc,stroke:#94a3b8\n```\n\nMarkdown diagram shows up to 120 files and 180 candidate connections. Use the HTML explorer for drill-down.\n\n");
    }
    for (from, to, name, status) in &review.architecture.edges {
        if *status != "retained" {
            let _ = writeln!(
                out,
                "- **{status}**: {} → {} · {} (inferred)",
                md(from),
                md(to),
                md(name)
            );
        }
    }
    out.push_str("## Architectural signals\n\n");
    if review.impacts.is_empty() {
        out.push_str("No cross-file call candidates found within the changed files. This does not rule out architectural effects.\n\n");
    }
    for edge in &review.impacts {
        let _ = writeln!(
            out,
            "- **{}**: {} → {} — {} (inferred)",
            edge.change,
            md(&edge.from),
            md(&edge.to),
            md(&edge.reason)
        );
    }
    if !review.impacts.is_empty() {
        out.push_str("\n```mermaid\nflowchart LR\n");
        for (i, file) in review.files.iter().enumerate() {
            let _ = writeln!(out, "  f{i}[\"{}\"]", mermaid(&file.path));
        }
        for edge in &review.impacts {
            let Some(from) = review.files.iter().position(|f| f.path == edge.from) else {
                continue;
            };
            let Some(to) = review.files.iter().position(|f| f.path == edge.to) else {
                continue;
            };
            let _ = writeln!(
                out,
                "  f{from} -.->|\"{}\"| f{to}",
                mermaid(&format!("{}: {}", edge.change, edge.reason))
            );
        }
        out.push_str("```\n");
    }
    out.push('\n');
    for file in &review.files {
        let _ = writeln!(
            out,
            "## {}\n\nStatus: {} · +{} / −{} lines\n",
            md(&file.path),
            md(&file.status),
            file.additions,
            file.deletions
        );
        for note in &file.notes {
            let _ = writeln!(out, "> {}\n", md(note));
        }
        for dependency in file.before.imports.difference(&file.after.imports) {
            let _ = writeln!(out, "- Removed import: {}", md(dependency));
        }
        for dependency in file.after.imports.difference(&file.before.imports) {
            let _ = writeln!(out, "- Added import: {}", md(dependency));
        }
        out.push_str("\n### Before\n\n");
        diagram(&mut out, &file.before, &file.removed, "removed");
        out.push_str("### After\n\n");
        diagram(&mut out, &file.after, &file.added, "added");
        let fence = fence(&file.patch);
        let _ = writeln!(
            out,
            "<details><summary>Source diff</summary>\n\n{fence}diff\n{}{fence}\n\n</details>\n",
            file.patch
        );
    }
    out
}

fn json(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn json_set(set: &BTreeSet<String>) -> String {
    format!(
        "[{}]",
        set.iter().map(|s| json(s)).collect::<Vec<_>>().join(",")
    )
}
fn architecture_html(review: &Review) -> String {
    let arch = &review.architecture;
    let paths: BTreeSet<_> = arch
        .before
        .iter()
        .chain(&arch.after)
        .map(|n| &n.path)
        .collect();
    let mut nodes = vec![];
    for path in paths {
        let before = arch.before.iter().find(|n| &n.path == path);
        let after = arch.after.iter().find(|n| &n.path == path);
        let status = match (before, after) {
            (None, _) => "added",
            (_, None) => "removed",
            (Some(b), Some(a)) if b.oid != a.oid => "modified",
            _ => "unchanged",
        };
        let file = review.files.iter().find(|f| &f.path == path);
        let source_before = file.map(|f| f.before.source.chars().take(16000).collect::<String>());
        let source_after = file.map(|f| f.after.source.chars().take(16000).collect::<String>());
        let empty = BTreeSet::new();
        nodes.push(format!("{{\"path\":{},\"status\":{},\"definitionsBefore\":{},\"definitionsAfter\":{},\"callsBefore\":{},\"callsAfter\":{},\"sourceBefore\":{},\"sourceAfter\":{}}}",json(path),json(status),json_set(before.map(|n|&n.definitions).unwrap_or(&empty)),json_set(after.map(|n|&n.definitions).unwrap_or(&empty)),json_set(before.map(|n|&n.calls).unwrap_or(&empty)),json_set(after.map(|n|&n.calls).unwrap_or(&empty)),source_before.as_deref().map(json).unwrap_or("null".into()),source_after.as_deref().map(json).unwrap_or("null".into())));
    }
    let edges = arch
        .edges
        .iter()
        .map(|(from, to, name, status)| {
            format!(
                "{{\"from\":{},\"to\":{},\"name\":{},\"status\":{}}}",
                json(from),
                json(to),
                json(name),
                json(status)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let data = format!("{{\"nodes\":[{}],\"edges\":[{}]}}", nodes.join(","), edges);
    let mut out = String::from(
        "<nav class='design-level-nav' aria-label='Design levels'><a class='active' href='#structural-map'><strong>Structural map</strong><span>Available · repository topology and lexical call candidates</span></a><a href='#hld-status'><strong>HLD</strong><span>Unavailable · requires confirmed onboarding and Phase 4</span></a><a href='#lld-status'><strong>LLD</strong><span>Unavailable · requires accepted HLD and Phase 5</span></a></nav><div class='design-level-status'><article id='hld-status'><span class='change-badge'>HLD unavailable</span><h2>No verified high-level design exists for this comparison</h2><p>A Mermaid HLD diff must be derived from a user-confirmed architecture baseline. The current file tree and possible-call matches cannot establish services, boundaries, contracts, data ownership or deployment topology.</p></article><article id='lld-status'><span class='change-badge'>LLD unavailable</span><h2>No verified low-level design exists for this comparison</h2><p>LLD navigation requires an accepted HLD ownership map plus source-backed module, symbol, state and error-path analysis. The current source-order flow sketches are not compiler control-flow graphs.</p></article></div><div id='structural-map' class='architecture-workspace'><div class='architecture-toolbar'><div><h2>Structural repository map, before &amp; after</h2><p>Click a directory or file to inspect topology and possible lexical dependencies. This view is supporting evidence, not HLD or LLD.</p></div><label>Display<select id='arch-mode'><option value='diff'>Structural diff</option><option value='before'>Provided source</option><option value='after'>Proposed changes</option></select></label></div>",
    );
    let _ = write!(
        out,
        "<div class='architecture-revisions'><span>BASE <code>{}</code></span><span>→</span><span>PROPOSAL <code>{}</code></span><small>{} analyzed · {} cached</small></div>",
        esc(&review.base),
        esc(&review.head),
        arch.analyzed,
        arch.reused
    );
    for note in &arch.notes {
        let _ = write!(out, "<div class='notice'>{}</div>", esc(note));
    }
    out.push_str("<div class='architecture-split'><div class='architecture-map'><div class='architecture-map-tools'><button id='arch-up' class='export'>↑ Up</button><button id='arch-root' class='export'>⌂ Codebase</button><input id='arch-search' type='search' aria-label='Filter architecture paths' placeholder='Filter paths…'></div><nav id='arch-breadcrumb' aria-label='Architecture location'></nav><div class='architecture-legend'><span class='plus'>+ Added</span><span class='minus'>− Removed</span><span>~ Modified</span><span>○ Unchanged</span></div><div id='arch-canvas'><svg id='arch-wires' aria-hidden='true'></svg><div id='arch-boxes'></div></div><p id='arch-map-status' class='muted' aria-live='polite'></p><h3>Direction of change</h3><div id='arch-routes'></div></div><div class='architecture-inspector'><div id='arch-detail' aria-live='polite'><h3>Select a component</h3><p class='muted'>Zoom into a folder on the left, then choose a file. Inspect the baseline and proposed code here, along with incoming and outgoing paths.</p></div></div></div></div><details class='architecture-setup'><summary>Choose a stable commit or source PR for the next review</summary><p class='muted'>Prepare an exact comparison against a commit/ref or a locally fetched PR. This static report cannot fetch refs or launch Git; run the generated command to produce the new review. A merge-base comparison is optional in the CLI, not enabled here.</p><div class='scope-form'><label>Primary repository<input id='arch-repo' placeholder='/path/to/repository'></label><label>Cache directory<input id='arch-cache' value='.flowdiff-cache'></label><label>Baseline type<select id='arch-base-kind'><option value='ref'>Commit / branch / tag / local ref</option><option value='pr'>Local GitHub PR number</option></select></label><label>Baseline<input id='arch-base'></label><label>Proposal type<select id='arch-head-kind'><option value='ref'>Commit / branch / tag / local ref</option><option value='pr'>Local GitHub PR number</option></select></label><label>Proposal<input id='arch-head'></label><label>Optional saved context scope<input id='arch-scope' placeholder='/path/to/review.scope'></label></div><button class='export' id='arch-command-build'>Prepare commands</button><p id='arch-command-note' class='muted' role='status'>PR numbers map to refs/remotes/origin/pr/NUMBER; fetch these refs separately first.</p><pre id='arch-commands' class='scope-command'></pre></details>");
    let _ = write!(
        out,
        "<textarea id='arch-data' hidden>{}</textarea><textarea id='arch-base-value' hidden>{}</textarea><textarea id='arch-head-value' hidden>{}</textarea>",
        esc(&data),
        esc(&review.base),
        esc(&review.head)
    );
    out
}

fn flow_html(flow: &Flow, changed: &BTreeSet<usize>, side: &str) -> String {
    let mut out = String::new();
    if flow.nodes.is_empty() {
        out.push_str("<div class='empty'>No extracted landmarks.<small>The file may be empty, unavailable, or unsupported. Check the source diff and notes.</small></div>");
    }
    for (i, node) in flow.nodes.iter().enumerate() {
        let state = if changed.contains(&i) { side } else { "same" };
        let symbol = match node.kind {
            "function" => "ƒ",
            "branch" => "◇",
            "loop" => "↻",
            "exit" => "↗",
            "error" => "!",
            "await" => "◷",
            _ => "·",
        };
        let _ = write!(
            out,
            "<div class='landmark {state}'><div class='node-meta'><span class='node-kind'>{symbol} {}</span><span>L{} <b>{}</b></span></div><code>{}</code></div>",
            node.kind,
            node.line,
            if state == "added" {
                "+"
            } else if state == "removed" {
                "−"
            } else {
                ""
            },
            esc(&node.label)
        );
        if i + 1 < flow.nodes.len() {
            out.push_str("<div class='connector' aria-hidden='true'>↓</div>");
        }
    }
    out
}

fn imports(file: &FileReview) -> String {
    let mut out = String::new();
    for item in file.before.imports.difference(&file.after.imports) {
        let _ = write!(
            out,
            "<div class='import removed'><b>−</b><code>{}</code></div>",
            esc(item)
        );
    }
    for item in file.after.imports.difference(&file.before.imports) {
        let _ = write!(
            out,
            "<div class='import added'><b>+</b><code>{}</code></div>",
            esc(item)
        );
    }
    if out.is_empty() {
        out.push_str("<p class='muted'>No import changes extracted.</p>");
    }
    out
}

fn chart_html(flow: &Flow, changed: &BTreeSet<usize>, side: &str, prefix: &str) -> String {
    let mut out = String::new();
    if flow.nodes.is_empty() {
        return "<div class='empty'>No flow landmarks available. Check the source diff and file notes.</div>".into();
    }
    out.push_str("<div class='chart-terminal'>START OF SKETCH</div><div class='chart-edge' aria-hidden='true'>↓</div>");
    for (i, node) in flow.nodes.iter().enumerate() {
        let state = if changed.contains(&i) { side } else { "same" };
        let (icon, title) = match node.kind {
            "function" => ("ƒ", "Function entry"),
            "branch" => ("◇", "Condition / branch"),
            "loop" => ("↻", "Loop"),
            "exit" => ("↗", "Exit / return"),
            "error" => ("!", "Error handling"),
            "await" => ("◷", "Await result"),
            _ => ("□", "Process step"),
        };
        let next_function = flow
            .nodes
            .iter()
            .skip(i + 1)
            .find(|n| n.kind == "function")
            .map(|n| n.line)
            .unwrap_or(usize::MAX);
        let mut preview = String::new();
        let mut budget = 1200;
        let mut last = node.line;
        let mut clipped = false;
        for (line, content) in flow.source.lines().enumerate().skip(node.line - 1).take(6) {
            if line + 1 >= next_function || budget == 0 {
                break;
            }
            let text: String = content.chars().take(budget).collect();
            budget -= text.chars().count();
            clipped |= text.len() < content.len();
            last = line + 1;
            let _ = write!(
                preview,
                "<span class='preview-line {}'><span class='preview-number'>{}</span><code>{}{}</code></span>",
                if line + 1 == node.line {
                    "preview-current"
                } else {
                    ""
                },
                line + 1,
                esc(&text),
                if text.len() < content.len() {
                    "…"
                } else {
                    ""
                }
            );
        }
        let summary: String = node.label.chars().take(65).collect();
        let badge = match state {
            "added" => "+ Added",
            "removed" => "− Removed",
            _ => "Unchanged",
        };
        let _ = write!(
            out,
            "<div class='chart-node {state} chart-{}' data-chart-node><button class='chart-trigger' type='button' aria-expanded='false' aria-controls='{prefix}-{i}'><span class='chart-node-top'><span class='chart-symbol'>{icon}</span><strong>{title}</strong><span class='chart-badge'>{badge}</span></span><span class='chart-summary'>{}{}</span><span class='chart-hint'>L{} · Hover or focus to inspect · Click to pin</span></button><div class='chart-preview' id='{prefix}-{i}' hidden><div class='preview-heading'>SOURCE PREVIEW <span>L{}–{}{} · up to 6 lines</span></div><pre>{preview}</pre></div></div><div class='chart-edge' aria-hidden='true'>↓</div>",
            node.kind,
            esc(&summary),
            if summary.len() < node.label.len() {
                "…"
            } else {
                ""
            },
            node.line,
            node.line,
            last,
            if clipped { " · clipped" } else { "" }
        );
    }
    out.push_str("<div class='chart-terminal'>END OF SKETCH</div>");
    out
}

fn flowcharts_html(review: &Review) -> String {
    let mut out = String::from(
        "<section id='view-charts' role='tabpanel' aria-labelledby='tab-charts' hidden><div class='view-intro'><p>Explore the flow <span>Hover a box to reveal its source inside</span></p><div class='legend'><span><i class='green'></i>Added</span><span><i class='red'></i>Removed</span></div></div><div class='chart-guidance'>Follow the source-order arrows. Hover or Tab to inspect a node; click or press Enter to pin its code open. Escape closes it. Previews show up to six source lines, not necessarily an entire block.</div>",
    );
    for (i, file) in review.files.iter().enumerate() {
        let _ = write!(
            out,
            "<article class='file-panel' data-panel='{i}' {}><div class='file-heading'><code>{}</code><span>FLOWCHART EXPLORER</span></div>",
            if i == 0 { "" } else { "hidden" },
            esc(&file.path)
        );
        for note in &file.notes {
            let _ = write!(out, "<div class='notice'>{}</div>", esc(note));
        }
        let _ = write!(
            out,
            "<div class='flow-columns'><div class='flow-column'><div class='column-label'>BASE<span>Before changes</span></div><div class='chart-track'>{}</div></div><div class='flow-column after'><div class='column-label'>HEAD<span>After changes</span></div><div class='chart-track'>{}</div></div></div></article>",
            chart_html(
                &file.before,
                &file.removed,
                "removed",
                &format!("chart-{i}-base")
            ),
            chart_html(
                &file.after,
                &file.added,
                "added",
                &format!("chart-{i}-head")
            )
        );
    }
    if review.files.is_empty() {
        out.push_str("<div class='empty'>No changed files to chart.</div>");
    }
    out.push_str("<div class='analysis-note'><b>Source order, not execution</b> Symbols identify syntax landmarks. Arrows do not establish runtime branches, reachability, or call relationships.</div></section>");
    out
}

pub fn html(review: &Review) -> String {
    let mut out = String::from(
        "<!doctype html><html lang='en'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'none'; connect-src 'none'; font-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\"><title>Flowdiff — Behavioral review</title><style>",
    );
    out.push_str(include_str!("style.css"));
    out.push_str(include_str!("report-dark.css"));
    out.push_str("</style></head><body><header class='topbar'><a class='brand' href='#'><span class='brand-mark'>CSB</span>CodeShouldBehave<span class='beta'>LOCAL</span></a><div class='top-context'>CODE REVIEW<span>/</span>BEHAVIORAL DIFF</div><button id='export' class='export'>↓ Export Markdown</button></header><div class='layout'><aside><div class='repo-label'>WORKSPACE</div>");
    let _ = write!(
        out,
        "<h2 class='repo'>{}</h2><div class='commit-pair'><span>{}</span><i>→</i><span>{}</span></div>",
        esc(&review.title),
        esc(&review.base.chars().take(10).collect::<String>()),
        esc(&review.head.chars().take(10).collect::<String>())
    );
    out.push_str("<div class='side-divider'></div><label class='search'><span>⌕</span><input id='search' type='search' placeholder='Find a file…' aria-label='Filter files'></label>");
    let _ = write!(
        out,
        "<div class='files-label'>CHANGED FILES <span>{}</span></div><nav aria-label='Changed files'>",
        review.files.len()
    );
    for (i, file) in review.files.iter().enumerate() {
        let _ = write!(
            out,
            "<button class='file-button {}' data-file='{i}' aria-pressed='{}'><span class='file-icon'>{}</span><span class='file-path'>{}</span><span class='file-count'>+{} / −{}</span><span class='reviewed-mark' aria-label='Reviewed' hidden>✓</span></button>",
            if i == 0 { "selected" } else { "" },
            i == 0,
            esc(&file.status),
            esc(&file.path),
            file.additions,
            file.deletions
        );
    }
    out.push_str("</nav><p id='no-results' hidden>No matching files</p><div class='local-status'><span class='status-dot'></span> Offline by design<small>No telemetry · no code execution<br>Review progress stays in this tab</small></div></aside><main><div class='eyebrow'><span class='status-dot'></span> REVIEW WITH CONTEXT</div><div class='heading-row'><div><h1>Changes</h1><p class='subtitle'>Source, flow and architecture · one pinned comparison</p></div><span class='heuristic'>◈ Static analysis · heuristic</span></div><div class='stats'>");
    let _ = write!(
        out,
        "<div><strong>{:02}</strong><span>Files changed</span></div><div><strong>{:02}</strong><span>Flow landmarks edited</span></div><div><strong>{:02}</strong><span>Possible call links</span></div><div class='scope-stat'><span class='scope-icon'>⌁</span><span>Analysis scope<strong>PR + selected context</strong></span></div></div>",
        review.files.len(),
        review.flow_edits(),
        review.impacts.len()
    );
    for warning in &review.warnings {
        let _ = write!(out, "<div class='notice'>{}</div>", esc(warning));
    }
    let _ = write!(
        out,
        "<div class='review-identity'><span>BASELINE <code>{}</code></span><span>PROPOSAL <code>{}</code></span><span>Primary scope: repository snapshot · {} context sources</span></div>",
        esc(&review.base),
        esc(&review.head),
        review.context.len()
    );
    out.push_str("<div class='review-toolbar'><div><strong id='active-file'>Select a changed file</strong><output id='review-progress' aria-live='polite'></output></div><div class='review-actions'><button id='previous-file'>← Previous</button><button id='next-file'>Next →</button><button id='mark-reviewed' aria-pressed='false'>Mark reviewed</button><button id='open-summary' class='primary-action'>Review summary</button></div></div>");
    out.push_str("<div class='tabs' role='tablist' aria-label='Review views'><button class='tab active' id='tab-source' role='tab' aria-controls='view-source' aria-selected='true' data-view='source'>Source diff</button><button class='tab' id='tab-architecture' role='tab' aria-controls='view-architecture' aria-selected='false' data-view='architecture'>Structure / HLD / LLD</button><button class='tab' id='tab-charts' role='tab' aria-controls='view-charts' aria-selected='false' data-view='charts'>Flowcharts</button><button class='tab' id='tab-flow' role='tab' aria-controls='view-flow' aria-selected='false' data-view='flow'>Landmarks</button><button class='tab' id='tab-context' role='tab' aria-controls='view-context' aria-selected='false' data-view='context'>Context</button><button class='tab' id='tab-summary' role='tab' aria-controls='view-summary' aria-selected='false' data-view='summary'>Summary</button></div><section id='view-flow' role='tabpanel' aria-labelledby='tab-flow' hidden><div class='view-intro'><p>Before & after <span>Source-order flow sketches</span></p><div class='legend'><span><i class='green'></i>Added</span><span><i class='red'></i>Removed</span><span><i></i>Unchanged</span></div></div>");
    for (i, file) in review.files.iter().enumerate() {
        let _ = write!(
            out,
            "<article class='file-panel' data-panel='{i}' {}><div class='file-heading'><code>{}</code><span><b class='plus'>+{}</b><b class='minus'>−{}</b></span></div>",
            if i == 0 { "" } else { "hidden" },
            esc(&file.path),
            file.additions,
            file.deletions
        );
        for note in &file.notes {
            let _ = write!(out, "<div class='notice'>{}</div>", esc(note));
        }
        let _ = write!(
            out,
            "<div class='flow-columns'><div class='flow-column'><div class='column-label'><span class='revision-dot'></span>BASE<span>{} removed</span></div><div class='flow-track'>{}</div></div><div class='flow-column after'><div class='column-label'><span class='revision-dot head'></span>HEAD<span>{} added</span></div><div class='flow-track'>{}</div></div></div></article>",
            file.removed.len(),
            flow_html(&file.before, &file.removed, "removed"),
            file.added.len(),
            flow_html(&file.after, &file.added, "added")
        );
    }
    if review.files.is_empty() {
        out.push_str("<div class='empty'>No changed files between these revisions.</div>");
    }
    out.push_str("<div class='analysis-note'><b>How to read this</b> Arrows connect landmarks in source order, not runtime paths. Branches, loops, exits and error propagation are highlighted from syntax. Validate behavior in the source diff; these sketches do not prove execution paths.</div></section><section id='view-architecture' role='tabpanel' aria-labelledby='tab-architecture' hidden>");
    out.push_str(&architecture_html(review));
    out.push_str("<div class='view-intro'><p>Architectural signals <span>Possible connections across changed files</span></p></div><div class='architecture-summary'><span class='architecture-icon'>◇</span><div><h2>A change can travel.</h2><p>These links match call names to definitions. They are candidates to investigate, not resolved dependencies. Additional scoped repositories, libraries and service contracts appear in the Context tab. Transitive effects are not automatically expanded.</p></div></div><div class='impact-list'>");
    if review.impacts.is_empty() {
        out.push_str("<div class='empty'>No cross-file call candidates found.<small>This does not rule out architectural effects.</small></div>");
    }
    for edge in &review.impacts {
        let _ = write!(
            out,
            "<div class='impact'><span class='change-badge {}'>{}</span><div><div class='impact-route'><code>{}</code><span>→</span><code>{}</code></div><small>{} · name match, unverified</small></div></div>",
            edge.change,
            edge.change,
            esc(&edge.from),
            esc(&edge.to),
            esc(&edge.reason)
        );
    }
    out.push_str("</div><h2 class='section-heading'>Import changes</h2>");
    for file in &review.files {
        let _ = write!(
            out,
            "<div class='import-card'><h3>{}</h3>{}</div>",
            esc(&file.path),
            imports(file)
        );
    }
    out.push_str("</section><section id='view-source' role='tabpanel' aria-labelledby='tab-source'><div class='view-intro'><p>The source of truth <span>Unified Git diff</span></p><label class='diff-filter'><input type='checkbox' id='diff-changes-only'> Changes only</label></div><p id='source-unavailable' class='notice' hidden>This context file is outside the detailed changed-file diff. Inspect its indexed symbols in Architecture, or select a changed file.</p>");
    for (i, file) in review.files.iter().enumerate() {
        let _ = write!(
            out,
            "<article class='file-panel source-panel' data-panel='{i}' {}><div class='file-heading'><code>{}</code><span><b class='plus'>+{}</b><b class='minus'>−{}</b></span></div>",
            if i == 0 { "" } else { "hidden" },
            esc(&file.path),
            file.additions,
            file.deletions
        );
        for note in &file.notes {
            let _ = write!(out, "<div class='notice'>{}</div>", esc(note));
        }
        out.push_str("<pre class='diff'>");
        for line in file.patch.lines() {
            let class = if line.starts_with("+++") || line.starts_with("---") {
                "diff-header"
            } else if line.starts_with('+') {
                "diff-add"
            } else if line.starts_with('-') {
                "diff-remove"
            } else if line.starts_with("@@") {
                "diff-hunk"
            } else {
                ""
            };
            let _ = writeln!(out, "<span class='{class}'>{}</span>", esc(line));
        }
        out.push_str("</pre></article>");
    }
    out.push_str("</section>");
    out.push_str(&context_html(review));
    out.push_str(&flowcharts_html(review));
    out.push_str(&observation_html(review));
    out.push_str("<footer>FLOWDIFF <span>Local evidence. Human judgment.</span><span>v0.1.0</span></footer></main></div><textarea id='markdown' hidden>");
    out.push_str(&esc(&review_markdown(review)));
    out.push_str("</textarea><script>");
    out.push_str(include_str!("ui.js"));
    out.push_str("</script></body></html>");
    out
}

fn observation_html(review: &Review) -> String {
    let mut out = String::from(
        "<section id='view-summary' role='tabpanel' aria-labelledby='tab-summary' hidden><div class='view-intro'><p>Review summary <span>Local observation record</span></p></div><p>Record what you verified. Export saves a JSON file; it does not approve a GitHub PR or change rollout controls. Progress and notes reset when this report is reloaded.</p><div class='summary-progress-actions'><output id='summary-progress' aria-live='polite'></output><button id='mark-all-reviewed' type='button'>Mark all files reviewed</button></div><p class='muted'>Bulk marking updates local inspection progress only. Acceptance still requires reviewer identity and explicit comparison, scope and coverage checks.</p><ul id='unreviewed-files'></ul><div class='scope-form'><label>Reviewer<input id='observation-reviewer' autocomplete='off' placeholder='Your name'></label><label>PR / review reference (optional)<input id='observation-reference' placeholder='e.g. SSC-SDE/codeshouldbehave#28'></label>",
    );
    for (id, label) in [
        (
            "identity",
            "Baseline and proposal match the intended comparison",
        ),
        (
            "scope",
            "Visible evidence stays within the authorized scope",
        ),
        (
            "coverage",
            "Expected changes are present or missing coverage is disclosed",
        ),
    ] {
        let _ = write!(
            out,
            "<label>{label}<select id='check-{id}'><option value='unknown'>Not checked</option><option value='yes'>Verified</option><option value='no'>Issue found</option></select></label>"
        );
    }
    out.push_str("</div><p class='muted'>This report is an immutable snapshot. Live publication freshness is not tested here. Missing flow landmarks are not proof of missing source changes.</p><label class='observation-notes'>Findings, unresolved questions and coverage limits<textarea id='observation-notes' rows='4' placeholder='Describe what you found. Leave unverified claims unresolved.'></textarea></label><div class='review-actions'><button id='observation-accept' class='primary-action' disabled>Accept observation &amp; export</button><button id='observation-stop'>Record blocker &amp; export</button></div><p id='observation-status' role='status'></p><h3>Analysis notes</h3>");
    for note in review.warnings.iter().chain(&review.architecture.notes) {
        let _ = write!(out, "<p class='notice'>{}</p>", esc(note));
    }
    for file in &review.files {
        for note in &file.notes {
            let _ = write!(
                out,
                "<p class='notice'><code>{}</code>: {}</p>",
                esc(&file.path),
                esc(note)
            );
        }
    }
    if review.files.is_empty() {
        out.push_str("<p class='notice'>No changed files in this report. Acceptance is unavailable; inspect the comparison or record a blocker.</p>");
    }
    let data = format!(
        "{{\"repository\":{},\"baseline\":{},\"proposal\":{},\"files\":[{}],\"warnings\":[{}]}}",
        json(&review.title),
        json(&review.base),
        json(&review.head),
        review
            .files
            .iter()
            .map(|f| json(&f.path))
            .collect::<Vec<_>>()
            .join(","),
        review
            .warnings
            .iter()
            .chain(&review.architecture.notes)
            .chain(review.files.iter().flat_map(|f| f.notes.iter()))
            .map(|n| json(n))
            .collect::<Vec<_>>()
            .join(",")
    );
    let _ = write!(
        out,
        "<textarea id='observation-data' hidden>{}</textarea></section>",
        esc(&data)
    );
    out
}

fn context_html(review: &Review) -> String {
    let mut out = String::from(
        "<section id='view-context' role='tabpanel' aria-labelledby='tab-context' hidden><div class='view-intro'><p>You set the boundary <span>Repositories, libraries and service contracts</span></p><span id='context-count' aria-live='polite'></span></div><div class='architecture-summary'><span class='architecture-icon'>◎</span><div><h2>Context is a choice.</h2><p>Select loaded sources for the Markdown export. New sources must be explicitly scoped and loaded by the CLI. No auto-discovery, network calls, dependency installation or AI upload.</p></div></div><div class='notice'>Unchecking a source removes it from the Markdown export, not from this existing HTML file. Regenerate with a narrower scope before sharing the HTML. Sources are context snapshots, not separate PR comparisons.</div>",
    );
    if review.context.is_empty() {
        out.push_str("<p class='muted'>No additional context loaded. Build a scope below, then run a review with --context review.scope.</p>");
    }
    for (i, source) in review.context.iter().enumerate() {
        let _ = write!(
            out,
            "<article class='context-card'><label class='context-choice'><input type='checkbox' data-context='{i}' checked><span><strong>{}</strong><small>{} · {} evidence files</small></span><span class='change-badge'>LOADED</span></label><div class='context-body' data-context-body='{i}'><div class='scope-details'><div><b>Snapshot</b><code>{} → {}</code></div><div><b>Include</b><code>{}</code></div><div><b>Exclude</b><code>{}</code></div></div>",
            esc(&source.name),
            esc(&source.kind),
            source.files.len(),
            esc(&source.revision),
            esc(&source.commit),
            esc(&source.include.join(", ")),
            esc(&source.exclude.join(", "))
        );
        for note in &source.notes {
            let _ = write!(out, "<p class='muted'>{}</p>", esc(note));
        }
        out.push_str("<h3>Possible impacts <small>name matches · up to 100</small></h3>");
        for signal in &source.signals {
            let _ = write!(out, "<p class='context-signal'>{}</p>", esc(signal));
        }
        if source.signals.is_empty() {
            out.push_str("<p class='muted'>No call matches extracted. Review contracts and other evidence manually.</p>");
        }
        for evidence in &source.files {
            let _ = write!(
                out,
                "<details class='evidence'><summary>{} · untrusted evidence</summary><pre>{}</pre></details>",
                esc(&evidence.path),
                esc(&evidence.content)
            );
        }
        let row = format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            source.kind,
            source.name,
            source.repo,
            source.commit,
            source.include.join(","),
            if source.exclude.is_empty() {
                "-".into()
            } else {
                source.exclude.join(",")
            }
        );
        let _ = write!(
            out,
            "</div><textarea id='context-md-{i}' hidden>{}</textarea><textarea id='context-scope-{i}' hidden>{}</textarea></article>",
            esc(&context_markdown(source)),
            esc(&row)
        );
    }
    out.push_str("<div class='scope-builder'><h2>Add context to the next review</h2><p class='muted'>Use a local Git repository containing the selected source or service contracts. Relative repository paths resolve from the saved scope file. Include literal files or directories, separated by commas. Use . only to explicitly include the whole repository.</p><div class='scope-form'><label>Source type<select id='scope-kind'><option value='repository'>Repository</option><option value='internal-library'>Internal library</option><option value='external-library'>External library</option><option value='service'>Third-party / internal service</option></select></label><label>Source name<input id='scope-name' placeholder='e.g. Orders API'></label><label>Local Git repository<input id='scope-repo' placeholder='/path/to/orders-api'></label><label>Revision or version<input id='scope-revision' placeholder='e.g. v2.1.0 or commit SHA'></label><label>Included paths<input id='scope-include' placeholder='src/api/,contracts/'></label><label>Excluded paths<input id='scope-exclude' placeholder='src/api/private/'></label></div><div class='scope-actions'><button id='add-context' class='export'>+ Add scoped source</button><button id='save-scope' class='export'>↓ Save review.scope</button></div><p id='scope-feedback' role='status' class='muted'></p><div id='pending-sources'></div><p class='muted'>Save includes checked loaded sources and pending sources below. Pending sources do not enter the current Markdown export. Load them with:</p><pre class='scope-command'>flowdiff review --repo /path/to/main-repo --base main --head HEAD --merge-base --context /path/to/review.scope --output new-review.html</pre></div></section>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chart_preview_escapes_source_and_stops_at_next_function() {
        let flow = crate::analysis::analyze(
            "a.rs",
            "fn first() {\n    let text = \"<script>alert(1)</script>\";\n}\nfn second() {\n    other();\n}",
        );
        let chart = chart_html(&flow, &BTreeSet::new(), "added", "test");
        let first = chart
            .split("id='test-0'")
            .nth(1)
            .unwrap()
            .split("</pre>")
            .next()
            .unwrap();
        assert!(first.contains("&lt;script&gt;"));
        assert!(!first.contains("fn second"));
        assert!(first.contains("L1–3"));
        assert!(chart.contains("aria-controls='test-0'"));
        assert!(!chart.contains("<script>"));
    }
    #[test]
    fn escapes_untrusted_content() {
        let mut review = crate::analysis::demo();
        review.files[0].path = "</textarea><script>alert(1)</script>".into();
        review.files[0].patch = "</pre><img src=x onerror=alert(1)>\n".into();
        let page = html(&review);
        assert!(!page.contains("<script>alert(1)"));
        assert!(!page.contains("<img src=x"));
        assert!(page.contains("&lt;img src=x"));
    }
    #[test]
    fn fences_cannot_be_closed_by_source() {
        assert_eq!(fence("```\nmalicious"), "````");
        assert!(!mermaid("\"]\nclick n0 \"javascript:alert(1)\"").contains('"'));
    }
}
