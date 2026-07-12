//! Prism-based Ruby syntax parsing and test-IR lowering.

use std::path::{Path, PathBuf};

use ruby_prism::{Node, Visit};
use testament_adapter_api::{
    AdapterResult, DetectScore, FrameworkAdapter, FrameworkSemantics, LanguageAdapter,
    MatcherSemantics, SyntaxNode, SyntaxTree,
};
use testament_core::{
    Assertion, AssertionKind, CallSite, Confidence, ExternalRef, ExternalRefKind, Fixture,
    HelperDef, LiteralKind, LiteralValue, SharedExample, SharedExampleRef, SourceSpan, Statement,
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
        let result = ruby_prism::parse(content);
        let has_error = result.errors().next().is_some();
        let mut syntax = SyntaxTree::from_parts("ruby", content, "program", has_error);
        let mut visitor = PrismSyntaxVisitor::new(content);
        visitor.visit(&result.node());
        syntax.nodes = visitor.nodes;
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
        apply_framework_semantics(
            &mut ir,
            &testament_adapter_api::FrameworkAdapter::semantics(self),
        );
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
                matcher("must_equal", "equality"),
                matcher("wont_equal", "equality"),
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

pub fn apply_framework_semantics(ir: &mut TestFileIr, semantics: &FrameworkSemantics) {
    for case in ir.suites.iter_mut().flat_map(suite_cases_mut) {
        for assertion in &mut case.assertions {
            if let Some(matcher) = semantics
                .assertion_matchers
                .iter()
                .find(|matcher| matcher.matcher == assertion.matcher)
            {
                assertion.kind = assertion_kind_from_semantics(&matcher.assertion_kind);
            }
        }
        for call in &case.calls {
            if !semantics
                .skip_markers
                .iter()
                .any(|marker| marker == &call.method)
                || case.tags.iter().any(|tag| tag.span == call.span)
            {
                continue;
            }
            case.tags.push(Tag {
                kind: if call.method == "skip" || call.method.starts_with('x') {
                    TagKind::Skipped
                } else {
                    TagKind::Pending
                },
                label: call.method.clone(),
                span: call.span.clone(),
            });
        }
    }
}

fn suite_cases_mut(suite: &mut TestSuite) -> Vec<&mut TestCase> {
    let mut cases = suite.cases.iter_mut().collect::<Vec<_>>();
    for nested in &mut suite.nested {
        cases.extend(suite_cases_mut(nested));
    }
    cases
}

fn assertion_kind_from_semantics(kind: &str) -> AssertionKind {
    match kind {
        "equality" => AssertionKind::Equality,
        "predicate" => AssertionKind::Predicate,
        "exception" => AssertionKind::Exception,
        "change" => AssertionKind::Change,
        "mock_verification" => AssertionKind::MockVerification,
        "collection" => AssertionKind::Collection,
        "snapshot" => AssertionKind::Snapshot,
        _ => AssertionKind::Other,
    }
}

struct PrismSyntaxVisitor<'source> {
    content: &'source [u8],
    line_starts: Vec<usize>,
    parents: Vec<usize>,
    nodes: Vec<SyntaxNode>,
}

impl<'source> PrismSyntaxVisitor<'source> {
    fn new(content: &'source [u8]) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            content
                .iter()
                .enumerate()
                .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1)),
        );
        Self {
            content,
            line_starts,
            parents: Vec::new(),
            nodes: Vec::new(),
        }
    }

    fn add_node(&mut self, node: &Node<'_>) -> usize {
        let location = node.location();
        let start_byte = location.start_offset();
        let end_byte = location.end_offset();
        let parent = self.parents.last().copied();
        let index = self.nodes.len();
        self.nodes.push(SyntaxNode {
            kind: prism_node_kind(node),
            text: self
                .content
                .get(start_byte..end_byte)
                .map(String::from_utf8_lossy)
                .map(|text| text.into_owned())
                .unwrap_or_default(),
            start_line: self.line_for_offset(start_byte),
            end_line: self.line_for_offset(end_byte.saturating_sub(1)),
            start_byte,
            end_byte,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        index
    }

    fn line_for_offset(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|start| *start <= offset)
            .max(1)
    }
}

impl<'pr> Visit<'pr> for PrismSyntaxVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        let index = self.add_node(&node);
        self.parents.push(index);
    }

    fn visit_branch_node_leave(&mut self) {
        self.parents.pop();
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.add_node(&node);
    }
}

