pub mod config;
pub mod discovery;
pub mod evidence;
pub mod gate;
pub mod ir;
pub mod metric;

pub use config::{
    AppConfig, EvidenceConfig, EvidenceInput, GateConfig, GateLevel, GateTarget, RatchetConfig,
    RuleConfig,
};
pub use discovery::{discover_test_files, matches_any_ignore, matches_test_pattern};
pub use evidence::*;
pub use gate::{
    BaselineFile, GateDirection, GateEvaluation, GateViolation, RatchetEvaluation, evaluate_gates,
    evaluate_ratchet, evaluate_ratchet_with_metrics, parse_baseline_files, parse_baseline_scores,
};
pub use ir::*;
pub use metric::*;
