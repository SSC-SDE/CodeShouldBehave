use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: &'static str,
    pub label: String,
    identity: String,
    pub line: usize,
}
#[derive(Clone, Default)]
pub struct Flow {
    pub source: String,
    pub nodes: Vec<Node>,
    pub definitions: BTreeSet<String>,
    pub calls: BTreeSet<String>,
    pub imports: BTreeSet<String>,
    pub supported: bool,
    pub truncated: bool,
}
pub struct FileReview {
    pub path: String,
    pub status: String,
    pub before: Flow,
    pub after: Flow,
    pub removed: BTreeSet<usize>,
    pub added: BTreeSet<usize>,
    pub patch: String,
    pub notes: Vec<String>,
    pub additions: usize,
    pub deletions: usize,
}
pub struct Impact {
    pub from: String,
    pub to: String,
    pub reason: String,
    pub change: &'static str,
}
pub struct Review {
    pub architecture: crate::architecture::Architecture,
    pub change_set: Option<crate::incremental::ChangeSet>,
    pub context: Vec<crate::context::Source>,
    pub title: String,
    pub base: String,
    pub head: String,
    pub files: Vec<FileReview>,
    pub impacts: Vec<Impact>,
    pub warnings: Vec<String>,
}
impl Review {
    pub fn flow_edits(&self) -> usize {
        self.files
            .iter()
            .map(|f| f.added.len() + f.removed.len())
            .sum()
    }
}
impl FileReview {
    pub fn new(path: String, status: String, before: Flow, after: Flow, patch: String) -> Self {
        let (removed, added) = align(&before.nodes, &after.nodes);
        let hunk_lines: Vec<_> = patch.lines().skip_while(|l| !l.starts_with("@@")).collect();
        let additions = hunk_lines.iter().filter(|l| l.starts_with('+')).count();
        let deletions = hunk_lines.iter().filter(|l| l.starts_with('-')).count();
        let mut notes = vec![];
        if !before.supported && !after.supported {
            notes.push("Flow extraction supports Rust, Python, JavaScript and TypeScript. Inspect this file in the source diff.".into());
        }
        if before.truncated || after.truncated {
            notes.push("Flow sketch limited to the first 180 landmarks per revision.".into());
        }
        if before
            .nodes
            .iter()
            .chain(&after.nodes)
            .any(|n| n.identity.chars().count() > 160)
        {
            notes.push("Long labels are shortened with …; comparison uses the full line. Read the source diff for complete text.".into());
        }
        Self {
            path,
            status,
            before,
            after,
            removed,
            added,
            patch,
            notes,
            additions,
            deletions,
        }
    }
}