fn prism_node_kind(node: &Node<'_>) -> String {
    if let Some(call) = node.as_call_node() {
        return format!("call:{}", String::from_utf8_lossy(call.name().as_slice()));
    }
    if let Some(definition) = node.as_def_node() {
        return format!(
            "def:{}",
            String::from_utf8_lossy(definition.name().as_slice())
        );
    }
    if node.as_global_variable_read_node().is_some()
        || node.as_global_variable_write_node().is_some()
        || node.as_global_variable_and_write_node().is_some()
        || node.as_global_variable_or_write_node().is_some()
        || node.as_global_variable_operator_write_node().is_some()
    {
        return "global_variable".to_owned();
    }
    if node.as_string_node().is_some() || node.as_interpolated_string_node().is_some() {
        return "string".to_owned();
    }
    if node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
    {
        return "regular_expression".to_owned();
    }
    if node.as_if_node().is_some() {
        return "if".to_owned();
    }
    if node.as_unless_node().is_some() {
        return "unless".to_owned();
    }
    if node.as_case_node().is_some() || node.as_case_match_node().is_some() {
        return "case".to_owned();
    }
    if node.as_while_node().is_some() {
        return "while".to_owned();
    }
    if node.as_until_node().is_some() {
        return "until".to_owned();
    }
    if node.as_for_node().is_some() {
        return "for".to_owned();
    }
    if node.as_float_node().is_some() || node.as_integer_node().is_some() {
        return "number".to_owned();
    }
    if node.as_nil_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
    {
        return "boundary".to_owned();
    }
    if node.as_array_node().is_some() || node.as_hash_node().is_some() {
        return "collection".to_owned();
    }
    if node.as_block_node().is_some() {
        return "block".to_owned();
    }
    if node.as_program_node().is_some() {
        return "program".to_owned();
    }
    "node".to_owned()
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

    for shared_node in &shared_nodes {
        if let Some(shared_example) =
            lower_shared_example(path, framework, tree, shared_node, &all_case_nodes)
        {
            ir.shared_examples.push(shared_example);
        }
    }

    collect_file_metadata(tree, &case_nodes, &shared_nodes, &mut ir);

    let top_level_suites = suite_nodes
        .iter()
        .copied()
        .filter(|candidate| nearest_containing_suite(&suite_nodes, candidate).is_none())
        .collect::<Vec<_>>();
    for suite_node in top_level_suites {
        ir.suites.push(lower_suite_node(
            path,
            framework,
            tree,
            suite_node,
            &suite_nodes,
            &case_nodes,
            &ir.shared_examples,
            &mut ir.shared_example_refs,
            None,
        ));
    }

    let orphan_cases = case_nodes
        .iter()
        .copied()
        .filter(|case| nearest_containing_suite(&suite_nodes, case).is_none())
        .collect::<Vec<_>>();
    let orphan_refs = shared_ref_nodes(tree)
        .into_iter()
        .filter(|reference| nearest_containing_suite(&suite_nodes, reference).is_none())
        .collect::<Vec<_>>();
    if !orphan_cases.is_empty() || !orphan_refs.is_empty() || ir.suites.is_empty() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("ruby tests")
            .to_owned();
        let mut suite = TestSuite::new(
            &name,
            SourceSpan {
                start_line: 1,
                end_line: tree.lines.last().map(|line| line.line).unwrap_or(1),
            },
        );
        suite.fixtures = suite_fixture_nodes_without_owner(tree, &suite_nodes)
            .into_iter()
            .map(fixture_from_node)
            .collect();
        for case_node in orphan_cases {
            if let Some(case) = lower_case_node(path, framework, tree, case_node, &name) {
                suite.cases.push(case);
            }
        }
        lower_shared_refs(
            path,
            &name,
            orphan_refs,
            &ir.shared_examples,
            &mut ir.shared_example_refs,
            &mut suite,
        );
        ir.suites.push(suite);
    }
    ir
}

