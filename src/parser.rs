use std::collections::BTreeSet;
use tree_sitter::{Node, ParseOptions, Parser, Point};

pub(crate) const RUST_PARSER_VERSION: &str = "tree-sitter-rust-0.24.2-core-0.25.10-lld-ir-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceSpan {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxSymbol {
    pub stable_key: String,
    pub correspondence_key: String,
    pub kind: String,
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxReference {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug)]
#[cfg(feature = "server")]
pub(crate) struct SyntaxImport {
    pub root: Option<String>,
    pub span: SourceSpan,
    pub test_only: bool,
}

#[cfg(feature = "server")]
#[derive(Clone, Debug)]
pub(crate) struct SyntaxContract {
    pub name: String,
    pub span: SourceSpan,
    pub test_only: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ParsedFile {
    pub definitions: BTreeSet<String>,
    pub calls: BTreeSet<String>,
    pub imports: BTreeSet<String>,
    #[cfg(feature = "server")]
    pub import_evidence: Vec<SyntaxImport>,
    #[cfg(feature = "server")]
    pub trait_evidence: Vec<SyntaxContract>,
    pub symbols: Vec<SyntaxSymbol>,
    pub references: Vec<SyntaxReference>,
    pub error_nodes: usize,
}

pub(crate) fn parse_rust(
    path: &str,
    source: &str,
    cancellation: &crate::core::Cancellation,
) -> Result<ParsedFile, String> {
    cancellation
        .checkpoint()
        .map_err(|_| "Rust parsing cancelled".to_owned())?;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|error| format!("Cannot load Rust parser: {error}"))?;
    let started = std::time::Instant::now();
    let mut progress = |_: &tree_sitter::ParseState| {
        cancellation.is_cancelled() || started.elapsed() >= std::time::Duration::from_secs(2)
    };
    let mut input = |offset: usize, _point: Point| &source.as_bytes()[offset..];
    let tree = parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or_else(|| {
            if cancellation.is_cancelled() {
                "Rust parsing cancelled".to_owned()
            } else {
                "Rust parser reached its two-second limit".to_owned()
            }
        })?;
    cancellation
        .checkpoint()
        .map_err(|_| "Rust parsing cancelled".to_owned())?;
    let mut parsed = ParsedFile::default();
    visit(
        path,
        source.as_bytes(),
        tree.root_node(),
        &mut Vec::new(),
        &mut parsed,
    )?;
    Ok(parsed)
}

fn visit(
    path: &str,
    source: &[u8],
    node: Node<'_>,
    scope: &mut Vec<String>,
    parsed: &mut ParsedFile,
) -> Result<(), String> {
    if node.is_error() || node.is_missing() {
        parsed.error_nodes += 1;
    }
    if matches!(
        node.kind(),
        "function_item" | "struct_item" | "enum_item" | "trait_item" | "type_item" | "const_item"
    ) {
        if let Some(name) = node.child_by_field_name("name") {
            let name = text(source, name)?.to_owned();
            parsed.definitions.insert(name.clone());
            let correspondence_key =
                format!("{path}:{}:{}:{}", scope.join("::"), node.kind(), name);
            parsed.symbols.push(SyntaxSymbol {
                stable_key: format!(
                    "{correspondence_key}:{}:{}",
                    node.start_position().row,
                    node.start_position().column
                ),
                correspondence_key,
                kind: node.kind().into(),
                name,
                span: span(node.start_position(), node.end_position()),
            });
            #[cfg(feature = "server")]
            if node.kind() == "trait_item" {
                parsed.trait_evidence.push(SyntaxContract {
                    name: text(source, node.child_by_field_name("name").unwrap())?.to_owned(),
                    span: span(node.start_position(), node.end_position()),
                    test_only: test_context(source, node),
                });
            }
        }
    }
    if node.kind() == "call_expression" {
        if let Some(function) = node.child_by_field_name("function") {
            if let Some(name) = terminal_identifier(source, function)? {
                parsed.calls.insert(name.clone());
                parsed.references.push(SyntaxReference {
                    name,
                    span: span(node.start_position(), node.end_position()),
                });
            }
        }
    }
    if node.kind() == "use_declaration" {
        parsed.imports.insert(text(source, node)?.trim().to_owned());
        #[cfg(feature = "server")]
        parsed.import_evidence.push(SyntaxImport {
            root: node
                .child_by_field_name("argument")
                .and_then(|argument| import_root(source, argument)),
            span: span(node.start_position(), node.end_position()),
            test_only: test_context(source, node),
        });
    }
    let scope_name = if node.kind() == "impl_item" {
        node.child_by_field_name("type")
            .map(|kind| text(source, kind).map(|name| format!("impl {name}")))
            .transpose()?
            .or_else(|| Some("impl unknown".into()))
    } else if matches!(node.kind(), "mod_item" | "trait_item" | "function_item") {
        node.child_by_field_name("name")
            .map(|name| text(source, name).map(str::to_owned))
            .transpose()?
    } else {
        None
    };
    if let Some(name) = &scope_name {
        scope.push(name.clone());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(path, source, child, scope, parsed)?;
    }
    if scope_name.is_some() {
        scope.pop();
    }
    Ok(())
}

// Only a single syntactic root is supported. Grouped roots remain unresolved.
#[cfg(feature = "server")]
fn import_root(source: &[u8], node: Node<'_>) -> Option<String> {
    if matches!(node.kind(), "identifier" | "crate" | "self" | "super") {
        return text(source, node).ok().map(str::to_owned);
    }
    if matches!(node.kind(), "use_list" | "line_comment" | "block_comment") {
        return None;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| import_root(source, child))
}

