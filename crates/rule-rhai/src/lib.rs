//! rule-rhai — Rhai 脚本规则引擎
//!
//! 允许用 Rhai 脚本编写自定义规则，通过 AST JSON 视图检查 Java 代码。

pub mod engine;
pub mod rule;

pub use engine::RhaiRuleEngine;
pub use rule::RhaiRule;
