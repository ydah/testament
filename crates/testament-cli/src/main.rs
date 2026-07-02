use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use testament_core::{
    discover_test_files, evaluate_ratchet, parse_baseline_scores, AppConfig, GateLevel,
    ProjectReport,
};
use testament_metrics::{analyze_content, analyze_paths, evaluate_project};
use testament_report::{render, render_json, render_tty, ReportFormat};

fn main() {
    let exit_code = match run(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("testament: {error}");
            2
        }
    };
    process::exit(exit_code);
}

fn run(args: Vec<String>) -> Result<i32, String> {
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        print_help();
        return Ok(0);
    }

    let command = args[0].as_str();
    let options = Options::parse(&args[1..])?;

    match command {
        "check" => check(options),
        "report" => report(options),
        "baseline" => baseline(options),
        "explain" => explain(options),
        "diff" => diff(options),
        unknown => Err(format!("unknown command `{unknown}`")),
    }
}

fn check(options: Options) -> Result<i32, String> {
    let (config, mut project) = analyze_project(&options)?;
    apply_ratchet(&options.root, &config, &mut project)?;
    println!("{}", render(&project, options.format));
    Ok(if project.passed { 0 } else { 1 })
}

fn report(options: Options) -> Result<i32, String> {
    let (_, project) = analyze_project(&options)?;
    println!("{}", render(&project, options.format));
    Ok(0)
}

fn baseline(options: Options) -> Result<i32, String> {
    let (config, project) = analyze_project(&options)?;
    let baseline = resolve_path(&options.root, Path::new(&config.ratchet.baseline));
    if let Some(parent) = baseline.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&baseline, render_json(&project)).map_err(|error| error.to_string())?;
    println!("wrote {}", baseline.display());
    Ok(0)
}

fn explain(options: Options) -> Result<i32, String> {
    let Some(target) = options.positionals.first() else {
        return Err("explain requires a file path or metric id".to_owned());
    };
    let config = AppConfig::load(&resolve_path(&options.root, &options.config_path))
        .map_err(|error| error.to_string())?;
    let path = resolve_path(&options.root, Path::new(target));

    if path.exists() || target.ends_with(".rb") {
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let file = testament_metrics::analyze_content(&path, &content, &config);
        let project = ProjectReport {
            files: vec![file],
            gates: Vec::new(),
            passed: true,
        };
        println!("{}", render_tty(&project));
        return Ok(0);
    }

    explain_metric(target)
}

fn diff(options: Options) -> Result<i32, String> {
    let Some(base) = options.base.as_deref() else {
        return Err("diff requires --base <ref>".to_owned());
    };
    let config = AppConfig::load(&resolve_path(&options.root, &options.config_path))
        .map_err(|error| error.to_string())?;
    let paths = changed_test_files(&options.root, base, &config)?;
    let files = analyze_paths(&paths, &config).map_err(|error| error.to_string())?;
    let mut project = evaluate_project(files, &config);
    apply_ratchet(&options.root, &config, &mut project)?;
    println!("{}", render(&project, options.format));
    Ok(if project.passed { 0 } else { 1 })
}

fn analyze_project(options: &Options) -> Result<(AppConfig, ProjectReport), String> {
    let config_path = resolve_path(&options.root, &options.config_path);
    let config = AppConfig::load(&config_path).map_err(|error| error.to_string())?;
    let paths = if options.positionals.is_empty() {
        discover_test_files(&options.root, &config).map_err(|error| error.to_string())?
    } else {
        options
            .positionals
            .iter()
            .map(|path| resolve_path(&options.root, Path::new(path)))
            .collect()
    };
    let files = analyze_paths(&paths, &config).map_err(|error| error.to_string())?;
    Ok((config.clone(), evaluate_project(files, &config)))
}

fn apply_ratchet(
    root: &Path,
    config: &AppConfig,
    project: &mut ProjectReport,
) -> Result<(), String> {
    if !config.ratchet.enabled {
        return Ok(());
    }

    let baseline = resolve_path(root, Path::new(&config.ratchet.baseline));
    if !baseline.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(baseline).map_err(|error| error.to_string())?;
    let scores = parse_baseline_scores(&content);
    let mut ratchet_violations =
        evaluate_ratchet(&scores, config.ratchet.tolerance, &project.files);
    project.gates.append(&mut ratchet_violations);
    project.passed = project
        .gates
        .iter()
        .all(|violation| violation.level != GateLevel::Error);
    Ok(())
}

