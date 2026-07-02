use std::path::{Path, PathBuf};

use testament_adapter_api::{
    AdapterResult, DetectScore, FrameworkAdapter, FrameworkSemantics, LanguageAdapter,
    MatcherSemantics, SyntaxNode, SyntaxTree,
};
use testament_core::{
    Assertion, AssertionKind, Confidence, ExternalRef, ExternalRefKind, Fixture, HelperDef,
    LiteralKind, LiteralValue, SharedExample, SharedExampleRef, SourceSpan, Statement,
    StatementRole, SubjectHint, Tag, TagKind, TestCase, TestDouble, TestFileIr, TestSuite,
    stable_test_id,
};

pub struct RubyAdapter;

impl LanguageAdapter for RubyAdapter {
    fn id(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, path: &Path, content: &[u8]) -> DetectScore {
        if path.extension().and_then(|extension| extension.to_str()) == Some("rb") {
            return DetectScore::HIGH;
        }
        if String::from_utf8_lossy(content).contains("RSpec.describe") {
            DetectScore::MEDIUM
        } else {
            DetectScore::NONE
        }
    }

    fn parse(&self, content: &[u8]) -> AdapterResult<SyntaxTree> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .map_err(|error| {
                testament_adapter_api::AdapterError::new(format!(
                    "failed to load Ruby grammar: {error}"
                ))
            })?;
        let tree = parser.parse(content, None).ok_or_else(|| {
            testament_adapter_api::AdapterError::new("tree-sitter failed to parse Ruby source")
        })?;
        let root = tree.root_node();
        let mut syntax = SyntaxTree::from_parts("ruby", content, root.kind(), root.has_error());
        collect_syntax_nodes(root, content, None, &mut syntax.nodes);
        Ok(syntax)
    }
}

impl FrameworkAdapter for RubyAdapter {
    fn id(&self) -> &'static str {
        "ruby-auto"
    }

    fn language(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, tree: &SyntaxTree, path: &Path) -> DetectScore {
        let content = tree.text();
        match detect_framework(&content) {
            "ruby" if path.extension().and_then(|extension| extension.to_str()) == Some("rb") => {
                DetectScore::LOW
            }
            "ruby" => DetectScore::NONE,
            _ => DetectScore::HIGH,
        }
    }

    fn lower(&self, tree: &SyntaxTree, path: &Path) -> AdapterResult<TestFileIr> {
        let mut ir = lower_from_syntax_tree(path, tree);
        ir.confidence = if tree.has_error {
            Confidence::Unresolved
        } else {
            Confidence::Exact
        };
        Ok(ir)
    }

    fn semantics(&self) -> FrameworkSemantics {
        FrameworkSemantics {
            assertion_matchers: vec![
                matcher("eq", "equality"),
                matcher("assert_equal", "equality"),
                matcher("raise_error", "exception"),
                matcher("change", "change"),
                matcher("receive", "mock_verification"),
                matcher("include", "collection"),
            ],
            skip_markers: vec![
                "skip".to_owned(),
                "pending".to_owned(),
                "xit".to_owned(),
                "omit".to_owned(),
            ],
            fixture_markers: vec![
                "before".to_owned(),
                "let".to_owned(),
                "let!".to_owned(),
                "setup".to_owned(),
            ],
        }
    }
}

