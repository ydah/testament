pub mod config;
pub mod discovery;
pub mod gate;
pub mod ir;
pub mod metric;

pub use config::{AppConfig, GateConfig, GateLevel, RatchetConfig, RuleConfig};
pub use discovery::{discover_test_files, matches_any_ignore};
pub use gate::{
    evaluate_gates, evaluate_ratchet, parse_baseline_scores, GateDirection, GateEvaluation,
    GateViolation,
};
pub use ir::*;
pub use metric::*;