fn explain_metric(metric_id: &str) -> Result<i32, String> {
    let sample = r#"
    RSpec.describe Example do
      it "works" do
        expect(1).to eq(1)
      end
    end
    "#;
    let file = analyze_content(
        Path::new("spec/example_spec.rb"),
        sample,
        &AppConfig::default(),
    );
    let Some(outcome) = file.outcomes.iter().find(|outcome| outcome.id == metric_id) else {
        return Err(format!("unknown metric `{metric_id}`"));
    };

    println!("{}", outcome.id);
    println!("axis: {}", outcome.axis.as_str());
    println!("definition: {}", outcome.provenance.definition);
    println!("approximation: {}", outcome.provenance.approximation);
    println!("references: {}", outcome.provenance.references.join(", "));
    Ok(0)
}

fn changed_test_files(root: &Path, base: &str, config: &AppConfig) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .arg("diff")
        .arg("--name-only")
        .arg("--diff-filter=ACMRTUXB")
        .arg(base)
        .current_dir(root)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("rb"))
        .filter(|path| {
            let normalized = path.to_string_lossy().replace('\\', "/");
            config
                .test_globs
                .iter()
                .any(|pattern| simple_glob_match(&normalized, pattern))
        })
        .map(|path| resolve_path(root, &path))
        .collect();
    Ok(paths)
}

fn simple_glob_match(path: &str, pattern: &str) -> bool {
    if let Some((prefix, suffix)) = pattern.split_once("/**/*") {
        return path.starts_with(prefix) && path.ends_with(suffix);
    }
    if let Some((prefix, suffix)) = pattern.split_once("/**/") {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        if let Some((file_prefix, file_suffix)) = suffix.split_once('*') {
            return path.starts_with(prefix)
                && file_name.starts_with(file_prefix)
                && file_name.ends_with(file_suffix);
        }
    }
    path == pattern
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[derive(Clone, Debug)]
struct Options {
    config_path: PathBuf,
    root: PathBuf,
    format: ReportFormat,
    positionals: Vec<String>,
    base: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            config_path: PathBuf::from("testament.toml"),
            root: env::current_dir().map_err(|error| error.to_string())?,
            format: ReportFormat::Tty,
            positionals: Vec::new(),
            base: None,
        };

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--config" | "-c" => {
                    index += 1;
                    options.config_path = PathBuf::from(required_arg(args, index, "--config")?);
                }
                "--format" | "-f" => {
                    index += 1;
                    let value = required_arg(args, index, "--format")?;
                    options.format = ReportFormat::parse(value)
                        .ok_or_else(|| format!("unknown report format `{value}`"))?;
                }
                "--root" => {
                    index += 1;
                    options.root = PathBuf::from(required_arg(args, index, "--root")?);
                }
                "--base" => {
                    index += 1;
                    options.base = Some(required_arg(args, index, "--base")?.to_owned());
                }
                "--json" => options.format = ReportFormat::Json,
                "--markdown" => options.format = ReportFormat::Markdown,
                option if option.starts_with('-') => {
                    return Err(format!("unknown option `{option}`"))
                }
                positional => options.positionals.push(positional.to_owned()),
            }
            index += 1;
        }

        Ok(options)
    }
}

fn required_arg<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        r#"testament

USAGE:
  testament check [--config testament.toml] [--format tty|json|md] [paths...]
  testament report [--format tty|json|md] [paths...]
  testament baseline [--config testament.toml]
  testament explain <file|metric>
  testament diff --base <ref> [--format tty|json|md]

COMMANDS:
  check      analyze tests and fail on error-level gate violations
  report     analyze tests and print a report without failing the process
  baseline   write the current JSON report to the configured ratchet baseline
  explain    show findings for a file or provenance for a metric
  diff       analyze changed test files from a git base ref
"#
    );
}

#[allow(dead_code)]
fn _io_error(error: io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_options() {
        let options = Options::parse(&[
            "--config".to_owned(),
            "custom.toml".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
            "spec/a_spec.rb".to_owned(),
        ])
        .unwrap();

        assert_eq!(options.config_path, PathBuf::from("custom.toml"));
        assert_eq!(options.format, ReportFormat::Json);
        assert_eq!(options.positionals, vec!["spec/a_spec.rb"]);
    }
}