#[allow(clippy::too_many_arguments)]
fn lower_suite_node(
    path: &Path,
    framework: &str,
    tree: &SyntaxTree,
    node: &SyntaxNode,
    suite_nodes: &[&SyntaxNode],
    case_nodes: &[&SyntaxNode],
    shared_examples: &[SharedExample],
    shared_refs: &mut Vec<SharedExampleRef>,
    parent_name: Option<&str>,
) -> TestSuite {
    let local_name = extract_name(&node.text).unwrap_or_else(|| "(anonymous suite)".to_owned());
    let full_name = parent_name
        .map(|parent| format!("{parent} {local_name}"))
        .unwrap_or_else(|| local_name.clone());
    let mut suite = TestSuite::new(local_name, node_span(node));

    for case_node in case_nodes.iter().copied().filter(|candidate| {
        nearest_containing_suite(suite_nodes, candidate).is_some_and(|owner| {
            owner.start_byte == node.start_byte && owner.end_byte == node.end_byte
        })
    }) {
        if let Some(case) = lower_case_node(path, framework, tree, case_node, &full_name) {
            suite.cases.push(case);
        }
    }

    let references = shared_ref_nodes(tree)
        .into_iter()
        .filter(|candidate| {
            nearest_containing_suite(suite_nodes, candidate).is_some_and(|owner| {
                owner.start_byte == node.start_byte && owner.end_byte == node.end_byte
            })
        })
        .collect::<Vec<_>>();
    lower_shared_refs(
        path,
        &full_name,
        references,
        shared_examples,
        shared_refs,
        &mut suite,
    );
    suite.fixtures = suite_fixture_nodes(tree, suite_nodes, node)
        .into_iter()
        .map(fixture_from_node)
        .collect();

    for child in suite_nodes.iter().copied().filter(|candidate| {
        nearest_containing_suite(suite_nodes, candidate).is_some_and(|owner| {
            owner.start_byte == node.start_byte && owner.end_byte == node.end_byte
        })
    }) {
        suite.nested.push(lower_suite_node(
            path,
            framework,
            tree,
            child,
            suite_nodes,
            case_nodes,
            shared_examples,
            shared_refs,
            Some(&full_name),
        ));
    }
    suite
}

fn nearest_containing_suite<'a>(
    suites: &'a [&SyntaxNode],
    target: &SyntaxNode,
) -> Option<&'a SyntaxNode> {
    suites
        .iter()
        .copied()
        .filter(|suite| {
            suite.start_byte <= target.start_byte
                && target.end_byte <= suite.end_byte
                && (suite.start_byte != target.start_byte || suite.end_byte != target.end_byte)
        })
        .min_by_key(|suite| suite.end_byte - suite.start_byte)
}

fn shared_ref_nodes(tree: &SyntaxTree) -> Vec<&SyntaxNode> {
    let shared_examples = shared_example_nodes(tree);
    tree.nodes
        .iter()
        .filter(|node| {
            matches!(
                call_name(node),
                Some("it_behaves_like" | "include_examples" | "include_context")
            )
        })
        .filter(|node| !inside_any_node(node, &shared_examples))
        .collect()
}

fn lower_shared_refs(
    path: &Path,
    suite_name: &str,
    references: Vec<&SyntaxNode>,
    shared_examples: &[SharedExample],
    shared_refs: &mut Vec<SharedExampleRef>,
    suite: &mut TestSuite,
) {
    for reference in references {
        let Some(name) = extract_name(&reference.text) else {
            continue;
        };
        shared_refs.push(SharedExampleRef {
            name: name.clone(),
            span: node_span(reference),
        });
        expand_shared_example_ref(
            path,
            suite_name,
            &name,
            reference.start_line,
            shared_examples,
            suite,
        );
    }
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
        .iter()
        .copied()
        .filter(|candidate| {
            !nodes.iter().any(|container| {
                container.start_byte < candidate.start_byte
                    && candidate.end_byte <= container.end_byte
            })
        })
        .collect()
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

    let assertion_spans = collect_case_ast(&mut case, tree, node);
    let sut_lines = case
        .sut_calls
        .iter()
        .filter_map(|call| {
            tree.nodes
                .iter()
                .find(|candidate| candidate.text == *call && is_sut_call_node(candidate))
                .map(|candidate| (candidate.start_line, candidate.end_line))
        })
        .collect::<Vec<_>>();

    for line in tree
        .lines
        .iter()
        .filter(|line| line.line >= span.start_line && line.line <= span.end_line)
    {
        let trimmed = line.text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let role = if assertion_spans
            .iter()
            .any(|assertion| assertion.start_line <= line.line && line.line <= assertion.end_line)
        {
            StatementRole::Assert
        } else if sut_lines
            .iter()
            .any(|(start, end)| *start <= line.line && line.line <= *end)
        {
            StatementRole::Act
        } else {
            StatementRole::Arrange
        };
        case.statements.push(Statement {
            text: trimmed.to_owned(),
            role,
            span: SourceSpan::line(line.line),
        });
        case.normalized_body.push(normalize_line(trimmed));
    }

    case.span = span;
    Some(case)
}

