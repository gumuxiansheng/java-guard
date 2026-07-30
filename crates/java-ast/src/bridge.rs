//! Parser Bridge — Rust 与 JavaParser jar 之间的桥接。
//!
//! MVP 阶段实现 CliParser：每次调用启动 JVM 进程。
//! 后续优化为 DaemonParser（常驻 JVM）。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ast::CompilationUnit;
use crate::error::ParseError;

/// Java 解析器接口。
pub trait JavaParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError>;
}

/// 通过 CLI 调用 java-parser.jar。
pub struct CliParser {
    jar_path: PathBuf,
    java_cmd: String,
}

impl CliParser {
    pub fn new(jar_path: impl AsRef<Path>) -> Self {
        CliParser {
            jar_path: jar_path.as_ref().to_path_buf(),
            java_cmd: std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string()),
        }
    }

    pub fn with_java_cmd(mut self, cmd: impl Into<String>) -> Self {
        self.java_cmd = cmd.into();
        self
    }
}

impl JavaParser for CliParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        // 写源码到临时文件
        let tmp_dir = std::env::temp_dir();
        let tmp_file = tmp_dir.join(format!(
            "javaguard_parse_{}.java",
            std::process::id()
        ));

        std::fs::write(&tmp_file, source)?;

        let output = Command::new(&self.java_cmd)
            .args(["-jar"])
            .arg(&self.jar_path)
            .args(["--input"])
            .arg(&tmp_file)
            .args(["--format", "json"])
            .output()
            .map_err(|e| ParseError::InvokeError(e.to_string()))?;

        // 清理临时文件
        let _ = std::fs::remove_file(&tmp_file);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ParseError::ParserError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut unit: CompilationUnit = serde_json::from_str(&stdout)?;
        unit.source_file = filename.to_string();
        Ok(unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parser_creation() {
        // 不受外部 JAVA_CMD 环境变量影响
        std::env::remove_var("JAVA_CMD");
        let parser = CliParser::new("/nonexistent/java-parser.jar");
        assert_eq!(parser.java_cmd, "java");
    }

    #[test]
    fn cli_parser_custom_java() {
        let parser = CliParser::new("/nonexistent/java-parser.jar")
            .with_java_cmd("/usr/lib/jvm/java-17/bin/java");
        assert_eq!(parser.java_cmd, "/usr/lib/jvm/java-17/bin/java");
    }
}
