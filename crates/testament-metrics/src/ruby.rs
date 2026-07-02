use std::path::{Path, PathBuf};

use testament_core::{
    Assertion, AssertionKind, Confidence, ExternalRef, ExternalRefKind, Fixture, HelperDef,
    LiteralKind, LiteralValue, SourceSpan, Statement, StatementRole, SubjectHint, Tag, TagKind,
    TestCase, TestDouble, TestFileIr, TestSuite, stable_test_id,
};

pub struct RubyAdapter;

impl RubyAdapter {
    pub fn lower(path: &Path, content: &str) -> TestFileIr {
        let framework = detect_framework(content);
        let mut ir = TestFileIr::new(path, "ruby", framework);
        ir.subject_hints.extend(infer_subject_hints(path));

        let suite_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("ruby tests")
            .to_owned();
        let mut suite = TestSuite::new(suite_name.clone(), SourceSpan::line(1));
        let mut current_suite = suite_name;
        let mut current = None::<CaseBuilder>;

        for (index, line) in content.lines().enumerate() {
            let line_no = index + 1;
            let trimmed = line.trim();

            if current.is_none() {
                update_suite_hint(trimmed, &mut current_suite);
                collect_fixture_or_helper(trimmed, line_no, &mut ir, &mut suite);
                if let Some(case) = start_case(path, framework, &current_suite, trimmed, line_no) {
                    current = Some(case);
                }
            }

            if let Some(builder) = current.as_mut() {
                collect_case_line(&mut builder.case, trimmed, line_no);
                builder.depth += block_delta(trimmed, builder.case.span.start_line == line_no);
                if builder.depth <= 0 {
                    let mut finished = current.take().expect("case exists");
                    finished.case.span.end_line = line_no;
                    suite.cases.push(finished.case);
                }
            }
        }

        if let Some(mut builder) = current {
            builder.case.span.end_line = content.lines().count().max(builder.case.span.start_line);
            suite.cases.push(builder.case);
        }

        suite.span.end_line = content.lines().count().max(1);
        ir.suites.push(suite);
        ir
    }
}

struct CaseBuilder {
    case: TestCase,
    depth: isize,
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
    let Some(candidate) = normalized
        .strip_prefix("spec/")
        .and_then(|path| path.strip_suffix("_spec.rb"))
        .or_else(|| normalized.strip_prefix("test/").and_then(|path| path.strip_suffix("_test.rb")))
    else {
        return Vec::new();
    };

    let candidate = candidate.strip_prefix("test_").unwrap_or(candidate);
    vec![SubjectHint {
        path: PathBuf::from(format!("lib/{candidate}.rb")),
        confidence: Confidence::Approximate,
    }]
}

fn update_suite_hint(trimmed: &str, current_suite: &mut String) {
    if starts_any(trimmed, &["RSpec.describe", "describe ", "context ", "class "]) {
        *current_suite = extract_name(trimmed).unwrap_or_else(|| trimmed.to_owned());
    }
}

fn collect_fixture_or_helper(trimmed: &str, line_no: usize, ir: &mut TestFileIr, suite: &mut TestSuite) {
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

fn start_case(
    path: &Path,
    framework: &str,
    suite: &str,
    trimmed: &str,
    line_no: usize,
) -> Option<CaseBuilder> {
    let skipped = starts_any(trimmed, &["xit ", "xit(", "xspecify ", "skip "]);
    let pending = starts_any(trimmed, &["pending ", "pending("]);
    let rspec_case = starts_any(
        trimmed,
        &["it ", "it(", "specify ", "specify(", "example ", "example(", "scenario "],
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

    Some(CaseBuilder { case, depth: 1 })
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
    let expected_expr = extract_between_after(trimmed, &matcher, "(", ")")
        .filter(|value| !value.is_empty());

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
        (args.first().cloned(), args.get(1).cloned().unwrap_or_default())
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
    } else if contains_any(&haystack, &["receive", "have_received", "assert_mock", "verify"]) {
        AssertionKind::MockVerification
    } else if contains_any(&haystack, &["include", "empty", "contain"]) {
        AssertionKind::Collection
    } else if contains_any(&haystack, &["snapshot", "match_snapshot"]) {
        AssertionKind::Snapshot
    } else if matcher.starts_with("be") || matcher.starts_with("assert") || matcher.starts_with("refute") {
        AssertionKind::Predicate
    } else {
        AssertionKind::Other
    }
}

fn collect_external_refs(case: &mut TestCase, trimmed: &str, line_no: usize) {
    for (kind, needles) in [
        (ExternalRefKind::FileSystem, &["File.", "FileUtils.", "Dir.", "open("][..]),
        (ExternalRefKind::Network, &["Net::HTTP", "URI.open", "Faraday.", "HTTP.", "WebMock"][..]),
        (ExternalRefKind::Database, &["ActiveRecord::Base.connection", ".find_by_sql", "Sequel."][..]),
        (ExternalRefKind::Time, &["Time.now", "Date.today", "DateTime.now", "Process.clock_gettime"][..]),
        (ExternalRefKind::Sleep, &["sleep ", "sleep("][..]),
        (ExternalRefKind::GlobalState, &["@@", "$", "ENV[", "Thread.current"][..]),
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
    for needle in ["double(", "instance_double", "class_double", "allow(", "receive(", "Minitest::Mock", ".stub("] {
        if trimmed.contains(needle) {
            case.doubles.push(TestDouble {
                kind: needle.trim_end_matches('(').to_owned(),
                span: SourceSpan::line(line_no),
            });
        }
    }
}

fn collect_misc_signals(case: &mut TestCase, trimmed: &str, line_no: usize) {
    if starts_any(trimmed, &["if ", "unless ", "case ", "while ", "until ", "for "])
        || trimmed.contains(".each do")
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
                &["expect.", "allow.", "Time.", "Date.", "File.", "Dir.", "Net.", "URI."],
            )
        })
        .filter(|token| !contains_any(token, &[".to", ".not_to", ".should", ".must_", ".wont_"]))
        .map(ToOwned::to_owned)
        .collect()
}

fn extract_literals(line: &str, line_no: usize) -> Vec<LiteralValue> {
    let mut literals = Vec::new();
    for token in line.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '-' || character == '_' || character == '.')
    }) {
        let kind = if matches!(token, "nil" | "true" | "false") {
            Some(LiteralKind::Boundary)
        } else if matches!(token, "0" | "1" | "-1") {
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

fn block_delta(trimmed: &str, is_start: bool) -> isize {
    let mut delta = 0;
    if !is_start && (trimmed.ends_with(" do") || trimmed.contains(" do |") || trimmed.starts_with("def ")) {
        delta += 1;
    }
    if trimmed == "end" || trimmed.starts_with("end ") {
        delta -= 1;
    }
    delta
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

    #[test]
    fn lowers_rspec_cases_and_smell_signals() {
        let ir = RubyAdapter::lower(
            Path::new("spec/order_spec.rb"),
            r#"
            RSpec.describe Order do
              let(:order) { described_class.new }

              it "waits" do
                sleep 1
                expect(order.total).to eq(0)
              end
            end
            "#,
        );

        let case = ir.cases()[0];
        assert_eq!(ir.framework, "rspec");
        assert_eq!(case.assertions.len(), 1);
        assert_eq!(case.external_refs[0].kind, ExternalRefKind::Sleep);
        assert_eq!(ir.shared_fixtures[0].name, "order");
    }
}