fn collect_syntax_nodes(
    node: tree_sitter::Node<'_>,
    content: &[u8],
    parent: Option<usize>,
    nodes: &mut Vec<SyntaxNode>,
) -> usize {
    let index = nodes.len();
    let range = node.byte_range();
    let text = content
        .get(range.clone())
        .map(String::from_utf8_lossy)
        .map(|text| text.into_owned())
        .unwrap_or_default();
    nodes.push(SyntaxNode {
        kind: node.kind().to_owned(),
        text,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        start_byte: range.start,
        end_byte: range.end,
        parent,
        children: Vec::new(),
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_index = collect_syntax_nodes(child, content, Some(index), nodes);
        nodes[index].children.push(child_index);
    }
    index
}

fn lower_from_syntax_tree(path: &Path, tree: &SyntaxTree) -> TestFileIr {
    let content = tree.text();
    let framework = detect_framework(&content);
    let mut ir = TestFileIr::new(path, "ruby", framework);
    ir.subject_hints.extend(infer_subject_hints(path));

    let suite_nodes = suite_nodes(tree);
    let shared_nodes = shared_example_nodes(tree);
    let shared_spans = shared_nodes
        .iter()
        .map(|node| expanded_case_span(tree, node))
        .collect::<Vec<_>>();
    let all_case_nodes = case_nodes(tree);
    let case_nodes = all_case_nodes
        .iter()
        .copied()
        .filter(|node| !is_inside_spans(node.start_line, &shared_spans))
        .collect::<Vec<_>>();

    let suite_name = suite_nodes
        .first()
        .and_then(|node| extract_name(&node.text))
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "ruby tests".to_owned());
    let mut suite = TestSuite::new(suite_name, SourceSpan::line(1));

    for shared_node in &shared_nodes {
        if let Some(shared_example) =
            lower_shared_example(path, framework, tree, shared_node, &all_case_nodes)
        {
            ir.shared_examples.push(shared_example);
        }
    }

    let case_spans = case_nodes
        .iter()
        .map(|node| expanded_case_span(tree, node))
        .collect::<Vec<_>>();

    for line in &tree.lines {
        if case_spans
            .iter()
            .chain(shared_spans.iter())
            .any(|span| line.line >= span.start_line && line.line <= span.end_line)
        {
            continue;
        }
        let trimmed = line.text.trim();
        collect_fixture_or_helper(trimmed, line.line, &mut ir, &mut suite);
        if let Some(ref_name) = shared_example_ref_name(trimmed) {
            ir.shared_example_refs.push(SharedExampleRef {
                name: ref_name.clone(),
                span: SourceSpan::line(line.line),
            });
            let current_suite =
                nearest_suite_name(&suite_nodes, line.line).unwrap_or_else(|| suite.name.clone());
            expand_shared_example_ref(
                path,
                &current_suite,
                &ref_name,
                line.line,
                &ir.shared_examples,
                &mut suite,
            );
        }
    }

    for node in case_nodes {
        let current_suite =
            nearest_suite_name(&suite_nodes, node.start_line).unwrap_or_else(|| suite.name.clone());
        if let Some(case) = lower_case_node(path, framework, tree, node, &current_suite) {
            suite.cases.push(case);
        }
    }

    suite.span.end_line = tree.lines.last().map(|line| line.line).unwrap_or(1);
    ir.suites.push(suite);
    ir
}

fn suite_nodes(tree: &SyntaxTree) -> Vec<&SyntaxNode> {
    let mut nodes = tree
        .nodes
        .iter()
        .filter(|node| is_suite_node(node))
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| (node.start_line, node.start_byte));
    nodes
}

fn case_nodes(tree: &SyntaxTree) -> Vec<&SyntaxNode> {
    let mut nodes = tree
        .nodes
        .iter()
        .filter(|node| is_case_node(node))
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| (node.start_line, node.start_byte));
    nodes.dedup_by_key(|node| node.start_byte);
    nodes
}

fn shared_example_nodes(tree: &SyntaxTree) -> Vec<&SyntaxNode> {
    let mut nodes = tree
        .nodes
        .iter()
        .filter(|node| is_shared_example_node(node))
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| (node.start_line, node.start_byte));
    nodes.dedup_by_key(|node| node.start_byte);
    nodes
}

fn lower_shared_example(
    path: &Path,
    framework: &str,
    tree: &SyntaxTree,
    node: &SyntaxNode,
    all_case_nodes: &[&SyntaxNode],
) -> Option<SharedExample> {
    let name = extract_name(&node.text)?;
    let span = expanded_case_span(tree, node);
    let cases = all_case_nodes
        .iter()
        .filter(|case_node| {
            case_node.start_line >= span.start_line && case_node.start_line <= span.end_line
        })
        .filter_map(|case_node| {
            lower_case_node(path, framework, tree, case_node, &format!("shared:{name}"))
        })
        .collect::<Vec<_>>();

    Some(SharedExample { name, span, cases })
}

