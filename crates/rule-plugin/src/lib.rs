//! rule-plugin — Java 插件加载机制（预留接口）。
//!
//! MVP 阶段只定义 trait 和加载器框架，不实际加载 jar。
//! 后续通过 JSON-RPC 与 JVM 子进程通信实现。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use guard_core::rule::{RuleId, Severity, Violation};

/// 插件规则接口：Java 插件需实现此接口的 Rust 侧映射。
///
/// Java 侧通过 JSON-RPC 协议通信：
/// 1. Rust 启动 JVM 子进程，加载插件 jar
/// 2. 通过 stdin/stdout 发送 JSON 请求
/// 3. Java 侧解析 AST 并返回违规列表
pub trait PluginRule: Send + Sync {
    /// 规则 ID。
    fn id(&self) -> &RuleId;

    /// 严重级别。
    fn severity(&self) -> Severity;

    /// 描述。
    fn description(&self) -> &str;

    /// 是否启用。
    fn enabled(&self) -> bool {
        true
    }

    /// 分析 AST JSON，返回违规列表。
    ///
    /// `ast_json` 是 JavaParser 输出的 JSON 字符串。
    fn analyze(&self, ast_json: &str, file: &str) -> Result<Vec<Violation>, PluginError>;
}

/// 插件加载错误。
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin jar not found: {0}")]
    JarNotFound(PathBuf),
    #[error("failed to start JVM: {0}")]
    JvmStart(String),
    #[error("plugin communication error: {0}")]
    Communication(String),
    #[error("plugin returned error: {0}")]
    PluginRuntime(String),
    #[error("invalid plugin response: {0}")]
    InvalidResponse(String),
}

/// 插件加载器配置。
#[derive(Debug, Clone)]
pub struct PluginConfig {
    /// 插件 jar 路径。
    pub jar_path: PathBuf,
    /// Java 运行时路径。
    pub java_cmd: String,
    /// JVM 参数。
    pub jvm_args: Vec<String>,
}

impl PluginConfig {
    pub fn new(jar_path: impl AsRef<Path>) -> Self {
        PluginConfig {
            jar_path: jar_path.as_ref().to_path_buf(),
            java_cmd: std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string()),
            jvm_args: vec!["-Xmx512m".to_string()],
        }
    }
}

/// 插件加载器（MVP 预留，不实际加载）。
pub struct PluginLoader {
    config: PluginConfig,
}

impl PluginLoader {
    pub fn new(config: PluginConfig) -> Self {
        PluginLoader { config }
    }

    /// 加载 jar 中所有实现 PluginRule 接口的类。
    ///
    /// MVP 阶段返回空列表，不实际加载。
    pub fn load(&self) -> Result<Vec<Box<dyn PluginRule>>, PluginError> {
        if !self.config.jar_path.exists() {
            return Err(PluginError::JarNotFound(self.config.jar_path.clone()));
        }

        // MVP: 不实际加载 Java 插件
        // 后续实现：
        // 1. 启动 JVM 子进程
        // 2. 通过反射加载 jar 中所有 PluginRule 实现类
        // 3. 对每个规则创建 Rust 侧代理
        Ok(vec![])
    }

    /// 检查 JVM 是否可用。
    pub fn check_jvm(&self) -> Result<(), PluginError> {
        let output = Command::new(&self.config.java_cmd)
            .args(["-version"])
            .output()
            .map_err(|e| PluginError::JvmStart(format!("failed to run java: {e}")))?;

        if !output.status.success() {
            return Err(PluginError::JvmStart("java -version failed".to_string()));
        }

        Ok(())
    }
}

/// JSON-RPC 请求（Rust → Java）。
#[derive(Debug, Serialize)]
struct RpcRequest {
    action: String,
    rule_id: String,
    ast_json: String,
    file: String,
}

/// JSON-RPC 响应（Java → Rust）。
#[derive(Debug, Deserialize)]
struct RpcResponse {
    status: String,
    violations: Vec<RpcViolation>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcViolation {
    line: usize,
    end_line: Option<usize>,
    message: String,
}

/// 通过 JSON-RPC 调用 Java 插件。
///
/// 这是一个内测函数，MVP 阶段不暴露。
fn call_plugin(
    config: &PluginConfig,
    rule_id: &str,
    ast_json: &str,
    file: &str,
) -> Result<Vec<Violation>, PluginError> {
    let request = RpcRequest {
        action: "analyze".to_string(),
        rule_id: rule_id.to_string(),
        ast_json: ast_json.to_string(),
        file: file.to_string(),
    };

    let request_json = serde_json::to_string(&request)
        .map_err(|e| PluginError::Communication(format!("serialize request: {e}")))?;

    let output = Command::new(&config.java_cmd)
        .args(["-jar"])
        .arg(&config.jar_path)
        .args(["--rpc"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| PluginError::JvmStart(format!("failed to start JVM: {e}")))?;

    // 写入请求，读取响应
    use std::io::Write;
    let mut child = output;
    if let Some(stdin) = &mut child.stdin {
        writeln!(stdin, "{request_json}")
            .map_err(|e| PluginError::Communication(format!("write stdin: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| PluginError::Communication(format!("wait output: {e}")))?;

    let response: RpcResponse = serde_json::from_slice(&output.stdout)
        .map_err(|e| PluginError::InvalidResponse(format!("parse response: {e}")))?;

    if response.status != "ok" {
        return Err(PluginError::PluginRuntime(
            response.error.unwrap_or_else(|| "unknown plugin error".to_string()),
        ));
    }

    let severity = Severity::Minor; // 从规则元数据获取
    Ok(response
        .violations
        .into_iter()
        .map(|v| {
            let mut violation = Violation::new(rule_id, severity, file, v.line, v.message);
            violation.end_line = v.end_line;
            violation
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_config_creation() {
        let config = PluginConfig::new("/nonexistent/plugin.jar");
        assert_eq!(config.jar_path, PathBuf::from("/nonexistent/plugin.jar"));
        assert_eq!(config.java_cmd, "java");
    }

    #[test]
    fn plugin_loader_missing_jar() {
        let config = PluginConfig::new("/nonexistent/plugin.jar");
        let loader = PluginLoader::new(config);
        let result = loader.load();
        assert!(result.is_err());
        match result {
            Err(PluginError::JarNotFound(_)) => {}
            _ => panic!("expected JarNotFound error"),
        }
    }

    #[test]
    fn plugin_loader_existing_jar_returns_empty() {
        // 创建临时 jar 文件
        let tmp = std::env::temp_dir().join("javaguard_plugin_test.jar");
        std::fs::write(&tmp, "dummy").unwrap();

        let config = PluginConfig::new(&tmp);
        let loader = PluginLoader::new(config);
        let result = loader.load();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);

        let _ = std::fs::remove_file(&tmp);
    }
}
