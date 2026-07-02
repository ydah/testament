use std::path::Path;

use testament_adapter_api::{DetectScore, FrameworkAdapter, LanguageAdapter, SyntaxTree};
use testament_core::{Confidence, TestFileIr};
use testament_fw_minitest::MinitestAdapter;
use testament_fw_rspec::RSpecAdapter;
use testament_fw_testunit::TestUnitAdapter;
use testament_lang_ruby::RubyAdapter;

pub struct AdapterRegistry {
    languages: Vec<Box<dyn LanguageAdapter>>,
    frameworks: Vec<Box<dyn FrameworkAdapter>>,
}

impl AdapterRegistry {
    pub fn builtin() -> Self {
        Self {
            languages: vec![Box::new(RubyAdapter)],
            frameworks: vec![
                Box::new(RSpecAdapter),
                Box::new(MinitestAdapter),
                Box::new(TestUnitAdapter),
                Box::new(RubyAdapter),
            ],
        }
    }

    pub fn lower(&self, path: &Path, content: &str) -> TestFileIr {
        let Some(language) = best_language(&self.languages, path, content.as_bytes()) else {
            return unresolved_ir(path, "unknown", "unknown");
        };
        let tree = match language.parse(content.as_bytes()) {
            Ok(tree) => tree,
            Err(_) => return unresolved_ir(path, language.id(), language.id()),
        };
        let Some(framework) = best_framework(&self.frameworks, &tree, path) else {
            return unresolved_ir(path, language.id(), language.id());
        };
        framework
            .lower(&tree, path)
            .unwrap_or_else(|_| unresolved_ir(path, language.id(), framework.id()))
    }
}

fn unresolved_ir(path: &Path, language: &str, framework: &str) -> TestFileIr {
    let mut ir = TestFileIr::new(path, language, framework);
    ir.confidence = Confidence::Unresolved;
    ir
}

fn best_language<'a>(
    adapters: &'a [Box<dyn LanguageAdapter>],
    path: &Path,
    content: &[u8],
) -> Option<&'a dyn LanguageAdapter> {
    adapters
        .iter()
        .map(|adapter| {
            (
                adapter.as_ref(),
                LanguageAdapter::detect(adapter.as_ref(), path, content),
            )
        })
        .filter(|(_, score)| *score > DetectScore::NONE)
        .max_by_key(|(_, score)| *score)
        .map(|(adapter, _)| adapter)
}

fn best_framework<'a>(
    adapters: &'a [Box<dyn FrameworkAdapter>],
    tree: &SyntaxTree,
    path: &Path,
) -> Option<&'a dyn FrameworkAdapter> {
    adapters
        .iter()
        .filter(|adapter| adapter.language() == tree.language.as_str())
        .map(|adapter| {
            (
                adapter.as_ref(),
                FrameworkAdapter::detect(adapter.as_ref(), tree, path),
            )
        })
        .filter(|(_, score)| *score > DetectScore::NONE)
        .max_by_key(|(_, score)| *score)
        .map(|(adapter, _)| adapter)
}
