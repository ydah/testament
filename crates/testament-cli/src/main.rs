use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use testament_core::{
    AppConfig, GateLevel, ProjectReport, discover_test_files, evaluate_ratchet,
    parse_baseline_scores,
};
use testament_evidence::load_configured_evidence;
use testament_metrics::{
    analyze_content_with_evidence, analyze_paths_with_evidence, evaluate_project, metric_catalog,
};
use testament_report::{ReportFormat, render, render_json, render_tty};

#[derive(Parser, Debug)]
#[command(name = "testament")]
#[command(about = "Research-informed test quality guardrails")]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[arg(short, long, global = true, default_value = "testament.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<CommandArgs>,
}

#[derive(Subcommand, Debug)]
enum CommandArgs {
    Check(AnalyzeArgs),
    Report(AnalyzeArgs),
    Baseline(AnalyzeArgs),
    Explain(ExplainArgs),
    Diff(DiffArgs),
}

#[derive(clap::Args, Debug)]
struct AnalyzeArgs {
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Tty)]
    format: OutputFormat,
    paths: Vec<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct ExplainArgs {
    target: String,
}

#[derive(clap::Args, Debug)]
struct DiffArgs {
    #[arg(long)]
    base: String,
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Tty)]
    format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Tty,
    Json,
    Markdown,
    Sarif,
    Junit,
}

impl From<OutputFormat> for ReportFormat {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Tty => Self::Tty,
            OutputFormat::Json => Self::Json,
            OutputFormat::Markdown => Self::Markdown,
            OutputFormat::Sarif => Self::Sarif,
            OutputFormat::Junit => Self::Junit,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("testament: {error}");
            2
        }
    };
    process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32, String> {
    let Some(command) = cli.command else {
        Cli::command()
            .print_help()
            .map_err(|error| error.to_string())?;
        println!();
        return Ok(0);
    };

    match command {
        CommandArgs::Check(args) => check(&cli.root, &cli.config, args),
        CommandArgs::Report(args) => report(&cli.root, &cli.config, args),
        CommandArgs::Baseline(args) => baseline(&cli.root, &cli.config, args),
        CommandArgs::Explain(args) => explain(&cli.root, &cli.config, args),
        CommandArgs::Diff(args) => diff(&cli.root, &cli.config, args),
    }
}

fn check(root: &Path, config_path: &Path, args: AnalyzeArgs) -> Result<i32, String> {
    let (config, mut project) = analyze_project(root, config_path, &args.paths)?;
    apply_ratchet(root, &config, &mut project)?;
    println!("{}", render(&project, args.format.into()));
    Ok(if project.passed { 0 } else { 1 })
}

fn report(root: &Path, config_path: &Path, args: AnalyzeArgs) -> Result<i32, String> {
    let (_, project) = analyze_project(root, config_path, &args.paths)?;
    println!("{}", render(&project, args.format.into()));
    Ok(0)
}

fn baseline(root: &Path, config_path: &Path, args: AnalyzeArgs) -> Result<i32, String> {
    let (config, project) = analyze_project(root, config_path, &args.paths)?;
    let baseline = resolve_path(root, Path::new(&config.ratchet.baseline));
    if let Some(parent) = baseline.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&baseline, render_json(&project)).map_err(|error| error.to_string())?;
    println!("wrote {}", baseline.display());
    Ok(0)
}

fn explain(root: &Path, config_path: &Path, args: ExplainArgs) -> Result<i32, String> {
    let config =
        AppConfig::load(&resolve_path(root, config_path)).map_err(|error| error.to_string())?;
    let path = resolve_path(root, Path::new(&args.target));

    if path.exists() || args.target.ends_with(".rb") {
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let evidence = load_configured_evidence(root, &config.evidence.inputs());
        let file = analyze_content_with_evidence(&path, &content, &config, &evidence);
        let project = ProjectReport {
            files: vec![file],
            gates: Vec::new(),
            passed: true,
        };
        println!("{}", render_tty(&project));
        return Ok(0);
    }

    explain_metric(&args.target, &config)
}

fn diff(root: &Path, config_path: &Path, args: DiffArgs) -> Result<i32, String> {
    let config =
        AppConfig::load(&resolve_path(root, config_path)).map_err(|error| error.to_string())?;
    let paths = changed_test_files(root, &args.base, &config)?;
    let evidence = load_configured_evidence(root, &config.evidence.inputs());
    let files = analyze_paths_with_evidence(&paths, &config, &evidence)
        .map_err(|error| error.to_string())?;
    let mut project = evaluate_project(files, &config);
    apply_ratchet(root, &config, &mut project)?;
    println!("{}", render(&project, args.format.into()));
    Ok(if project.passed { 0 } else { 1 })
}

fn analyze_project(
    root: &Path,
    config_path: &Path,
    explicit_paths: &[PathBuf],
) -> Result<(AppConfig, ProjectReport), String> {
    let config =
        AppConfig::load(&resolve_path(root, config_path)).map_err(|error| error.to_string())?;
    let paths = if explicit_paths.is_empty() {
        discover_test_files(root, &config).map_err(|error| error.to_string())?
    } else {
        explicit_paths
            .iter()
            .map(|path| resolve_path(root, path))
            .collect()
    };
    let evidence = load_configured_evidence(root, &config.evidence.inputs());
    let files = analyze_paths_with_evidence(&paths, &config, &evidence)
        .map_err(|error| error.to_string())?;
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

fn explain_metric(metric_id: &str, config: &AppConfig) -> Result<i32, String> {
    let catalog = metric_catalog(config);
    let Some(outcome) = catalog.iter().find(|outcome| outcome.id == metric_id) else {
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

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_common_options() {
        let cli = Cli::parse_from([
            "testament",
            "--config",
            "custom.toml",
            "report",
            "--format",
            "json",
            "spec/a_spec.rb",
        ]);

        assert_eq!(cli.config, PathBuf::from("custom.toml"));
        let CommandArgs::Report(args) = cli.command.unwrap() else {
            panic!("expected report command");
        };
        assert_eq!(args.format, OutputFormat::Json);
        assert_eq!(args.paths, vec![PathBuf::from("spec/a_spec.rb")]);
    }
}
