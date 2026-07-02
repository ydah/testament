use std::fs;
use std::io;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct AppConfig {
    pub languages: Vec<String>,
    pub test_globs: Vec<String>,
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
        Ok(Self::parse(&content))
    }

    pub fn parse(input: &str) -> Self {
        let mut config = Self::default();
        let mut section = String::new();
        let mut rule_section = None::<String>;

        for raw_line in input.lines() {
            let line = strip_comment(raw_line).trim().to_owned();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                let name = line.trim_matches(['[', ']']);
                section = name.to_owned();
                rule_section = parse_rule_section(name);
                continue;
            }

            let Some((key, value)) = split_assignment(&line) else {
                continue;
            };

            match section.as_str() {
                "project" => match key {
                    "languages" => config.languages = parse_array(value),
                    "test_globs" => config.test_globs = parse_array(value),
                    _ => {}
                },
                "gates" => {
                    if let Some(gate) = GateConfig::parse(key, value) {
                        config
                            .gates
                            .retain(|existing| existing.metric_id != gate.metric_id);
                        config.gates.push(gate);
                    }
                }
                "ratchet" => match key {
                    "enabled" => config.ratchet.enabled = parse_bool(value),
                    "baseline" => config.ratchet.baseline = unquote(value).to_owned(),
                    "tolerance" => config.ratchet.tolerance = parse_f64(value).unwrap_or(0.0),
                    _ => {}
                },
                "ignore" => {
                    if key == "paths" {
                        config.ignore_paths = parse_array(value);
                    }
                }
                _ => {
                    if let Some(rule_id) = &rule_section {
                        config.rules.apply(rule_id, key, value);
                    }
                }
            }
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

#[derive(Clone, Debug, PartialEq)]
pub struct GateConfig {
    pub metric_id: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub level: GateLevel,
    pub when_evidence_available: bool,
}

impl GateConfig {
    fn parse(key: &str, value: &str) -> Option<Self> {
        let metric_id = unquote(key).to_owned();
        if metric_id.is_empty() {
            return None;
        }

        let body = value.trim().trim_start_matches('{').trim_end_matches('}');
        let mut gate = Self {
            metric_id,
            min: None,
            max: None,
            level: GateLevel::Error,
            when_evidence_available: false,
        };

        for part in body.split(',') {
            let Some((name, raw)) = split_assignment(part.trim()) else {
                continue;
            };
            match name {
                "min" => gate.min = parse_f64(raw),
                "max" => gate.max = parse_f64(raw),
                "level" => gate.level = GateLevel::parse(unquote(raw)),
                "when" => gate.when_evidence_available = unquote(raw) == "evidence-available",
                _ => {}
            }
        }

        Some(gate)
    }
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
    pub fn apply(&mut self, rule_id: &str, key: &str, value: &str) {
        match (rule_id, key) {
            ("smell.assertion_roulette", "max_assertions") => {
                self.assertion_roulette_max =
                    parse_usize(value).unwrap_or(self.assertion_roulette_max);
            }
            ("smell.eager_test", "max_sut_calls") => {
                self.eager_test_max_sut_calls =
                    parse_usize(value).unwrap_or(self.eager_test_max_sut_calls);
            }
            ("smell.mock_overuse", "max_ratio") => {
                self.mock_overuse_ratio = parse_f64(value).unwrap_or(self.mock_overuse_ratio);
            }
            ("redundancy.structural_similarity", "threshold") => {
                self.structural_similarity_threshold =
                    parse_f64(value).unwrap_or(self.structural_similarity_threshold);
            }
            ("smell.magic_number", "allow") => self.magic_number_allowlist = parse_array(value),
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

fn parse_rule_section(section: &str) -> Option<String> {
    if !section.starts_with("rules.") {
        return None;
    }
    Some(unquote(section.trim_start_matches("rules.")).to_owned())
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or(line)
}

fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn parse_array(value: &str) -> Vec<String> {
    let value = value.trim();
    let inner = value.trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(str::trim)
        .map(unquote)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_bool(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("true")
}

fn parse_f64(value: &str) -> Option<f64> {
    unquote(value).parse::<f64>().ok()
}

fn parse_usize(value: &str) -> Option<usize> {
    unquote(value).parse::<usize>().ok()
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_gates_and_rule_overrides() {
        let config = AppConfig::parse(
            r#"
            [project]
            test_globs = ["spec/**/*_spec.rb"]

            [gates]
            "maintainability.smell_score" = { min = 0.90 }
            "redundancy.candidate_ratio" = { max = 0.10, level = "warn" }

            [rules."smell.eager_test"]
            max_sut_calls = 4
            "#,
        );

        assert_eq!(config.test_globs, vec!["spec/**/*_spec.rb"]);
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