fn lower_case_node(
    path: &Path,
    framework: &str,
    tree: &SyntaxTree,
    node: &SyntaxNode,
    suite: &str,
) -> Option<TestCase> {
    let span = expanded_case_span(tree, node);
    let first_line = tree
        .lines
        .iter()
        .find(|line| line.line == node.start_line)
        .map(|line| line.text.trim().to_owned())
        .unwrap_or_else(|| node.text.lines().next().unwrap_or("").trim().to_owned());
    let mut case = start_case(path, framework, suite, &first_line, node.start_line)?;
    case.span = span.clone();

    for line in tree
        .lines
        .iter()
        .filter(|line| line.line >= span.start_line && line.line <= span.end_line)
    {
        collect_case_line(&mut case, line.text.trim(), line.line);
    }

    case.span = span;
    Some(case)
}

fn expand_shared_example_ref(
    path: &Path,
    suite_name: &str,
    ref_name: &str,
    ref_line: usize,
    shared_examples: &[SharedExample],
    suite: &mut TestSuite,
) {
    let Some(shared) = shared_examples
        .iter()
        .find(|shared| shared.name == ref_name)
    else {
        return;
    };

    for template in &shared.cases {
        let expanded_name = format!("{ref_name} / {}", template.name);
        let mut case = template.clone();
        case.id = stable_test_id(path, suite_name, &expanded_name, ref_line);
        case.name = expanded_name;
        case.span = SourceSpan::line(ref_line);
        case.evidence_aliases = evidence_aliases(&case.id, suite_name, &case.name);
        suite.cases.push(case);
    }
}

fn is_suite_node(node: &SyntaxNode) -> bool {
    if !is_call_like(node) {
        return false;
    }
    let text = node.text.trim_start();
    starts_any(
        text,
        &[
            "RSpec.describe",
            "RSpec.context",
            "describe ",
            "describe(",
            "context ",
            "context(",
        ],
    )
}

fn is_shared_example_node(node: &SyntaxNode) -> bool {
    if !is_call_like(node) {
        return false;
    }
    let text = node.text.trim_start();
    starts_any(
        text,
        &[
            "shared_examples ",
            "shared_examples(",
            "RSpec.shared_examples ",
            "RSpec.shared_examples(",
            "shared_examples_for ",
            "shared_examples_for(",
            "RSpec.shared_examples_for ",
            "RSpec.shared_examples_for(",
            "shared_context ",
            "shared_context(",
            "RSpec.shared_context ",
            "RSpec.shared_context(",
        ],
    )
}

fn is_case_node(node: &SyntaxNode) -> bool {
    if !is_call_like(node) {
        return false;
    }
    let text = node.text.trim_start();
    starts_any(
        text,
        &[
            "it ",
            "it(",
            "specify ",
            "specify(",
            "example ",
            "example(",
            "scenario ",
            "xit ",
            "xit(",
            "xspecify ",
            "pending ",
            "pending(",
            "test ",
            "test(",
        ],
    ) || text.starts_with("def test_")
}

fn is_inside_spans(line: usize, spans: &[SourceSpan]) -> bool {
    spans
        .iter()
        .any(|span| line >= span.start_line && line <= span.end_line)
}

fn is_call_like(node: &SyntaxNode) -> bool {
    matches!(
        node.kind.as_str(),
        "call" | "command" | "method" | "method_add_block"
    )
}

fn expanded_case_span(tree: &SyntaxTree, node: &SyntaxNode) -> SourceSpan {
    let mut start_line = node.start_line;
    let mut end_line = node.end_line;
    let mut parent = node.parent;

    while let Some(parent_index) = parent {
        let Some(parent_node) = tree.nodes.get(parent_index) else {
            break;
        };
        if parent_node.start_line != start_line {
            break;
        }
        if !matches!(
            parent_node.kind.as_str(),
            "call" | "command" | "block" | "do_block" | "method_add_block"
        ) {
            break;
        }
        end_line = end_line.max(parent_node.end_line);
        start_line = start_line.min(parent_node.start_line);
        parent = parent_node.parent;
    }

    SourceSpan {
        start_line,
        end_line,
    }
}

fn nearest_suite_name(suites: &[&SyntaxNode], line: usize) -> Option<String> {
    suites
        .iter()
        .filter(|suite| suite.start_line <= line)
        .max_by_key(|suite| suite.start_line)
        .and_then(|suite| extract_name(&suite.text))
}

fn matcher(matcher: &str, assertion_kind: &str) -> MatcherSemantics {
    MatcherSemantics {
        matcher: matcher.to_owned(),
        assertion_kind: assertion_kind.to_owned(),
    }
}