fn collect_case_ast(
    case: &mut TestCase,
    tree: &SyntaxTree,
    case_node: &SyntaxNode,
) -> Vec<SourceSpan> {
    let descendants = tree
        .nodes
        .iter()
        .filter(|node| {
            case_node.start_byte < node.start_byte && node.end_byte <= case_node.end_byte
        })
        .collect::<Vec<_>>();
    let assertion_nodes = descendants
        .iter()
        .copied()
        .filter(|node| is_assertion_node(node))
        .filter(|candidate| {
            !descendants.iter().any(|container| {
                is_assertion_node(container)
                    && container.start_byte < candidate.start_byte
                    && candidate.end_byte <= container.end_byte
            })
        })
        .collect::<Vec<_>>();
    let assertion_ranges = assertion_nodes
        .iter()
        .map(|node| (node.start_byte, node.end_byte))
        .collect::<Vec<_>>();

    for node in &assertion_nodes {
        case.assertions.push(parse_assertion_node(tree, node));
    }

    for node in descendants {
        let inside_assertion = assertion_ranges
            .iter()
            .any(|(start, end)| *start <= node.start_byte && node.end_byte <= *end);

        match node.kind.as_str() {
            "global_variable" => push_external_ref(case, ExternalRefKind::GlobalState, node),
            "if" | "unless" | "case" | "while" | "until" | "for" => {
                case.control_flow.push(node_span(node));
            }
            "number" => case.literals.push(LiteralValue {
                raw: node.text.clone(),
                kind: if matches!(node.text.as_str(), "0" | "1" | "-1") {
                    LiteralKind::Boundary
                } else {
                    LiteralKind::Number
                },
                span: node_span(node),
            }),
            "boundary" => case.literals.push(LiteralValue {
                raw: node.text.clone(),
                kind: LiteralKind::Boundary,
                span: node_span(node),
            }),
            "collection" if matches!(node.text.trim(), "[]" | "{}") => {
                case.literals.push(LiteralValue {
                    raw: "empty".to_owned(),
                    kind: LiteralKind::Boundary,
                    span: node_span(node),
                });
            }
            _ => {}
        }

        let Some(name) = call_name(node) else {
            continue;
        };
        case.calls.push(CallSite {
            method: name.to_owned(),
            expression: node.text.clone(),
            span: node_span(node),
        });
        if name == "pending" && !case.tags.iter().any(|tag| tag.kind == TagKind::Pending) {
            case.tags.push(Tag {
                kind: TagKind::Pending,
                label: "pending".to_owned(),
                span: node_span(node),
            });
        }
        collect_external_call(case, name, node);
        collect_double_call(case, name, node);
        if matches!(name, "puts" | "p" | "pp") {
            case.print_lines.push(node_span(node));
        }
        if !inside_assertion && is_sut_call_node(node) {
            case.sut_calls.push(node.text.clone());
        }
    }

    case.sut_calls.sort();
    case.sut_calls.dedup();
    case.calls
        .dedup_by(|left, right| left.method == right.method && left.span == right.span);
    assertion_nodes
        .into_iter()
        .map(node_span)
        .collect::<Vec<_>>()
}

fn is_assertion_node(node: &SyntaxNode) -> bool {
    let Some(name) = call_name(node) else {
        return false;
    };
    matches!(name, "to" | "not_to" | "to_not" | "should" | "should_not")
        || name.starts_with("assert")
        || name.starts_with("refute")
        || name.starts_with("must_")
        || name.starts_with("wont_")
        || name == "flunk"
}

fn parse_assertion_node(tree: &SyntaxTree, node: &SyntaxNode) -> Assertion {
    let name = call_name(node).unwrap_or("assert");
    if matches!(name, "to" | "not_to" | "to_not" | "should" | "should_not") {
        return parse_rspec_assertion_node(tree, node);
    }
    parse_assert_style_assertion_node(tree, node, name)
}

fn parse_assert_style_assertion_node(
    tree: &SyntaxTree,
    node: &SyntaxNode,
    matcher: &str,
) -> Assertion {
    let children = node
        .children
        .iter()
        .filter_map(|index| tree.nodes.get(*index))
        .collect::<Vec<_>>();
    let receiver = children
        .iter()
        .copied()
        .find(|child| is_receiver_of_call(node, child));
    let arguments = children
        .iter()
        .copied()
        .filter(|child| !is_receiver_of_call(node, child))
        .map(|child| child.text.trim().to_owned())
        .collect::<Vec<_>>();
    let (expected_expr, subject_expr, message_index) = if let Some(receiver) = receiver {
        (arguments.first().cloned(), receiver.text.clone(), 1)
    } else if matcher.contains("equal") && arguments.len() >= 2 {
        (Some(arguments[0].clone()), arguments[1].clone(), 2)
    } else {
        (
            arguments.first().cloned(),
            arguments.get(1).cloned().unwrap_or_default(),
            2,
        )
    };
    Assertion {
        kind: classify_assertion(matcher, ""),
        matcher: matcher.to_owned(),
        subject_expr,
        expected_expr,
        has_message: arguments.len() > message_index,
        span: node_span(node),
    }
}

