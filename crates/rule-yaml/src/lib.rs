//! rule-yaml — YAML 声明式规则引擎
//!
//! 从 YAML 文件加载规则定义，编译为可执行的 Rule<CompilationUnit>。

pub mod adapter;
pub mod loader;
pub mod matcher;
pub mod rule;

pub use adapter::YamlRuleAdapter;
pub use loader::{load_rule_dir, load_rule_file, load_rule_str};
pub use rule::{Pattern, PatternKind, YamlRule};