fn detect_framework(content: &str) -> &'static str {
    if content.contains("RSpec.describe")
        || content.contains("RSpec.context")
        || content.contains("expect(")
        || content.contains("describe ")
    {
        "rspec"
    } else if content.contains("Minitest::") || content.contains("minitest/autorun") {
        "minitest"
    } else if content.contains("Test::Unit") || content.contains("test/unit") {
        "test-unit"
    } else {
        "ruby"
    }
}

fn infer_subject_hints(path: &Path) -> Vec<SubjectHint> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut candidates = Vec::new();

    if let Some(candidate) = normalized
        .strip_prefix("spec/")
        .and_then(|path| path.strip_suffix("_spec.rb"))
        .or_else(|| {
            normalized
                .strip_prefix("test/")
                .and_then(|path| path.strip_suffix("_test.rb"))
        })
    {
        candidates.push(
            candidate
                .strip_prefix("test_")
                .unwrap_or(candidate)
                .to_owned(),
        );
    }

    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        && let Some(candidate) = stem
            .strip_suffix("_spec")
            .or_else(|| stem.strip_suffix("_test"))
    {
        candidates.push(
            candidate
                .strip_prefix("test_")
                .unwrap_or(candidate)
                .to_owned(),
        );
    }

    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .map(|candidate| SubjectHint {
            path: PathBuf::from(format!("lib/{candidate}.rb")),
            confidence: Confidence::Approximate,
        })
        .collect()
}

fn collect_fixture_or_helper(
    trimmed: &str,
    line_no: usize,
    ir: &mut TestFileIr,
    suite: &mut TestSuite,
) {
    if trimmed.starts_with("let!(") || trimmed.starts_with("let ") || trimmed.starts_with("let(") {
        let name = extract_between(trimmed, "(:", ")")
            .or_else(|| extract_between(trimmed, "('", "'"))
            .or_else(|| extract_between(trimmed, "(\"", "\""))
            .unwrap_or_else(|| "let".to_owned());
        ir.shared_fixtures.push(Fixture {
            name,
            span: SourceSpan::line(line_no),
            eager: trimmed.starts_with("let!"),
        });
    } else if starts_any(trimmed, &["before ", "before(", "setup do", "def setup"]) {
        suite.fixtures.push(Fixture {
            name: "setup".to_owned(),
            span: SourceSpan::line(line_no),
            eager: true,
        });
    } else if trimmed.starts_with("subject") {
        ir.shared_fixtures.push(Fixture {
            name: "subject".to_owned(),
            span: SourceSpan::line(line_no),
            eager: false,
        });
    } else if trimmed.starts_with("def ") && !trimmed.starts_with("def test_") {
        ir.helpers.push(HelperDef {
            name: trimmed.trim_start_matches("def ").trim().to_owned(),
            span: SourceSpan::line(line_no),
        });
    }
}

fn shared_example_ref_name(trimmed: &str) -> Option<String> {
    if !starts_any(
        trimmed,
        &[
            "it_behaves_like ",
            "it_behaves_like(",
            "include_examples ",
            "include_examples(",
            "include_context ",
            "include_context(",
        ],
    ) {
        return None;
    }
    extract_name(trimmed)
}

fn start_case(
    path: &Path,
    framework: &str,
    suite: &str,
    trimmed: &str,
    line_no: usize,
) -> Option<TestCase> {
    let skipped = starts_any(trimmed, &["xit ", "xit(", "xspecify ", "skip "]);
    let pending = starts_any(trimmed, &["pending ", "pending("]);
    let rspec_case = starts_any(
        trimmed,
        &[
            "it ",
            "it(",
            "specify ",
            "specify(",
            "example ",
            "example(",
            "scenario ",
        ],
    ) || skipped
        || pending;
    let method_case = trimmed.starts_with("def test_");
    let test_block = trimmed.starts_with("test ") || trimmed.starts_with("test(");

    if framework == "rspec" && !rspec_case {
        return None;
    }
    if framework != "rspec" && !(method_case || test_block || rspec_case) {
        return None;
    }

    let name = extract_name(trimmed).unwrap_or_else(|| {
        trimmed
            .trim_start_matches("def ")
            .trim_end_matches(" do")
            .replace('_', " ")
    });
    let id = stable_test_id(path, suite, &name, line_no);
    let mut case = TestCase::new(id, name, SourceSpan::line(line_no));
    case.evidence_aliases = evidence_aliases(&case.id, suite, &case.name);

    if skipped {
        case.tags.push(Tag {
            kind: TagKind::Skipped,
            label: "skip".to_owned(),
            span: SourceSpan::line(line_no),
        });
    }
    if pending {
        case.tags.push(Tag {
            kind: TagKind::Pending,
            label: "pending".to_owned(),
            span: SourceSpan::line(line_no),
        });
    }
    if trimmed.contains(":focus") || trimmed.contains("focus: true") {
        case.tags.push(Tag {
            kind: TagKind::Focus,
            label: "focus".to_owned(),
            span: SourceSpan::line(line_no),
        });
    }

    Some(case)
}

