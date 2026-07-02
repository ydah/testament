use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use testament_adapter_api::{AdapterError, AdapterResult, EvidenceProvider};
use testament_core::{
    CoverageEvidence, CoverageRequirement, EvidenceInput, EvidenceSet, EvidenceSource,
    FileCoverage, MutationEvidence, PerTestCoverageEvidence,
};

pub fn load_configured_evidence(root: &Path, inputs: &[&EvidenceInput]) -> EvidenceSet {
    let mut evidence = EvidenceSet::default();
    for input in inputs {
        let path = resolve_path(root, Path::new(&input.path));
        match load_evidence(&input.format, &path) {
            Ok(mut loaded) => {
                loaded.sources.push(EvidenceSource {
                    format: input.format.clone(),
                    path,
                });
                evidence.merge(loaded);
            }
            Err(error) => evidence.warnings.push(format!(
                "failed to load {} from {}: {}",
                input.format,
                path.display(),
                error.message
            )),
        }
    }
    evidence
}

pub fn load_evidence(format: &str, path: &Path) -> AdapterResult<EvidenceSet> {
    let content = fs::read_to_string(path)
        .map_err(|error| AdapterError::new(format!("{}: {error}", path.display())))?;
    let mut evidence = match format {
        "simplecov-json" => EvidenceSet {
            coverage: Some(parse_simplecov_json(&content)?),
            ..EvidenceSet::default()
        },
        "lcov" => EvidenceSet {
            coverage: Some(parse_lcov(&content)),
            ..EvidenceSet::default()
        },
        "cobertura" | "cobertura-xml" => EvidenceSet {
            coverage: Some(parse_cobertura_xml(&content)),
            ..EvidenceSet::default()
        },
        "mutant-json" => EvidenceSet {
            mutation: Some(parse_mutation_json(&content)?),
            ..EvidenceSet::default()
        },
        "stryker-json" => EvidenceSet {
            mutation: Some(parse_mutation_json(&content)?),
            ..EvidenceSet::default()
        },
        "per-test-json" | "testament-per-test-json" => EvidenceSet {
            per_test_coverage: Some(parse_per_test_json(&content)?),
            ..EvidenceSet::default()
        },
        other => return Err(AdapterError::new(format!("unsupported evidence format `{other}`"))),
    };
    evidence.sources.push(EvidenceSource {
        format: format.to_owned(),
        path: path.to_path_buf(),
    });
    Ok(evidence)
}

pub struct JsonEvidenceProvider {
    id: &'static str,
}

impl JsonEvidenceProvider {
    pub fn new(id: &'static str) -> Self {
        Self { id }
    }
}

impl EvidenceProvider for JsonEvidenceProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn load(&self, input: &Path) -> AdapterResult<EvidenceSet> {
        load_evidence(self.id, input)
    }
}