fn parse_rspec_assertion_node(tree: &SyntaxTree, node: &SyntaxNode) -> Assertion {
    let direct_calls = node
        .children
        .iter()
        .filter_map(|index| tree.nodes.get(*index))
        .filter(|child| call_name(child).is_some())
        .collect::<Vec<_>>();
    let expect = direct_calls
        .iter()
        .copied()
        .find(|child| call_name(child) == Some("expect"));
    let matcher_node = direct_calls
        .iter()
        .copied()
        .rev()
        .find(|child| call_name(child) != Some("expect"));
    let matcher = matcher_node
        .and_then(call_name)
        .unwrap_or("matcher")
        .to_owned();
    let subject_expr = expect
        .and_then(|expect| first_argument_text(tree, expect))
        .or_else(|| balanced_group_after(&node.text, "expect", '(', ')'))
        .or_else(|| balanced_group_after(&node.text, "expect", '{', '}'))
        .unwrap_or_default();
    let expected_expr = matcher_node.and_then(|matcher| first_argument_text(tree, matcher));

    Assertion {
        kind: classify_assertion(&matcher, ""),
        matcher,
        subject_expr,
        expected_expr,
        has_message: node.text.contains("because(") || node.text.contains("failure_message"),
        span: node_span(node),
    }
}

fn first_argument_text(tree: &SyntaxTree, call: &SyntaxNode) -> Option<String> {
    call.children
        .iter()
        .filter_map(|index| tree.nodes.get(*index))
        .find(|child| {
            child.start_byte >= call.start_byte
                && child.end_byte <= call.end_byte
                && !is_receiver_of_call(call, child)
        })
        .map(|child| child.text.trim().to_owned())
}

fn is_receiver_of_call(call: &SyntaxNode, child: &SyntaxNode) -> bool {
    let Some(name) = call_name(call) else {
        return false;
    };
    let relative_message = call.text.rfind(name).unwrap_or(0);
    child.end_byte.saturating_sub(call.start_byte) <= relative_message
}

fn collect_external_call(case: &mut TestCase, name: &str, node: &SyntaxNode) {
    let text = node.text.trim_start();
    let kind = if name == "sleep" {
        Some(ExternalRefKind::Sleep)
    } else if matches!(name, "open" | "read" | "write" | "binread" | "binwrite")
        && starts_any(
            text,
            &["File.", "FileUtils.", "Dir.", "Kernel.open", "open("],
        )
    {
        Some(ExternalRefKind::FileSystem)
    } else if starts_any(
        text,
        &["Net::HTTP", "URI.open", "Faraday.", "HTTP.", "WebMock"],
    ) {
        Some(ExternalRefKind::Network)
    } else if starts_any(text, &["ActiveRecord::Base.connection", "Sequel."])
        || name == "find_by_sql"
    {
        Some(ExternalRefKind::Database)
    } else if matches!(name, "now" | "today" | "clock_gettime")
        && starts_any(text, &["Time.", "Date.", "DateTime.", "Process."])
    {
        Some(ExternalRefKind::Time)
    } else if text.starts_with("ENV[") || text.starts_with("Thread.current") {
        Some(ExternalRefKind::GlobalState)
    } else {
        None
    };
    if let Some(kind) = kind {
        push_external_ref(case, kind, node);
    }
}

fn push_external_ref(case: &mut TestCase, kind: ExternalRefKind, node: &SyntaxNode) {
    if case
        .external_refs
        .iter()
        .any(|reference| reference.kind == kind && reference.span.start_line == node.start_line)
    {
        return;
    }
    case.external_refs.push(ExternalRef {
        kind,
        expression: node.text.clone(),
        span: node_span(node),
    });
}

fn collect_double_call(case: &mut TestCase, name: &str, node: &SyntaxNode) {
    if !matches!(
        name,
        "double" | "instance_double" | "class_double" | "allow" | "stub"
    ) {
        return;
    }
    if case
        .doubles
        .iter()
        .any(|double| double.span.start_line == node.start_line)
    {
        return;
    }
    case.doubles.push(TestDouble {
        kind: name.to_owned(),
        span: node_span(node),
    });
}

fn is_sut_call_node(node: &SyntaxNode) -> bool {
    let Some(name) = call_name(node) else {
        return false;
    };
    node.text.contains('.')
        && !matches!(
            name,
            "to" | "not_to"
                | "to_not"
                | "should"
                | "should_not"
                | "expect"
                | "allow"
                | "receive"
                | "eq"
                | "eql"
                | "equal"
                | "include"
                | "raise_error"
                | "change"
                | "now"
                | "today"
                | "read"
                | "write"
                | "open"
        )
}