fn evidence_aliases(id: &str, suite: &str, name: &str) -> Vec<String> {
    let mut aliases = vec![
        id.to_owned(),
        name.to_owned(),
        format!("{suite} {name}"),
        format!("{suite}::{name}"),
        format!("{suite}#{name}"),
    ];
    aliases.sort();
    aliases.dedup();
    aliases
}

fn collect_case_line(case: &mut TestCase, trimmed: &str, line_no: usize) {
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return;
    }

    let role = if is_assertion_line(trimmed) {
        StatementRole::Assert
    } else if looks_like_sut_call(trimmed) {
        StatementRole::Act
    } else {
        StatementRole::Arrange
    };

    case.statements.push(Statement {
        text: trimmed.to_owned(),
        role,
        span: SourceSpan::line(line_no),
    });
    case.normalized_body.push(normalize_line(trimmed));
    collect_assertion(case, trimmed, line_no);
    collect_external_refs(case, trimmed, line_no);
    collect_doubles(case, trimmed, line_no);
    collect_misc_signals(case, trimmed, line_no);
}

fn collect_assertion(case: &mut TestCase, trimmed: &str, line_no: usize) {
    let Some(mut assertion) = parse_assertion(trimmed, line_no) else {
        return;
    };
    if assertion.subject_expr.is_empty() {
        assertion.subject_expr = trimmed.to_owned();
    }
    case.assertions.push(assertion);
}

fn parse_assertion(trimmed: &str, line_no: usize) -> Option<Assertion> {
    if trimmed.contains("expect(") || trimmed.contains("expect {") || trimmed.contains(".should") {
        return Some(parse_rspec_assertion(trimmed, line_no));
    }
    if starts_any(
        trimmed,
        &["assert", "refute", "flunk", "_(", "must_", "wont_"],
    ) || trimmed.contains(".must_")
        || trimmed.contains(".wont_")
    {
        return Some(parse_assert_style_assertion(trimmed, line_no));
    }
    None
}

fn parse_rspec_assertion(trimmed: &str, line_no: usize) -> Assertion {
    let matcher = extract_rspec_matcher(trimmed);
    let subject_expr = extract_between(trimmed, "expect(", ")")
        .or_else(|| extract_between(trimmed, "expect {", "}"))
        .unwrap_or_default();
    let expected_expr =
        extract_between_after(trimmed, &matcher, "(", ")").filter(|value| !value.is_empty());

    Assertion {
        kind: classify_assertion(&matcher, trimmed),
        matcher,
        subject_expr,
        expected_expr,
        has_message: trimmed.contains("because(") || trimmed.contains("failure_message"),
        span: SourceSpan::line(line_no),
    }
}

fn parse_assert_style_assertion(trimmed: &str, line_no: usize) -> Assertion {
    let matcher = trimmed
        .split([' ', '('])
        .next()
        .unwrap_or("assert")
        .trim_start_matches("_(")
        .to_owned();
    let args = assertion_args(trimmed, &matcher);
    let (expected_expr, subject_expr) = if matcher.contains("equal") && args.len() >= 2 {
        (Some(args[0].clone()), args[1].clone())
    } else {
        (
            args.first().cloned(),
            args.get(1).cloned().unwrap_or_default(),
        )
    };

    Assertion {
        kind: classify_assertion(&matcher, trimmed),
        matcher,
        subject_expr,
        expected_expr,
        has_message: args.len() >= 3,
        span: SourceSpan::line(line_no),
    }
}

