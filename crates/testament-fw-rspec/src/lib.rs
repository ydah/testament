//! RSpec detection and framework semantics for testament's Ruby adapter.

use std::path::Path;

use testament_adapter_api::{
    AdapterResult, DetectScore, FrameworkAdapter, FrameworkSemantics, MatcherSemantics, SyntaxTree,
};
use testament_core::TestFileIr;
use testament_lang_ruby::{RubyAdapter, apply_framework_semantics};

pub struct RSpecAdapter;

impl FrameworkAdapter for RSpecAdapter {
    fn id(&self) -> &'static str {
        "rspec"
    }

    fn language(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, tree: &SyntaxTree, path: &Path) -> DetectScore {
        let content = tree.text();
        if content.contains("minitest/autorun")
            || content.contains("Minitest::")
            || content.contains("Test::Unit")
            || content.contains("test/unit")
        {
            return DetectScore::NONE;
        }
        if content.contains("RSpec.describe")
            || content.contains("expect(")
            || path.to_string_lossy().ends_with("_spec.rb")
        {
            DetectScore::HIGH
        } else {
            DetectScore::NONE
        }
    }

    fn lower(&self, tree: &SyntaxTree, path: &Path) -> AdapterResult<TestFileIr> {
        let mut ir = testament_adapter_api::FrameworkAdapter::lower(&RubyAdapter, tree, path)?;
        ir.framework = self.id().to_owned();
        apply_framework_semantics(&mut ir, &self.semantics());
        Ok(ir)
    }

    fn semantics(&self) -> FrameworkSemantics {
        FrameworkSemantics {
            assertion_matchers: vec![
                matcher("eq", "equality"),
                matcher("eql", "equality"),
                matcher("raise_error", "exception"),
                matcher("change", "change"),
                matcher("receive", "mock_verification"),
                matcher("include", "collection"),
            ],
            skip_markers: vec!["skip".to_owned(), "pending".to_owned(), "xit".to_owned()],
            fixture_markers: vec!["before".to_owned(), "let".to_owned(), "let!".to_owned()],
        }
    }
}

fn matcher(matcher: &str, assertion_kind: &str) -> MatcherSemantics {
    MatcherSemantics {
        matcher: matcher.to_owned(),
        assertion_kind: assertion_kind.to_owned(),
    }
}