fn node_span(node: &SyntaxNode) -> SourceSpan {
    SourceSpan {
        start_line: node.start_line,
        end_line: node.end_line,
    }
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
        case.evidence_aliases.push(format!(
            "{suite_name} behaves like {ref_name} {}",
            template.name
        ));
        case.evidence_aliases.sort();
        case.evidence_aliases.dedup();
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
    if node.kind.starts_with("def:test_") {
        return true;
    }
    if !is_call_like(node) {
        return false;
    }
    matches!(
        call_name(node),
        Some("it" | "specify" | "example" | "scenario" | "xit" | "xspecify" | "pending" | "test")
    )
}

fn is_inside_spans(line: usize, spans: &[SourceSpan]) -> bool {
    spans
        .iter()
        .any(|span| line >= span.start_line && line <= span.end_line)
}

fn is_call_like(node: &SyntaxNode) -> bool {
    node.kind.starts_with("call:") || node.kind.starts_with("def:")
}

fn expanded_case_span(tree: &SyntaxTree, node: &SyntaxNode) -> SourceSpan {
    let _ = tree;
    SourceSpan {
        start_line: node.start_line,
        end_line: node.end_line,
    }
}

fn call_name(node: &SyntaxNode) -> Option<&str> {
    node.kind.strip_prefix("call:")
}

fn matcher(matcher: &str, assertion_kind: &str) -> MatcherSemantics {
    MatcherSemantics {
        matcher: matcher.to_owned(),
        assertion_kind: assertion_kind.to_owned(),
    }
}

fn detect_framework(content: &str) -> &'static str {
    if content.contains("minitest/autorun") || content.contains("Minitest::") {
        "minitest"
    } else if content.contains("Test::Unit") || content.contains("test/unit") {
        "test-unit"
    } else if content.contains("RSpec.describe")
        || content.contains("RSpec.context")
        || content.contains("expect(")
        || content.contains("describe ")
    {
        "rspec"
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

fn collect_file_metadata(
    tree: &SyntaxTree,
    case_nodes: &[&SyntaxNode],
    shared_nodes: &[&SyntaxNode],
    ir: &mut TestFileIr,
) {
    for node in &tree.nodes {
        if inside_any_node(node, case_nodes) {
            continue;
        }
        let inside_shared_example = inside_any_node(node, shared_nodes);
        if !inside_shared_example && let Some("let" | "let!" | "subject") = call_name(node) {
            ir.shared_fixtures.push(fixture_from_node(node));
        }
        let Some(name) = node.kind.strip_prefix("def:") else {
            continue;
        };
        if name.starts_with("test_") || name == "setup" || name == "teardown" {
            continue;
        }
        let contains_assertion = tree.nodes.iter().any(|candidate| {
            node.start_byte < candidate.start_byte
                && candidate.end_byte <= node.end_byte
                && is_assertion_node(candidate)
        });
        ir.helpers.push(HelperDef {
            name: name.to_owned(),
            span: node_span(node),
            contains_assertion,
        });
    }
    ir.shared_fixtures
        .dedup_by(|left, right| left.name == right.name && left.span == right.span);
}

fn inside_any_node(node: &SyntaxNode, containers: &[&SyntaxNode]) -> bool {
    containers.iter().any(|container| {
        container.start_byte <= node.start_byte && node.end_byte <= container.end_byte
    })
}

fn fixture_from_node(node: &SyntaxNode) -> Fixture {
    let method = call_name(node).or_else(|| node.kind.strip_prefix("def:"));
    let name = match method {
        Some("let" | "let!") => balanced_group_after(&node.text, "let", '(', ')')
            .map(|name| {
                name.trim()
                    .trim_start_matches(':')
                    .trim_matches(['\'', '"'])
                    .to_owned()
            })
            .unwrap_or_else(|| "let".to_owned()),
        Some("subject") => "subject".to_owned(),
        _ => "setup".to_owned(),
    };
    Fixture {
        name,
        span: node_span(node),
        eager: matches!(method, Some("let!" | "before" | "setup")),
    }
}

fn suite_fixture_nodes<'a>(
    tree: &'a SyntaxTree,
    suites: &[&SyntaxNode],
    owner: &SyntaxNode,
) -> Vec<&'a SyntaxNode> {
    tree.nodes
        .iter()
        .filter(|node| is_suite_fixture_node(node))
        .filter(|node| {
            nearest_containing_suite(suites, node).is_some_and(|suite| {
                suite.start_byte == owner.start_byte && suite.end_byte == owner.end_byte
            })
        })
        .collect()
}

fn suite_fixture_nodes_without_owner<'a>(
    tree: &'a SyntaxTree,
    suites: &[&SyntaxNode],
) -> Vec<&'a SyntaxNode> {
    tree.nodes
        .iter()
        .filter(|node| is_suite_fixture_node(node))
        .filter(|node| nearest_containing_suite(suites, node).is_none())
        .collect()
}