fn assertion_args(trimmed: &str, matcher: &str) -> Vec<String> {
    let raw = trimmed
        .trim_start_matches(matcher)
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    raw.split(',')
        .map(str::trim)
        .filter(|arg| !arg.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn extract_rspec_matcher(trimmed: &str) -> String {
    for needle in [".to_not ", ".not_to ", ".to ", ".should_not ", ".should "] {
        if let Some(rest) = trimmed.split_once(needle).map(|(_, rest)| rest) {
            return rest
                .split(['(', ' ', '{'])
                .next()
                .unwrap_or("matcher")
                .trim_start_matches("be_")
                .to_owned();
        }
    }
    "matcher".to_owned()
}

fn classify_assertion(matcher: &str, line: &str) -> AssertionKind {
    let haystack = format!("{matcher} {line}");
    if contains_any(&haystack, &["eq", "equal", "eql", "same", "=="]) {
        AssertionKind::Equality
    } else if contains_any(&haystack, &["raise", "throw"]) {
        AssertionKind::Exception
    } else if contains_any(&haystack, &["change", "difference"]) {
        AssertionKind::Change
    } else if contains_any(
        &haystack,
        &["receive", "have_received", "assert_mock", "verify"],
    ) {
        AssertionKind::MockVerification
    } else if contains_any(&haystack, &["include", "empty", "contain"]) {
        AssertionKind::Collection
    } else if contains_any(&haystack, &["snapshot", "match_snapshot"]) {
        AssertionKind::Snapshot
    } else if matcher.starts_with("be")
        || matcher.starts_with("assert")
        || matcher.starts_with("refute")
    {
        AssertionKind::Predicate
    } else {
        AssertionKind::Other
    }
}

fn collect_external_refs(case: &mut TestCase, trimmed: &str, line_no: usize) {
    for (kind, needles) in [
        (
            ExternalRefKind::FileSystem,
            &["File.", "FileUtils.", "Dir.", "open("][..],
        ),
        (
            ExternalRefKind::Network,
            &["Net::HTTP", "URI.open", "Faraday.", "HTTP.", "WebMock"][..],
        ),
        (
            ExternalRefKind::Database,
            &["ActiveRecord::Base.connection", ".find_by_sql", "Sequel."][..],
        ),
        (
            ExternalRefKind::Time,
            &[
                "Time.now",
                "Date.today",
                "DateTime.now",
                "Process.clock_gettime",
            ][..],
        ),
        (ExternalRefKind::Sleep, &["sleep ", "sleep("][..]),
        (
            ExternalRefKind::GlobalState,
            &["@@", "$", "ENV[", "Thread.current"][..],
        ),
    ] {
        if contains_any(trimmed, needles) {
            case.external_refs.push(ExternalRef {
                kind,
                expression: trimmed.to_owned(),
                span: SourceSpan::line(line_no),
            });
        }
    }
}

fn collect_doubles(case: &mut TestCase, trimmed: &str, line_no: usize) {
    for needle in [
        "double(",
        "instance_double",
        "class_double",
        "allow(",
        "receive(",
        "Minitest::Mock",
        ".stub(",
    ] {
        if trimmed.contains(needle) {
            case.doubles.push(TestDouble {
                kind: needle.trim_end_matches('(').to_owned(),
                span: SourceSpan::line(line_no),
            });
        }
    }
}

fn collect_misc_signals(case: &mut TestCase, trimmed: &str, line_no: usize) {
    if starts_any(
        trimmed,
        &["if ", "unless ", "case ", "while ", "until ", "for "],
    ) || trimmed.contains(".each do")
    {
        case.control_flow.push(SourceSpan::line(line_no));
    }
    if starts_any(trimmed, &["puts ", "p ", "pp "]) {
        case.print_lines.push(SourceSpan::line(line_no));
    }
    case.sut_calls.extend(extract_sut_calls(trimmed));
    case.literals.extend(extract_literals(trimmed, line_no));
}

fn extract_sut_calls(line: &str) -> Vec<String> {
    line.split(|character: char| !is_ident_or_dot(character))
        .filter(|token| token.contains('.'))
        .filter(|token| {
            !starts_any(
                token,
                &[
                    "expect.", "allow.", "Time.", "Date.", "File.", "Dir.", "Net.", "URI.",
                ],
            )
        })
        .filter(|token| !contains_any(token, &[".to", ".not_to", ".should", ".must_", ".wont_"]))
        .map(ToOwned::to_owned)
        .collect()
}

fn extract_literals(line: &str, line_no: usize) -> Vec<LiteralValue> {
    let mut literals = Vec::new();
    for token in line.split(|character: char| {
        !(character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || character == '.')
    }) {
        let kind = if matches!(token, "nil" | "true" | "false" | "0" | "1" | "-1") {
            Some(LiteralKind::Boundary)
        } else if token.parse::<i64>().is_ok() || token.parse::<f64>().is_ok() {
            Some(LiteralKind::Number)
        } else {
            None
        };
        if let Some(kind) = kind {
            literals.push(LiteralValue {
                raw: token.to_owned(),
                kind,
                span: SourceSpan::line(line_no),
            });
        }
    }
    if line.contains("\"\"") || line.contains("''") || line.contains("[]") || line.contains("{}") {
        literals.push(LiteralValue {
            raw: "empty".to_owned(),
            kind: LiteralKind::Boundary,
            span: SourceSpan::line(line_no),
        });
    }
    literals
}

fn looks_like_sut_call(trimmed: &str) -> bool {
    !extract_sut_calls(trimmed).is_empty()
}

fn is_assertion_line(trimmed: &str) -> bool {
    parse_assertion(trimmed, 0).is_some()
}

fn normalize_line(line: &str) -> String {
    line.split_whitespace()
        .map(|token| {
            if token.parse::<f64>().is_ok() {
                "<num>"
            } else if token.starts_with('"') || token.starts_with('\'') {
                "<str>"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_name(line: &str) -> Option<String> {
    extract_between(line, "\"", "\"")
        .or_else(|| extract_between(line, "'", "'"))
        .or_else(|| line.split_whitespace().nth(1).map(ToOwned::to_owned))
}

fn extract_between(line: &str, start: &str, end: &str) -> Option<String> {
    let (_, rest) = line.split_once(start)?;
    let (value, _) = rest.split_once(end)?;
    Some(value.trim().to_owned())
}

fn extract_between_after(line: &str, after: &str, start: &str, end: &str) -> Option<String> {
    let (_, rest) = line.split_once(after)?;
    extract_between(rest, start, end)
}

fn contains_any(line: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| line.contains(needle))
}

fn starts_any(line: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| line.starts_with(needle))
}

fn is_ident_or_dot(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '?' | '!')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use testament_adapter_api::{FrameworkAdapter, LanguageAdapter};

    #[test]
    fn lowers_rspec_cases_and_smell_signals() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
            RSpec.describe Order do
              let(:order) { described_class.new }

              it "waits" do
                sleep 1
                expect(order.total).to eq(0)
              end
            end
            "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        let case = ir.cases()[0];
        assert_eq!(ir.framework, "rspec");
        assert_eq!(case.assertions.len(), 1);
        assert_eq!(case.external_refs[0].kind, ExternalRefKind::Sleep);
        assert_eq!(ir.shared_fixtures[0].name, "order");
    }

    #[test]
    fn parses_with_tree_sitter_and_marks_confidence() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Order do
                  it "works" do
                    expect(1).to eq(1)
                  end
                end
                "#,
            )
            .unwrap();

        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        assert_eq!(tree.root_kind, "program");
        assert!(tree.nodes.iter().any(|node| node.kind == "call"));
        assert!(!tree.has_error);
        assert_eq!(ir.confidence, Confidence::Exact);
        assert_eq!(ir.case_count(), 1);
        assert_eq!(ir.assertion_count(), 1);
    }

    #[test]
    fn marks_syntax_errors_as_unresolved() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(b"RSpec.describe Order do\n  it 'breaks'\n")
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        assert!(tree.has_error);
        assert_eq!(ir.confidence, Confidence::Unresolved);
    }

    #[test]
    fn keeps_and_expands_shared_examples() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.shared_examples "priced thing" do
                  it "has a price" do
                    expect(subject.price).to eq(1)
                  end
                end

                RSpec.describe Product do
                  it_behaves_like "priced thing"
                end
                "#,
            )
            .unwrap();
        let ir =
            FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/product_spec.rb")).unwrap();

        assert_eq!(ir.shared_examples.len(), 1);
        assert_eq!(ir.shared_example_refs.len(), 1);
        assert_eq!(ir.case_count(), 1);
        assert_eq!(ir.assertion_count(), 1);
        assert!(ir.cases()[0].name.contains("priced thing"));
    }
}
