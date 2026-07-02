pub mod config;
pub mod discovery;
pub mod evidence;
pub mod gate;
pub mod ir;
pub mod metric;

pub use config::{
    AppConfig, EvidenceConfig, EvidenceInput, GateConfig, GateLevel, RatchetConfig, RuleConfig,
};
pub use discovery::{discover_test_files, matches_any_ignore};
pub use evidence::*;
pub use gate::{
    GateDirection, GateEvaluation, GateViolation, evaluate_gates, evaluate_ratchet,
    parse_baseline_scores,
};
pub use ir::*;
pub use metric::*;
