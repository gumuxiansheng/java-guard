//! guard-core — JavaGuard 与 SqlGuard 共享的核心类型。
//!
//! 语言无关的规则引擎核心：Severity、Violation、Rule trait、Reporter、Git Diff、Gate。

pub mod gate;
pub mod git_diff;
pub mod reporter;
pub mod rule;

pub use gate::{GateConfig, GateResult, SeverityCounts};
pub use git_diff::{DiffKind, FileDiff, GitDiffError, LineFilter, LineRange};
pub use reporter::{report, report_to, ConsoleReporter, CsvReporter, JsonReporter, ReportFormat, SarifReporter, ScanStats};
pub use rule::{Rule, RuleId, Severity, Violation, ViolationCollector};
