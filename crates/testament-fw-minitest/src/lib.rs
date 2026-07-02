use std::path::Path;

use testament_adapter_api::{
    AdapterResult, DetectScore, FrameworkAdapter, FrameworkSemantics, MatcherSemantics, SyntaxTree,
};
use testament_core::TestFileIr;
use testament_lang_ruby::RubyAdapter;

pub struct MinitestAdapter;

impl FrameworkAdapter for MinitestAdapter {
    fn id(&self) -> &'static str {
        "minitest"
    }

    fn language(&self) -> &'static str {
        "ruby"
    }

    fn detect(&self, tree: &SyntaxTree, path: &Path) -> DetectScore {
        let content = tree.text();
        if content.contains("Minitest::")
            || content.contains("minitest/autorun")
            || path.to_string_lossy().ends_with("_test.rb")
        {
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
                matcher("refute", "predicate"),
                matcher("assert_raises", "exception"),
                matcher("must_equal", "equality"),
                matcher("wont_equal", "equality"),
            ],
            skip_markers: vec!["skip".to_owned()],
            fixture_markers: vec!["setup".to_owned(), "before".to_owned()],
        }
    }
}

fn matcher(matcher: &str, assertion_kind: &str) -> MatcherSemantics {
    MatcherSemantics {
        matcher: matcher.to_owned(),
        assertion_kind: assertion_kind.to_owned(),
    }
}
