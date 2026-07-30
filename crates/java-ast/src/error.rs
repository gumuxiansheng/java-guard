//! 解析错误类型。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("failed to invoke java parser: {0}")]
    InvokeError(String),

    #[error("java parser returned error: {0}")]
    ParserError(String),

    #[error("failed to deserialize AST JSON: {0}")]
    DeserializeError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