fn is_suite_fixture_node(node: &SyntaxNode) -> bool {
    matches!(call_name(node), Some("before" | "setup")) || node.kind == "def:setup"
}

fn start_case(
    path: &Path,
    _framework: &str,
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

    if !(method_case || test_block || rspec_case) {
        return None;
    }

    let name = if let Some(method) = trimmed.strip_prefix("def test_") {
        format!(
            "test {}",
            method
                .split(['(', ' ', ';'])
                .next()
                .unwrap_or(method)
                .replace('_', " ")
        )
    } else {
        extract_name(trimmed).unwrap_or_else(|| format!("(anonymous at line {line_no})"))
    };
    let id = stable_test_id(path, suite, &name, line_no);
    let mut case = TestCase::new(id, name, SourceSpan::line(line_no));
    case.evidence_aliases = evidence_aliases(&case.id, suite, &case.name);
    if let Some(method) = trimmed.strip_prefix("def ") {
        let method = method.split(['(', ' ', ';']).next().unwrap_or(method);
        case.evidence_aliases.push(method.to_owned());
        case.evidence_aliases.push(format!("{suite}#{method}"));
        case.evidence_aliases.sort();
        case.evidence_aliases.dedup();
    }

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

fn classify_assertion(matcher: &str, _line: &str) -> AssertionKind {
    if matches!(
        matcher,
        "eq" | "equal" | "eql" | "same" | "==" | "assert_equal" | "assert_same"
    ) {
        AssertionKind::Equality
    } else if matcher.contains("raise") || matcher.contains("throw") {
        AssertionKind::Exception
    } else if matcher == "change" || matcher.contains("difference") {
        AssertionKind::Change
    } else if matches!(
        matcher,
        "receive" | "have_received" | "assert_mock" | "verify"
    ) {
        AssertionKind::MockVerification
    } else if matches!(matcher, "include" | "contain" | "be_empty" | "assert_empty") {
        AssertionKind::Collection
    } else if matches!(matcher, "snapshot" | "match_snapshot") {
        AssertionKind::Snapshot
    } else if matcher == "be"
        || matcher.starts_with("be_")
        || matcher.starts_with("assert")
        || matcher.starts_with("refute")
    {
        AssertionKind::Predicate
    } else {
        AssertionKind::Other
    }
}

fn normalize_line(line: &str) -> String {
    let characters = line.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(line.len());
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if matches!(character, '\'' | '"') {
            normalized.push_str("<str>");
            let quote = character;
            index += 1;
            while index < characters.len() {
                if characters[index] == '\\' {
                    index += 2;
                } else if characters[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if character.is_ascii_digit()
            && index
                .checked_sub(1)
                .is_none_or(|previous| !is_ruby_identifier(characters[previous]))
        {
            normalized.push_str("<num>");
            index += 1;
            while index < characters.len()
                && (characters[index].is_ascii_digit() || characters[index] == '.')
            {
                index += 1;
            }
            continue;
        }
        if character == ':'
            && characters
                .get(index + 1)
                .is_some_and(|next| is_ruby_identifier(*next))
        {
            normalized.push_str("<sym>");
            index += 2;
            while index < characters.len() && is_ruby_identifier(characters[index]) {
                index += 1;
            }
            continue;
        }
        normalized.push(character);
        index += 1;
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_ruby_identifier(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn extract_name(line: &str) -> Option<String> {
    let header = line.lines().next().unwrap_or(line);
    extract_between(header, "\"", "\"")
        .or_else(|| extract_between(header, "'", "'"))
        .or_else(|| header.split_whitespace().nth(1).map(ToOwned::to_owned))
}

fn extract_between(line: &str, start: &str, end: &str) -> Option<String> {
    let (_, rest) = line.split_once(start)?;
    let (value, _) = rest.split_once(end)?;
    Some(value.trim().to_owned())
}

fn balanced_group_after(text: &str, marker: &str, opening: char, closing: char) -> Option<String> {
    let marker_start = text.find(marker)?;
    let opening_offset = text[marker_start + marker.len()..].find(opening)?;
    let content_start = marker_start + marker.len() + opening_offset + opening.len_utf8();
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;

    for (relative, character) in text[content_start..].char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == opening {
            depth += 1;
        } else if character == closing {
            depth -= 1;
            if depth == 0 {
                return Some(
                    text[content_start..content_start + relative]
                        .trim()
                        .to_owned(),
                );
            }
        }
    }
    None
}

fn starts_any(line: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| line.starts_with(needle))
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
    fn parses_with_prism_and_marks_confidence() {
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
        assert!(tree.nodes.iter().any(|node| node.kind == "call:expect"));
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
        assert!(
            ir.cases()[0]
                .evidence_aliases
                .contains(&"Product behaves like priced thing has a price".to_owned()),
            "{:?}",
            ir.cases()[0].evidence_aliases
        );
    }

    #[test]
    fn uses_ast_boundaries_for_nested_and_multiline_assertions() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Order do
                  it "works" do
                    pending "later"
                    expect(order.total(with_tax: true))
                      .to eq(0)
                    assert_equal foo(1, 2), bar
                  end
                end
                "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        assert_eq!(ir.case_count(), 1);
        let case = ir.cases()[0];
        assert_eq!(case.assertions.len(), 2);
        assert_eq!(
            case.assertions[0].subject_expr,
            "order.total(with_tax: true)"
        );
        assert_eq!(case.assertions[0].matcher, "eq");
        assert_eq!(
            case.assertions[1].expected_expr.as_deref(),
            Some("foo(1, 2)")
        );
        assert_eq!(case.assertions[1].subject_expr, "bar");
        assert!(!case.assertions[1].has_message);
        assert!(case.tags.iter().any(|tag| tag.kind == TagKind::Pending));
    }

    #[test]
    fn classifies_matchers_without_subject_substring_false_positives() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Sequence do
                  it "classifies" do
                    expect(sequence).to be_valid
                    expect(sequence).to be_empty
                  end
                end
                "#,
            )
            .unwrap();
        let ir =
            FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/sequence_spec.rb")).unwrap();
        let assertions = &ir.cases()[0].assertions;

        assert_eq!(assertions[0].matcher, "be_valid");
        assert_eq!(assertions[0].kind, AssertionKind::Predicate);
        assert_eq!(assertions[1].kind, AssertionKind::Collection);
    }

    #[test]
    fn ignores_code_like_text_and_assertion_receives_for_smell_signals() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Price do
                  it "checks text" do
                    message = "please sleep 1 second and $100"
                    expect(message).to match(/foo$/)
                    expect(service).to receive(:call)
                    value = 3.14
                  end
                end
                "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/price_spec.rb")).unwrap();
        let case = ir.cases()[0];

        assert!(case.external_refs.is_empty());
        assert!(case.doubles.is_empty());
        assert!(case.sut_calls.is_empty());
    }

    #[test]
    fn minitest_markers_take_priority_over_describe() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                require "minitest/autorun"
                describe Order do
                  it "works" do
                    _(1).must_equal 1
                  end
                end
                "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("test/order_test.rb")).unwrap();

        assert_eq!(ir.framework, "minitest");
        assert_eq!(ir.case_count(), 1);
        assert_eq!(ir.cases()[0].assertions[0].matcher, "must_equal");
        assert_eq!(ir.cases()[0].assertions[0].kind, AssertionKind::Equality);
    }

    #[test]
    fn normalizes_literals_inside_call_syntax() {
        assert_eq!(
            normalize_line(r#"expect(user.name).to eq("Alice")"#),
            "expect(user.name).to eq(<str>)"
        );
        assert_eq!(
            normalize_line("assert_equal foo(1.5), :ready"),
            "assert_equal foo(<num>), <sym>"
        );
    }

    #[test]
    fn materializes_nested_suite_hierarchy() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Order do
                  context "when empty" do
                    it "is invalid" do
                      expect(subject).to be_invalid
                    end
                  end
                end
                "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        assert_eq!(ir.suites.len(), 1);
        assert_eq!(ir.suites[0].name, "Order");
        assert_eq!(ir.suites[0].nested.len(), 1);
        assert_eq!(ir.suites[0].nested[0].name, "when empty");
        assert_eq!(ir.suites[0].nested[0].cases.len(), 1);
        assert!(
            ir.suites[0].nested[0].cases[0]
                .evidence_aliases
                .contains(&"Order when empty is invalid".to_owned())
        );
    }

    #[test]
    fn records_asserting_helpers_and_case_calls() {
        let adapter = RubyAdapter;
        let tree = adapter
            .parse(
                br#"
                RSpec.describe Order do
                  def expect_valid_order(order)
                    expect(order).to be_valid
                  end

                  it "is valid" do
                    expect_valid_order(subject)
                  end
                end
                "#,
            )
            .unwrap();
        let ir = FrameworkAdapter::lower(&adapter, &tree, Path::new("spec/order_spec.rb")).unwrap();

        assert!(ir.helpers[0].contains_assertion);
        assert!(
            ir.cases()[0]
                .calls
                .iter()
                .any(|call| call.method == "expect_valid_order")
        );
    }
}
