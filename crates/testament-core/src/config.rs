use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub languages: Vec<String>,
    pub test_globs: Vec<String>,
    pub evidence: EvidenceConfig,
    pub gates: Vec<GateConfig>,
    pub ratchet: RatchetConfig,
    pub rules: RuleConfig,
    pub ignore_paths: Vec<String>,
}

impl AppConfig {
    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        Self::try_parse(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    pub fn parse(input: &str) -> Self {
        Self::try_parse(input).expect("invalid testament TOML")
    }

    pub fn try_parse(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str::<RawConfig>(input).map(Self::from_raw)
    }

    fn from_raw(raw: RawConfig) -> Self {
        let mut config = Self::default();
        if let Some(project) = raw.project {
            if let Some(languages) = project.languages {
                config.languages = languages;
            }
            if let Some(test_globs) = project.test_globs {
                config.test_globs = test_globs;
            }
        }
        if let Some(evidence) = raw.evidence {
            config.evidence = EvidenceConfig::from_raw(evidence);
        }
        if let Some(gates) = raw.gates {
            for (metric_id, raw_gate) in gates {
                let gate = GateConfig {
                    metric_id: metric_id.clone(),
                    min: raw_gate.min,
                    max: raw_gate.max,
                    level: raw_gate
                        .level
                        .as_deref()
                        .map(GateLevel::parse)
                        .unwrap_or(GateLevel::Error),
                    when_evidence_available: raw_gate.when.as_deref() == Some("evidence-available"),
                };
                config
                    .gates
                    .retain(|existing| existing.metric_id != metric_id);
                config.gates.push(gate);
            }
        }
        if let Some(ratchet) = raw.ratchet {
            if let Some(enabled) = ratchet.enabled {
                config.ratchet.enabled = enabled;
            }
            if let Some(baseline) = ratchet.baseline {
                config.ratchet.baseline = baseline;
            }
            if let Some(tolerance) = ratchet.tolerance {
                config.ratchet.tolerance = tolerance;
            }
        }
        if let Some(paths) = raw.ignore.and_then(|ignore| ignore.paths) {
            config.ignore_paths = paths;
        }
        if let Some(rules) = raw.rules {
            config.rules.apply_raw(rules);
        }
        config
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            languages: vec!["ruby".to_owned()],
            test_globs: vec![
                "spec/**/*_spec.rb".to_owned(),
                "test/**/*_test.rb".to_owned(),
                "test/**/test_*.rb".to_owned(),
            ],
            evidence: EvidenceConfig::default(),
            gates: vec![
                GateConfig {
                    metric_id: "adequacy.assertion_density".to_owned(),
                    min: Some(0.50),
                    max: None,
                    level: GateLevel::Error,
                    when_evidence_available: false,
                },
                GateConfig {
                    metric_id: "maintainability.smell_score".to_owned(),
                    min: Some(0.85),
                    max: None,
                    level: GateLevel::Error,
                    when_evidence_available: false,
                },
                GateConfig {
                    metric_id: "redundancy.candidate_ratio".to_owned(),
                    min: None,
                    max: Some(0.15),
                    level: GateLevel::Warn,
                    when_evidence_available: false,
                },
            ],
            ratchet: RatchetConfig::default(),
            rules: RuleConfig::default(),
            ignore_paths: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct EvidenceConfig {
    pub coverage: Option<EvidenceInput>,
    pub mutation: Option<EvidenceInput>,
    pub per_test_coverage: Option<EvidenceInput>,
}

impl EvidenceConfig {
    fn from_raw(raw: RawEvidence) -> Self {
        Self {
            coverage: raw.coverage.map(EvidenceInput::from_raw),
            mutation: raw.mutation.map(EvidenceInput::from_raw),
            per_test_coverage: raw.per_test_coverage.map(EvidenceInput::from_raw),
        }
    }

    pub fn inputs(&self) -> Vec<&EvidenceInput> {
        [
            self.coverage.as_ref(),
            self.mutation.as_ref(),
            self.per_test_coverage.as_ref(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceInput {
    pub format: String,
    pub path: String,
}

impl EvidenceInput {
    fn from_raw(raw: RawEvidenceInput) -> Self {
        Self {
            format: raw.format,
            path: raw.path,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateConfig {
    pub metric_id: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub level: GateLevel,
    pub when_evidence_available: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateLevel {
    Error,
    Warn,
}

impl GateLevel {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("warn") || value.eq_ignore_ascii_case("warning") {
            Self::Warn
        } else {
            Self::Error
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RatchetConfig {
    pub enabled: bool,
    pub baseline: String,
    pub tolerance: f64,
}

impl Default for RatchetConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            baseline: ".testament/baseline.json".to_owned(),
            tolerance: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuleConfig {
    pub assertion_roulette_max: usize,
    pub eager_test_max_sut_calls: usize,
    pub structural_similarity_threshold: f64,
    pub mock_overuse_ratio: f64,
    pub magic_number_allowlist: Vec<String>,
}

impl RuleConfig {
    fn apply_raw(&mut self, rules: BTreeMap<String, toml::Value>) {
        for (rule_id, value) in rules {
            let Some(table) = value.as_table() else {
                continue;
            };
            for (key, value) in table {
                self.apply_value(&rule_id, key, value);
            }
        }
    }

    fn apply_value(&mut self, rule_id: &str, key: &str, value: &toml::Value) {
        match (rule_id, key) {
            ("smell.assertion_roulette", "max_assertions") => {
                self.assertion_roulette_max = value
                    .as_integer()
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(self.assertion_roulette_max);
            }
            ("smell.eager_test", "max_sut_calls") => {
                self.eager_test_max_sut_calls = value
                    .as_integer()
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(self.eager_test_max_sut_calls);
            }
            ("smell.mock_overuse", "max_ratio") => {
                self.mock_overuse_ratio = value.as_float().unwrap_or(self.mock_overuse_ratio);
            }
            ("redundancy.structural_similarity", "threshold") => {
                self.structural_similarity_threshold = value
                    .as_float()
                    .unwrap_or(self.structural_similarity_threshold);
            }
            ("smell.magic_number", "allow") => {
                if let Some(values) = value.as_array() {
                    self.magic_number_allowlist = values
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect();
                }
            }
            _ => {}
        }
    }
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            assertion_roulette_max: 3,
            eager_test_max_sut_calls: 3,
            structural_similarity_threshold: 0.88,
            mock_overuse_ratio: 2.0,
            magic_number_allowlist: vec!["0".to_owned(), "1".to_owned(), "-1".to_owned()],
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    project: Option<RawProject>,
    evidence: Option<RawEvidence>,
    gates: Option<BTreeMap<String, RawGate>>,
    ratchet: Option<RawRatchet>,
    rules: Option<BTreeMap<String, toml::Value>>,
    ignore: Option<RawIgnore>,
}

#[derive(Debug, Deserialize)]
struct RawProject {
    languages: Option<Vec<String>>,
    test_globs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawEvidence {
    coverage: Option<RawEvidenceInput>,
    mutation: Option<RawEvidenceInput>,
    per_test_coverage: Option<RawEvidenceInput>,
}

#[derive(Debug, Deserialize)]
struct RawEvidenceInput {
    format: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct RawGate {
    min: Option<f64>,
    max: Option<f64>,
    level: Option<String>,
    when: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRatchet {
    enabled: Option<bool>,
    baseline: Option<String>,
    tolerance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawIgnore {
    paths: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_gates_evidence_and_rule_overrides() {
        let config = AppConfig::parse(
            r#"
            [project]
            test_globs = ["spec/**/*_spec.rb"]

            [evidence]
            coverage = { format = "simplecov-json", path = "coverage/.resultset.json" }
            mutation = { format = "mutant-json", path = "tmp/mutant/report.json" }

            [gates]
            "maintainability.smell_score" = { min = 0.90 }
            "redundancy.candidate_ratio" = { max = 0.10, level = "warn" }

            [rules."smell.eager_test"]
            max_sut_calls = 4
            "#,
        );

        assert_eq!(config.test_globs, vec!["spec/**/*_spec.rb"]);
        assert_eq!(
            config
                .evidence
                .coverage
                .as_ref()
                .map(|input| input.format.as_str()),
            Some("simplecov-json")
        );
        assert_eq!(config.rules.eager_test_max_sut_calls, 4);
        assert_eq!(config.gates.len(), 3);
        assert_eq!(
            config
                .gates
                .iter()
                .find(|gate| gate.metric_id == "maintainability.smell_score")
                .and_then(|gate| gate.min),
            Some(0.90)
        );
    }
}