#[cfg(feature = "server")]
fn test_context(source: &[u8], mut node: Node<'_>) -> bool {
    loop {
        let mut sibling = node.prev_named_sibling();
        while let Some(attribute) = sibling {
            if attribute.kind() == "attribute_item" {
                let compact = text(source, attribute)
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                // Only definite simple test predicates are classified. Complex
                // cfg expressions remain unverified instead of evaluating features.
                if compact == "#[cfg(test)]"
                    || compact == "#[test]"
                    || compact.starts_with("#[tokio::test")
                    || compact.starts_with("#[async_std::test")
                {
                    return true;
                }
            } else if !matches!(attribute.kind(), "line_comment" | "block_comment") {
                break;
            }
            sibling = attribute.prev_named_sibling();
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}

fn terminal_identifier(source: &[u8], node: Node<'_>) -> Result<Option<String>, String> {
    if matches!(node.kind(), "identifier" | "field_identifier") {
        return Ok(Some(text(source, node)?.to_owned()));
    }
    let mut cursor = node.walk();
    let mut result = None;
    for child in node.named_children(&mut cursor) {
        if let Some(name) = terminal_identifier(source, child)? {
            result = Some(name);
        }
    }
    Ok(result)
}

fn text<'a>(source: &'a [u8], node: Node<'_>) -> Result<&'a str, String> {
    node.utf8_text(source)
        .map_err(|_| "Parser returned a non-UTF-8 source span".into())
}

fn span(start: Point, end: Point) -> SourceSpan {
    SourceSpan {
        start_line: start.row + 1,
        start_column: start.column + 1,
        end_line: end.row + 1,
        end_column: end.column + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_scoped_symbols_calls_imports_and_spans() {
        let parsed = parse_rust(
            "src/lib.rs",
            "use crate::client::send;\nmod a { fn run() { client.send(); } }\nmod b { fn run() { send(); } }",
            &crate::core::Cancellation::default(),
        )
        .unwrap();
        assert_eq!(parsed.symbols.len(), 2);
        assert_ne!(parsed.symbols[0].stable_key, parsed.symbols[1].stable_key);
        assert_ne!(
            parsed.symbols[0].correspondence_key,
            parsed.symbols[1].correspondence_key
        );
        assert_eq!(parsed.calls, BTreeSet::from(["send".into()]));
        assert_eq!(parsed.references.len(), 2);
        assert_eq!(parsed.imports.len(), 1);
        assert_eq!(parsed.symbols[0].span.start_line, 2);
    }

    #[test]
    fn malformed_rust_retains_recovery_diagnostics() {
        let parsed = parse_rust(
            "broken.rs",
            "fn broken( { call(); }",
            &crate::core::Cancellation::default(),
        )
        .unwrap();
        assert!(parsed.error_nodes > 0);
    }

    #[test]
    fn correspondence_survives_line_moves_and_methods_keep_scope_identity() {
        let cancellation = crate::core::Cancellation::default();
        let first = parse_rust(
            "src/lib.rs",
            "impl A { fn run() {} }\nimpl B { fn run() {} }",
            &cancellation,
        )
        .unwrap();
        let moved = parse_rust(
            "src/lib.rs",
            "\n\nimpl A { fn run() {} }\nimpl B { fn run() {} }",
            &cancellation,
        )
        .unwrap();
        let first_methods = first
            .symbols
            .iter()
            .filter(|symbol| symbol.name == "run")
            .collect::<Vec<_>>();
        let moved_methods = moved
            .symbols
            .iter()
            .filter(|symbol| symbol.name == "run")
            .collect::<Vec<_>>();
        assert_eq!(first_methods.len(), 2);
        assert_ne!(
            first_methods[0].correspondence_key,
            first_methods[1].correspondence_key
        );
        assert_eq!(
            first_methods
                .iter()
                .map(|symbol| &symbol.correspondence_key)
                .collect::<BTreeSet<_>>(),
            moved_methods
                .iter()
                .map(|symbol| &symbol.correspondence_key)
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    #[cfg(feature = "server")]
    fn import_evidence_tracks_roots_and_definite_test_context_without_comment_or_alias_claims() {
        let parsed = parse_rust("src/lib.rs", "use /* sqlx */ harmless as axum;\nuse foo::{tonic, reqwest};\nuse {axum::Router, sqlx::Pool};\n#[cfg(test)]\nmod tests { use lapin::Channel; trait Sample {} }\n#[cfg(not(test))]\nmod live { use object_store::ObjectStore; }\n", &crate::Cancellation::default()).unwrap();
        assert_eq!(parsed.error_nodes, 0);
        let roots = parsed
            .import_evidence
            .iter()
            .map(|item| (item.root.as_deref(), item.test_only))
            .collect::<Vec<_>>();
        assert_eq!(
            roots,
            vec![
                (Some("harmless"), false),
                (Some("foo"), false),
                (None, false),
                (Some("lapin"), true),
                (Some("object_store"), false)
            ]
        );
        assert_eq!(parsed.import_evidence[3].span.start_line, 5);
        assert!(parsed.trait_evidence[0].test_only);
    }

    #[test]
    fn cancellation_stops_parser_before_publication() {
        let cancellation = crate::core::Cancellation::default();
        cancellation.cancel();
        assert_eq!(
            parse_rust("src/lib.rs", "fn run() {}", &cancellation).unwrap_err(),
            "Rust parsing cancelled"
        );
    }
}
