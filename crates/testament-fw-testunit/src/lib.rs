use std::path::Path;

use testament_adapter_api::{
    AdapterResult, DetectScore, FrameworkAdapter, FrameworkSemantics, MatcherSemantics, SyntaxTree,
};
use testament_core::TestFileIr;
use testament_lang_ruby::RubyAdapter;

pub struct TestUnitAdapter;

impl FrameworkAdapter for TestUnitAdapter {
    fn id(&self) -> &'static str {
        "test-unit"
    }

    fn language(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, tree: &SyntaxTree, _path: &Path) -> DetectScore {
        let content = tree.text();
        if content.contains("Test::Unit") || content.contains("test/unit") {
            DetectScore::HIGH
        } else {
            DetectScore::NONE
        }
    }

    fn lower(&self, tree: &SyntaxTree, path: &Path) -> AdapterResult<TestFileIr> {
        Ok(RubyAdapter::lower(path, &tree.text()))
    }

    fn semantics(&self) -> FrameworkSemantics {
        FrameworkSemantics {
            assertion_matchers: vec![
                matcher("assert_equal", "equality"),
                matcher("assert", "predicate"),
                matcher("assert_raise", "exception"),
                matcher("assert_include", "collection"),
            ],
            skip_markers: vec!["omit".to_owned(), "pend".to_owned()],
            fixture_markers: vec!["setup".to_owned(), "teardown".to_owned()],
        }
    }
}

fn matcher(matcher: &str, assertion_kind: &str) -> MatcherSemantics {
    MatcherSemantics {
        matcher: matcher.to_owned(),
        assertion_kind: assertion_kind.to_owned(),
    }
}
