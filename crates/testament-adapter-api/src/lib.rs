use std::path::Path;

use testament_core::{EvidenceSet, TestFileIr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct DetectScore(pub u8);

impl DetectScore {
    pub const NONE: Self = Self(0);
    pub const LOW: Self = Self(25);
    pub const MEDIUM: Self = Self(60);
    pub const HIGH: Self = Self(90);
    pub const EXACT: Self = Self(100);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxTree {
    pub language: String,
    pub lines: Vec<SyntaxLine>,
    pub root_kind: String,
    pub has_error: bool,
    pub nodes: Vec<SyntaxNode>,
}

impl SyntaxTree {
    pub fn new(language: impl Into<String>, content: &[u8]) -> Self {
        Self::from_parts(language, content, String::new(), false)
    }

    pub fn from_parts(
        language: impl Into<String>,
        content: &[u8],
        root_kind: impl Into<String>,
        has_error: bool,
    ) -> Self {
        let text = String::from_utf8_lossy(content);
        Self {
            language: language.into(),
            lines: text
                .lines()
                .enumerate()
                .map(|(index, text)| SyntaxLine {
                    line: index + 1,
                    text: text.to_owned(),
                })
                .collect(),
            root_kind: root_kind.into(),
            has_error,
            nodes: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxLine {
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNode {
    pub kind: String,
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterError {
    pub message: String,
}

impl AdapterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub type AdapterResult<T> = Result<T, AdapterError>;

pub trait LanguageAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn detect(&self, path: &Path, content: &[u8]) -> DetectScore;
    fn parse(&self, content: &[u8]) -> AdapterResult<SyntaxTree>;
}

pub trait FrameworkAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn language(&self) -> &'static str;
    fn detect(&self, tree: &SyntaxTree, path: &Path) -> DetectScore;
    fn lower(&self, tree: &SyntaxTree, path: &Path) -> AdapterResult<TestFileIr>;
    fn semantics(&self) -> FrameworkSemantics;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameworkSemantics {
    pub assertion_matchers: Vec<MatcherSemantics>,
    pub skip_markers: Vec<String>,
    pub fixture_markers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatcherSemantics {
    pub matcher: String,
    pub assertion_kind: String,
}

pub trait EvidenceProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn load(&self, input: &Path) -> AdapterResult<EvidenceSet>;
}