// These are source-order landmarks, not a compiler control-flow graph.
// Keep the comparison bounded and align repeated statements with an LCS.
fn align(before: &[Node], after: &[Node]) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let mut dp = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    let same = |i: usize, j: usize| {
        before[i].kind == after[j].kind && before[i].identity == after[j].identity
    };
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            dp[i][j] = if same(i, j) {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut removed = (0..before.len()).collect::<BTreeSet<_>>();
    let mut added = (0..after.len()).collect::<BTreeSet<_>>();
    let (mut i, mut j) = (0, 0);
    while i < before.len() && j < after.len() {
        if same(i, j) {
            removed.remove(&i);
            added.remove(&j);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (removed, added)
}

#[derive(Default)]
struct Mask {
    block: usize,
    quote: Option<char>,
    triple: bool,
}
impl Mask {
    fn line(&mut self, line: &str, python: bool) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if self.block > 0 {
                if c == '*' && next == Some('/') {
                    self.block -= 1;
                    i += 2;
                } else if c == '/' && next == Some('*') {
                    self.block += 1;
                    i += 2;
                } else {
                    i += 1;
                }
                out.push(' ');
                continue;
            }
            if let Some(q) = self.quote {
                if c == '\\' {
                    i = (i + 2).min(chars.len());
                    out.push(' ');
                    continue;
                }
                if c == q && (!self.triple || (next == Some(q) && chars.get(i + 2) == Some(&q))) {
                    i += if self.triple { 3 } else { 1 };
                    self.quote = None;
                    self.triple = false;
                } else {
                    i += 1;
                }
                out.push(' ');
                continue;
            }
            if (python && c == '#') || (!python && c == '/' && next == Some('/')) {
                break;
            }
            if !python && c == '/' && next == Some('*') {
                self.block = 1;
                i += 2;
                out.push(' ');
                continue;
            }
            // Do not interpret Rust lifetimes as character literals.
            let single = c == '\'' && (python || chars[i + 1..].contains(&'\''));
            if c == '"' || c == '`' || single {
                self.quote = Some(c);
                self.triple = python && next == Some(c) && chars.get(i + 2) == Some(&c);
                i += if self.triple { 3 } else { 1 };
                out.push(' ');
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }
}

fn words(code: &str) -> Vec<&str> {
    code.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect()
}
fn short(text: &str) -> String {
    let clean = text.trim().to_owned();
    if clean.chars().count() > 160 {
        format!("{}…", clean.chars().take(157).collect::<String>())
    } else {
        clean
    }
}

pub fn analyze(path: &str, source: &str) -> Flow {
    let ext = path.rsplit('.').next().unwrap_or("");
    let supported = matches!(
        ext,
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs"
    );
    let mut flow = Flow {
        supported,
        source: source.to_owned(),
        ..Flow::default()
    };
    if !supported {
        return flow;
    }
    let mut mask = Mask::default();
    for (number, original) in source.lines().enumerate() {
        let masked = mask.line(original, ext == "py");
        let code = masked.trim();
        let tokens = words(code);
        if tokens.is_empty() {
            continue;
        }
        let import = matches!(tokens[0], "use" | "import" | "from" | "mod" | "require")
            || code.contains("require(");
        if import {
            flow.imports.insert(original.trim().to_owned());
            continue;
        }
        let mut definition = None;
        for (index, token) in tokens.iter().enumerate() {
            if matches!(*token, "fn" | "function" | "def") {
                definition = tokens.get(index + 1).copied();
                break;
            }
        }
        if definition.is_none()
            && code.contains("=>")
            && matches!(tokens[0], "const" | "let" | "export")
        {
            let offset = usize::from(tokens[0] == "export");
            if matches!(tokens.get(offset), Some(&"const") | Some(&"let")) {
                definition = tokens.get(offset + 1).copied();
            }
        }
        if let Some(name) = definition {
            flow.definitions.insert(name.into());
        }
        // Calls are lexical candidates only. No dynamic dispatch/name resolution.
        for (index, ch) in code.char_indices() {
            if ch == '(' {
                let prefix = code[..index].trim_end();
                let name = prefix
                    .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                if !name.is_empty()
                    && Some(name) != definition
                    && !matches!(
                        name,
                        "if" | "while"
                            | "for"
                            | "match"
                            | "switch"
                            | "catch"
                            | "fn"
                            | "function"
                            | "def"
                            | "sizeof"
                    )
                {
                    flow.calls.insert(name.into());
                }
            }
        }
        let has = |word: &str| tokens.contains(&word);
        let kind = if definition.is_some() {
            "function"
        } else if has("if") || has("else") || has("match") || has("switch") || has("case") {
            "branch"
        } else if has("for") || has("while") || has("loop") {
            "loop"
        } else if has("return") || has("break") || has("continue") {
            "exit"
        } else if has("throw") || has("raise") || has("catch") || code.contains('?') {
            "error"
        } else if has("await") {
            "await"
        } else {
            "step"
        };
        if flow.nodes.len() < 180 {
            flow.nodes.push(Node {
                kind,
                label: short(original),
                identity: original.trim().to_owned(),
                line: number + 1,
            });
        } else {
            flow.truncated = true;
        }
    }
    flow
}

pub fn impacts(files: &[FileReview]) -> Vec<Impact> {
    let mut result = vec![];
    for caller in files {
        for target in files {
            if caller.path == target.path {
                continue;
            }
            let old: BTreeSet<_> = caller
                .before
                .calls
                .intersection(&target.before.definitions)
                .cloned()
                .collect();
            let new: BTreeSet<_> = caller
                .after
                .calls
                .intersection(&target.after.definitions)
                .cloned()
                .collect();
            for name in old.union(&new).take(20) {
                let change = if !old.contains(name) {
                    "added"
                } else if !new.contains(name) {
                    "removed"
                } else {
                    "retained"
                };
                result.push(Impact {
                    from: caller.path.clone(),
                    to: target.path.clone(),
                    reason: format!("possible call to {name}()"),
                    change,
                });
            }
        }
    }
    result.truncate(300);
    result
}

pub fn demo() -> Review {
    let before = include_str!("../fixtures/demo/checkout.before.rs");
    let after = include_str!("../fixtures/demo/checkout.after.rs");
    let payment_before =
        "pub fn charge(amount: u64) -> Result<Receipt, Error> {\n    gateway.capture(amount)\n}\n";
    let payment_after = "pub fn authorize(amount: u64) -> Result<Hold, Error> {\n    gateway.authorize(amount)\n}\n\npub fn capture(hold: Hold) -> Result<Receipt, Error> {\n    gateway.capture(hold)\n}\n";
    let pairs = [
        ("src/checkout.rs", before, after),
        ("src/payments.rs", payment_before, payment_after),
    ];
    let files: Vec<_> = pairs
        .into_iter()
        .map(|(path, old, new)| {
            let patch = format!(
                "--- a/{path}\n+++ b/{path}\n@@ illustrative demo (full replacement) @@\n{}{}",
                old.lines().map(|l| format!("-{l}\n")).collect::<String>(),
                new.lines().map(|l| format!("+{l}\n")).collect::<String>()
            );
            FileReview::new(
                path.into(),
                "M".into(),
                analyze(path, old),
                analyze(path, new),
                patch,
            )
        })
        .collect();
    let impacts = impacts(&files);
    Review {
        architecture: crate::architecture::demo(&files),
        change_set: None,
        context: crate::context::demo(&files),
        title: "checkout-service".into(),
        base: "demo-before".into(),
        head: "demo-after".into(),
        files,
        impacts,
        warnings: vec!["Demo data. Source diffs show illustrative full replacements.".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligns_insertions_without_marking_later_lines() {
        let old = analyze("a.rs", "fn go() {\nfoo();\nbar();\n}");
        let new = analyze("a.rs", "fn go() {\ncheck();\nfoo();\nbar();\n}");
        let (removed, added) = align(&old.nodes, &new.nodes);
        assert!(removed.is_empty());
        assert_eq!(added, BTreeSet::from([1]));
    }
    #[test]
    fn ignores_comments_and_string_calls() {
        let flow = analyze(
            "a.rs",
            "// evil()\n/* hidden() */\nfn go() {\nlet text = \"fake()\";\nreal();\n}",
        );
        assert_eq!(flow.calls, BTreeSet::from(["real".into()]));
    }
    #[test]
    fn detects_removed_calls_and_new_branches() {
        let review = demo();
        assert!(
            review
                .impacts
                .iter()
                .any(|e| e.change == "removed" && e.reason.contains("charge"))
        );
        assert!(
            review
                .impacts
                .iter()
                .any(|e| e.change == "added" && e.reason.contains("authorize"))
        );
        assert!(
            review.files[0]
                .after
                .nodes
                .iter()
                .any(|n| n.kind == "branch")
        );
    }
    #[test]
    fn masks_python_docstrings() {
        let flow = analyze(
            "a.py",
            "\"\"\"\nphantom()\n\"\"\"\ndef run():\n    real()\n",
        );
        assert_eq!(flow.calls, BTreeSet::from(["real".into()]));
        assert!(flow.definitions.contains("run"));
    }
    #[test]
    fn bounded_landmarks() {
        let flow = analyze("a.rs", &"work();\n".repeat(1000));
        assert_eq!(flow.nodes.len(), 180);
        assert!(flow.truncated);
    }
    #[test]
    fn detects_changes_beyond_visible_label_and_inside_strings() {
        let prefix = "x".repeat(200);
        let old = analyze("a.rs", &format!("let value = \"{prefix}old\";"));
        let new = analyze("a.rs", &format!("let value = \"{prefix}new\";"));
        let (removed, added) = align(&old.nodes, &new.nodes);
        assert_eq!(removed.len(), 1);
        assert_eq!(added.len(), 1);
        let old = analyze("a.rs", "let value = \"one space\";");
        let new = analyze("a.rs", "let value = \"one  space\";");
        assert_eq!(align(&old.nodes, &new.nodes).1.len(), 1);
    }
}
