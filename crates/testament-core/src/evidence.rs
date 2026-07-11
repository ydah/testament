use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvidenceSet {
    pub coverage: Option<CoverageEvidence>,
    pub mutation: Option<MutationEvidence>,
    pub per_test_coverage: Option<PerTestCoverageEvidence>,
    pub trace: Option<TraceEvidence>,
    pub sources: Vec<EvidenceSource>,
    pub warnings: Vec<String>,
}

impl EvidenceSet {
    pub fn is_empty(&self) -> bool {
        self.coverage.is_none()
            && self.mutation.is_none()
            && self.per_test_coverage.is_none()
            && self.trace.is_none()
    }

    pub fn merge(&mut self, other: EvidenceSet) {
        if let Some(other_coverage) = other.coverage {
            self.coverage
                .get_or_insert_with(CoverageEvidence::default)
                .files
                .extend(other_coverage.files);
        }
        if let Some(other_mutation) = other.mutation {
            merge_mutation(
                self.mutation.get_or_insert_with(MutationEvidence::default),
                other_mutation,
            );
        }
        if let Some(other_coverage) = other.per_test_coverage {
            let coverage = self
                .per_test_coverage
                .get_or_insert_with(PerTestCoverageEvidence::default);
            for (case, requirements) in other_coverage.cases {
                coverage.cases.entry(case).or_default().extend(requirements);
            }
        }
        if let Some(other_trace) = other.trace {
            let trace = self.trace.get_or_insert_with(TraceEvidence::default);
            for (case, evidence) in other_trace.cases {
                let current = trace.cases.entry(case).or_default();
                current.executed_lines.extend(evidence.executed_lines);
                current.checked_lines.extend(evidence.checked_lines);
            }
        }
        self.sources.extend(other.sources);
        self.warnings.extend(other.warnings);
    }
}

fn merge_mutation(current: &mut MutationEvidence, other: MutationEvidence) {
    current.total += other.total;
    current.killed += other.killed;
    current.equivalent_marked += other.equivalent_marked;
    if current.total == 0 {
        current.score_override = other.score_override.or(current.score_override);
    } else {
        current.score_override = None;
    }
    for (case, mutants) in other.per_test_kills {
        current
            .per_test_kills
            .entry(case)
            .or_default()
            .extend(mutants);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSource {
    pub format: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageEvidence {
    pub files: BTreeMap<String, FileCoverage>,
}

impl CoverageEvidence {
    pub fn file_for_path(&self, path: &Path) -> Option<&FileCoverage> {
        let normalized = normalize_path(path);
        self.files
            .get(&normalized)
            .or_else(|| self.files.get(normalized.trim_start_matches("./")))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|(candidate, _)| {
                        Path::new(&normalized).ends_with(Path::new(candidate.as_str()))
                    })
                    .map(|(_, coverage)| coverage)
            })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileCoverage {
    pub line_rate: Option<f64>,
    pub branch_rate: Option<f64>,
    pub covered_lines: BTreeSet<usize>,
    pub executable_lines: BTreeSet<usize>,
}

impl FileCoverage {
    pub fn line_coverage(&self) -> Option<f64> {
        self.line_rate.or_else(|| {
            if self.executable_lines.is_empty() {
                None
            } else {
                Some(self.covered_lines.len() as f64 / self.executable_lines.len() as f64)
            }
        })
    }

    pub fn branch_coverage(&self) -> Option<f64> {
        self.branch_rate
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerTestCoverageEvidence {
    pub cases: BTreeMap<String, BTreeSet<CoverageRequirement>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceEvidence {
    pub cases: BTreeMap<String, TraceCaseEvidence>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceCaseEvidence {
    pub executed_lines: BTreeSet<CoverageRequirement>,
    pub checked_lines: BTreeSet<CoverageRequirement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CoverageRequirement {
    pub path: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutationEvidence {
    pub total: usize,
    pub killed: usize,
    pub equivalent_marked: usize,
    pub score_override: Option<f64>,
    pub per_test_kills: BTreeMap<String, BTreeSet<String>>,
}

impl MutationEvidence {
    pub fn score(&self) -> Option<f64> {
        if let Some(score) = self.score_override {
            return Some(score);
        }
        let denominator = self.total.saturating_sub(self.equivalent_marked);
        if denominator == 0 {
            return None;
        }
        Some(self.killed as f64 / denominator as f64)
    }
}

pub fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_path_suffixes_match_by_component() {
        let mut evidence = CoverageEvidence::default();
        evidence
            .files
            .insert("lib/cart.rb".to_owned(), FileCoverage::default());

        assert!(
            evidence
                .file_for_path(Path::new("/work/lib/cart.rb"))
                .is_some()
        );
        assert!(
            evidence
                .file_for_path(Path::new("/work/lib/shopping_cart.rb"))
                .is_none()
        );
    }

    #[test]
    fn merging_evidence_preserves_inputs_of_the_same_kind() {
        let mut left = EvidenceSet {
            coverage: Some(CoverageEvidence {
                files: BTreeMap::from([("lib/a.rb".to_owned(), FileCoverage::default())]),
            }),
            ..EvidenceSet::default()
        };
        left.merge(EvidenceSet {
            coverage: Some(CoverageEvidence {
                files: BTreeMap::from([("lib/b.rb".to_owned(), FileCoverage::default())]),
            }),
            ..EvidenceSet::default()
        });

        assert_eq!(left.coverage.unwrap().files.len(), 2);
    }
}
