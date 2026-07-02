use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum Axis {
    Adequacy,
    Redundancy,
    Maintainability,
}

impl Axis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adequacy => "adequacy",
            Self::Redundancy => "redundancy",
            Self::Maintainability => "maintainability",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum Confidence {
    Exact,
    Approximate,
    Unresolved,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Approximate => "approximate",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct SourceSpan {
    pub start_line: usize,
    pub end_line: usize,
}

impl SourceSpan {
    pub fn line(line: usize) -> Self {
        Self {
            start_line: line,
            end_line: line,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TestFileIr {
    pub path: PathBuf,
    pub language: String,
    pub framework: String,
    pub suites: Vec<TestSuite>,
    pub shared_fixtures: Vec<Fixture>,
    pub shared_examples: Vec<SharedExample>,
    pub shared_example_refs: Vec<SharedExampleRef>,
    pub helpers: Vec<HelperDef>,
    pub subject_hints: Vec<SubjectHint>,
    pub confidence: Confidence,
}

impl TestFileIr {
    pub fn new(path: impl Into<PathBuf>, language: &str, framework: &str) -> Self {
        Self {
            path: path.into(),
            language: language.to_owned(),
            framework: framework.to_owned(),
            suites: Vec::new(),
            shared_fixtures: Vec::new(),
            shared_examples: Vec::new(),
            shared_example_refs: Vec::new(),
            helpers: Vec::new(),
            subject_hints: Vec::new(),
            confidence: Confidence::Approximate,
        }
    }

    pub fn path_display(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    pub fn cases(&self) -> Vec<&TestCase> {
        let mut cases = Vec::new();
        for suite in &self.suites {
            suite.collect_cases(&mut cases);
        }
        cases
    }

    pub fn case_count(&self) -> usize {
        self.suites.iter().map(TestSuite::case_count).sum()
    }

    pub fn assertion_count(&self) -> usize {
        self.cases().iter().map(|case| case.assertions.len()).sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TestSuite {
    pub name: String,
    pub span: SourceSpan,
    pub cases: Vec<TestCase>,
    pub nested: Vec<TestSuite>,
    pub fixtures: Vec<Fixture>,
}

impl TestSuite {
    pub fn new(name: impl Into<String>, span: SourceSpan) -> Self {
        Self {
            name: name.into(),
            span,
            cases: Vec::new(),
            nested: Vec::new(),
            fixtures: Vec::new(),
        }
    }

    fn collect_cases<'a>(&'a self, cases: &mut Vec<&'a TestCase>) {
        cases.extend(self.cases.iter());
        for suite in &self.nested {
            suite.collect_cases(cases);
        }
    }

    fn case_count(&self) -> usize {
        self.cases.len() + self.nested.iter().map(TestSuite::case_count).sum::<usize>()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TestCase {
    pub id: String,
    pub evidence_aliases: Vec<String>,
    pub name: String,
    pub span: SourceSpan,
    pub statements: Vec<Statement>,
    pub assertions: Vec<Assertion>,
    pub doubles: Vec<TestDouble>,
    pub external_refs: Vec<ExternalRef>,
    pub tags: Vec<Tag>,
    pub sut_calls: Vec<String>,
    pub control_flow: Vec<SourceSpan>,
    pub print_lines: Vec<SourceSpan>,
    pub literals: Vec<LiteralValue>,
    pub normalized_body: Vec<String>,
}

impl TestCase {
    pub fn new(id: impl Into<String>, name: impl Into<String>, span: SourceSpan) -> Self {
        Self {
            id: id.into(),
            evidence_aliases: Vec::new(),
            name: name.into(),
            span,
            statements: Vec::new(),
            assertions: Vec::new(),
            doubles: Vec::new(),
            external_refs: Vec::new(),
            tags: Vec::new(),
            sut_calls: Vec::new(),
            control_flow: Vec::new(),
            print_lines: Vec::new(),
            literals: Vec::new(),
            normalized_body: Vec::new(),
        }
    }

    pub fn has_tag(&self, kind: TagKind) -> bool {
        self.tags.iter().any(|tag| tag.kind == kind)
    }
}

pub fn resolve_test_case_id(cases: &[&TestCase], raw_key: &str) -> Option<String> {
    for case in cases {
        if case.id == raw_key || case.evidence_aliases.iter().any(|alias| alias == raw_key) {
            return Some(case.id.clone());
        }
    }

    let normalized_key = normalize_evidence_key(raw_key);
    for case in cases {
        if case
            .evidence_aliases
            .iter()
            .any(|alias| normalize_evidence_key(alias) == normalized_key)
        {
            return Some(case.id.clone());
        }
    }

    let matches = cases
        .iter()
        .filter(|case| {
            case.evidence_aliases.iter().any(|alias| {
                let normalized_alias = normalize_evidence_key(alias);
                !normalized_alias.is_empty()
                    && (normalized_key.ends_with(&normalized_alias)
                        || normalized_key.contains(&normalized_alias))
            })
        })
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Some(matches[0].id.clone());
    }
    None
}

fn normalize_evidence_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Statement {
    pub text: String,
    pub role: StatementRole,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum StatementRole {
    Arrange,
    Act,
    Assert,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Assertion {
    pub kind: AssertionKind,
    pub matcher: String,
    pub subject_expr: String,
    pub expected_expr: Option<String>,
    pub has_message: bool,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Deserialize, Serialize)]
pub enum AssertionKind {
    Equality,
    Predicate,
    Exception,
    Change,
    Snapshot,
    MockVerification,
    Collection,
    Other,
}

impl AssertionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equality => "equality",
            Self::Predicate => "predicate",
            Self::Exception => "exception",
            Self::Change => "change",
            Self::Snapshot => "snapshot",
            Self::MockVerification => "mock_verification",
            Self::Collection => "collection",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TestDouble {
    pub kind: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExternalRef {
    pub kind: ExternalRefKind,
    pub expression: String,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ExternalRefKind {
    FileSystem,
    Network,
    Database,
    Time,
    Sleep,
    GlobalState,
}

impl ExternalRefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileSystem => "file_system",
            Self::Network => "network",
            Self::Database => "database",
            Self::Time => "time",
            Self::Sleep => "sleep",
            Self::GlobalState => "global_state",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Tag {
    pub kind: TagKind,
    pub label: String,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TagKind {
    Skipped,
    Pending,
    Focus,
}

impl TagKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Pending => "pending",
            Self::Focus => "focus",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Fixture {
    pub name: String,
    pub span: SourceSpan,
    pub eager: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct HelperDef {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SubjectHint {
    pub path: PathBuf,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LiteralValue {
    pub raw: String,
    pub kind: LiteralKind,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum LiteralKind {
    Boundary,
    Number,
    String,
    Nil,
    Boolean,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SharedExample {
    pub name: String,
    pub span: SourceSpan,
    pub cases: Vec<TestCase>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SharedExampleRef {
    pub name: String,
    pub span: SourceSpan,
}

pub fn stable_test_id(path: &Path, suite: &str, name: &str, line: usize) -> String {
    let input = format!("{}::{suite}::{name}::{line}", path.to_string_lossy());
    format!("{:016x}", fnv1a_64(input.as_bytes()))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
