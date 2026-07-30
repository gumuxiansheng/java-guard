//! guard-core — 共享核心类型与接口
//!
//! 从 SqlGuard 借鉴设计，独立实现。包含 Severity、Violation、Rule trait、
//! ViolationCollector、ReportFormat 等引擎核心类型，供 java-guard 及未来其他语言扫描器复用。

pub mod rule;
pub mod reporter;

pub use rule::{Severity, Violation, RuleId, Rule, ViolationCollector, SeverityParseError};
pub use reporter::ReportFormat;
