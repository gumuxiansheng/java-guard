#![recursion_limit = "1024"]

//! java-ast — Java AST 模型与 Parser Bridge。
//!
//! 提供 Rust 侧的 Java AST 类型定义（与 JavaParser jar 输出的 JSON 一一对应），
//! 以及 CLI/Daemon 解析器桥接。

pub mod ast;
pub mod bridge;
pub mod error;

pub use bridge::{CliParser, JavaParser};
pub use error::ParseError;