fn parse_simplecov_json(content: &str) -> AdapterResult<CoverageEvidence> {
    let value = parse_json(content)?;
    let mut coverage = CoverageEvidence::default();

    if let Some(files) = value.get("files").and_then(Value::as_object) {
        for (path, file) in files {
            let file_coverage = parse_simplecov_file(file);
            coverage.files.insert(normalize_json_path(path), file_coverage);
        }
        return Ok(coverage);
    }

    if let Some(command_sets) = value.as_object() {
        for command in command_sets.values() {
            let Some(files) = command
                .get("coverage")
                .or_else(|| command.get("files"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (path, file) in files {
                coverage
                    .files
                    .insert(normalize_json_path(path), parse_simplecov_file(file));
            }
        }
    }

    Ok(coverage)
}

fn parse_simplecov_file(file: &Value) -> FileCoverage {
    let line_values = file
        .get("lines")
        .or_else(|| file.get("coverage"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut coverage = FileCoverage::default();

    for (index, value) in line_values.iter().enumerate() {
        if value.is_null() {
            continue;
        }
        let line = index + 1;
        coverage.executable_lines.insert(line);
        if value.as_i64().unwrap_or_default() > 0 {
            coverage.covered_lines.insert(line);
        }
    }

    coverage.line_rate = ratio(coverage.covered_lines.len(), coverage.executable_lines.len());
    coverage.branch_rate = file
        .get("branches")
        .and_then(branch_rate_from_simplecov)
        .or_else(|| file.get("branch_coverage").and_then(Value::as_f64));
    coverage
}

fn branch_rate_from_simplecov(value: &Value) -> Option<f64> {
    let mut covered = 0_usize;
    let mut total = 0_usize;
    for branch in flatten_json_numbers(value) {
        total += 1;
        if branch > 0 {
            covered += 1;
        }
    }
    ratio(covered, total)
}

fn parse_lcov(content: &str) -> CoverageEvidence {
    let mut evidence = CoverageEvidence::default();
    let mut current_path = None::<String>;
    let mut current = FileCoverage::default();
    let mut branch_total = 0_usize;
    let mut branch_covered = 0_usize;

    for line in content.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            flush_lcov(
                &mut evidence,
                &mut current_path,
                &mut current,
                &mut branch_total,
                &mut branch_covered,
            );
            current_path = Some(normalize_json_path(path));
        } else if let Some(data) = line.strip_prefix("DA:") {
            let mut parts = data.split(',');
            let line_no = parts.next().and_then(|value| value.parse::<usize>().ok());
            let hits = parts.next().and_then(|value| value.parse::<usize>().ok());
            if let Some(line_no) = line_no {
                current.executable_lines.insert(line_no);
                if hits.unwrap_or_default() > 0 {
                    current.covered_lines.insert(line_no);
                }
            }
        } else if let Some(data) = line.strip_prefix("BRDA:") {
            branch_total += 1;
            if data.rsplit(',').next().is_some_and(|hits| hits != "-" && hits != "0") {
                branch_covered += 1;
            }
        } else if line == "end_of_record" {
            flush_lcov(
                &mut evidence,
                &mut current_path,
                &mut current,
                &mut branch_total,
                &mut branch_covered,
            );
        }
    }

    flush_lcov(
        &mut evidence,
        &mut current_path,
        &mut current,
        &mut branch_total,
        &mut branch_covered,
    );
    evidence
}

fn flush_lcov(
    evidence: &mut CoverageEvidence,
    current_path: &mut Option<String>,
    current: &mut FileCoverage,
    branch_total: &mut usize,
    branch_covered: &mut usize,
) {
    let Some(path) = current_path.take() else {
        return;
    };
    current.line_rate = ratio(current.covered_lines.len(), current.executable_lines.len());
    current.branch_rate = ratio(*branch_covered, *branch_total);
    evidence.files.insert(path, std::mem::take(current));
    *branch_total = 0;
    *branch_covered = 0;
}

fn parse_cobertura_xml(content: &str) -> CoverageEvidence {
    let mut evidence = CoverageEvidence::default();
    let mut current_path = None::<String>;
    let mut current = FileCoverage::default();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with("<class ") {
            if let Some(path) = current_path.take() {
                current.line_rate = ratio(current.covered_lines.len(), current.executable_lines.len());
                evidence.files.insert(path, std::mem::take(&mut current));
            }
            current_path = attr(line, "filename").as_deref().map(normalize_json_path);
            current.line_rate = attr(line, "line-rate").and_then(|value| value.parse().ok());
            current.branch_rate = attr(line, "branch-rate").and_then(|value| value.parse().ok());
        } else if line.starts_with("<line ") {
            let line_no = attr(line, "number").and_then(|value| value.parse::<usize>().ok());
            let hits = attr(line, "hits").and_then(|value| value.parse::<usize>().ok());
            if let Some(line_no) = line_no {
                current.executable_lines.insert(line_no);
                if hits.unwrap_or_default() > 0 {
                    current.covered_lines.insert(line_no);
                }
            }
        }
    }

    if let Some(path) = current_path.take() {
        current.line_rate = current
            .line_rate
            .or_else(|| ratio(current.covered_lines.len(), current.executable_lines.len()));
        evidence.files.insert(path, current);
    }
    evidence
}

fn parse_mutation_json(content: &str) -> AdapterResult<MutationEvidence> {
    let value = parse_json(content)?;
    let mut mutation = MutationEvidence::default();

    mutation.total = usize_value(&value, &["total", "total_mutants", "totalMutants"]).unwrap_or(0);
    mutation.killed = usize_value(&value, &["killed", "killed_mutants", "killedMutants"]).unwrap_or(0);
    mutation.equivalent_marked =
        usize_value(&value, &["equivalent", "equivalent_marked", "equivalentMarked"]).unwrap_or(0);

    if let Some(mutants) = value
        .get("mutants")
        .or_else(|| value.get("mutationResults"))
        .and_then(Value::as_array)
    {
        mutation.total = mutation.total.max(mutants.len());
        mutation.killed = mutation.killed.max(
            mutants
                .iter()
                .filter(|mutant| {
                    string_any(mutant, &["status", "result"]).is_some_and(|status| {
                        matches!(
                            status.as_str(),
                            "killed" | "KILLED" | "timeout" | "Timeout" | "TIMED_OUT"
                        )
                    })
                })
                .count(),
        );
    }

    if let Some(per_test) = value.get("per_test_kills").or_else(|| value.get("perTestKills")) {
        mutation.per_test_kills = parse_string_set_map(per_test);
    }

    if let Some(score) = value.get("mutation_score").and_then(Value::as_f64) {
        if mutation.total == 0 {
            mutation.total = 1000;
            mutation.killed = (score * 1000.0).round() as usize;
        }
    }

    Ok(mutation)
}

fn parse_per_test_json(content: &str) -> AdapterResult<PerTestCoverageEvidence> {
    let value = parse_json(content)?;
    let mut evidence = PerTestCoverageEvidence::default();
    let cases = value
        .get("cases")
        .or_else(|| value.get("tests"))
        .unwrap_or(&value);

    let Some(case_map) = cases.as_object() else {
        return Ok(evidence);
    };

    for (case_id, raw) in case_map {
        let mut requirements = BTreeSet::new();
        if let Some(files) = raw.as_object() {
            for (path, lines) in files {
                for line in lines.as_array().into_iter().flatten() {
                    if let Some(line) = line.as_u64().and_then(|value| usize::try_from(value).ok()) {
                        requirements.insert(CoverageRequirement {
                            path: normalize_json_path(path),
                            line,
                        });
                    }
                }
            }
        }
        evidence.cases.insert(case_id.clone(), requirements);
    }

    Ok(evidence)
}

fn parse_json(content: &str) -> AdapterResult<Value> {
    serde_json::from_str(content).map_err(|error| AdapterError::new(error.to_string()))
}

fn parse_string_set_map(value: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut map = BTreeMap::new();
    let Some(object) = value.as_object() else {
        return map;
    };
    for (case_id, raw_values) in object {
        let values = raw_values
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        map.insert(case_id.clone(), values);
    }
    map
}

fn flatten_json_numbers(value: &Value) -> Vec<i64> {
    match value {
        Value::Number(number) => number.as_i64().into_iter().collect(),
        Value::Array(values) => values.iter().flat_map(flatten_json_numbers).collect(),
        Value::Object(values) => values.values().flat_map(flatten_json_numbers).collect(),
        _ => Vec::new(),
    }
}

fn usize_value(value: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
    })
}

fn string_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn attr(line: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let (_, rest) = line.split_once(&needle)?;
    let (value, _) = rest.split_once('"')?;
    Some(value.to_owned())
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

fn normalize_json_path(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lcov_lines_and_branches() {
        let evidence = parse_lcov(
            r#"
SF:lib/cart.rb
DA:1,1
DA:2,0
BRDA:1,0,0,1
BRDA:2,0,0,0
end_of_record
"#,
        );
        let file = evidence.files.get("lib/cart.rb").unwrap();
        assert_eq!(file.line_coverage(), Some(0.5));
        assert_eq!(file.branch_coverage(), Some(0.5));
    }

    #[test]
    fn parses_mutation_summary_and_per_test_kills() {
        let mutation = parse_mutation_json(
            r#"{"total": 3, "killed": 2, "per_test_kills": {"case-1": ["m1", "m2"]}}"#,
        )
        .unwrap();
        assert_eq!(mutation.score(), Some(2.0 / 3.0));
        assert_eq!(mutation.per_test_kills["case-1"].len(), 2);
    }
}
