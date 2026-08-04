//! 内置规则注册表。

pub mod j008_empty_catch;
pub mod j009_infinite_loop;

use guard_core::rule::Rule;
use java_ast::ast::CompilationUnit;
use std::sync::Arc;

/// 返回所有内置 Rust 规则。
pub fn builtin_rules() -> Vec<Arc<dyn Rule<CompilationUnit>>> {
    vec![
        Arc::new(j008_empty_catch::EmptyCatchRule::new()),
        Arc::new(j009_infinite_loop::InfiniteLoopRule::new()),
    ]
}
