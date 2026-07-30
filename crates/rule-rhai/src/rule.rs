//! Rhai 脚本规则定义与加载。

use serde::Deserialize;
use thiserror::Error;

/// 一条 Rhai 脚本规则的元数据。
#[derive(Debug, Clone, Deserialize)]
pub struct RhaiRule {
    /// 规则 ID
    pub id: String,
    /// 规则标题
    pub title: String,
    /// 严重级别
    pub severity: String,
    /// 分类
    #[serde(default)]
    pub category: String,
    /// Rhai 脚本内容
    pub script: String,
    /// 是否默认启用
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Error)]
pub enum RhaiRuleError {
    #[error("failed to parse rule YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("script compilation failed: {0}")]
    Compile(String),
}

/// 从 YAML 字符串加载 Rhai 规则。
pub fn load_rhai_rule_str(yaml: &str) -> Result<RhaiRule, RhaiRuleError> {
    let rule: RhaiRule = serde_yaml::from_str(yaml)?;
    Ok(rule)
}

/// 从文件加载 Rhai 规则。
pub fn load_rhai_rule_file(path: &std::path::Path) -> Result<RhaiRule, RhaiRuleError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| RhaiRuleError::Compile(format!("read {}: {e}", path.display())))?;
    load_rhai_rule_str(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rhai_rule() {
        let yaml = r#"
id: J006
title: "方法不超过 50 行"
severity: minor
category: code-smell
script: |
  let unit = ast;
  let violations = [];
  for type in unit.types {
    for member in type.members {
      if member.kind == "MethodDeclaration" {
        let lines = member.end_line - member.line;
        if lines > 50 {
          violations.push({
            line: member.line,
            message: "方法长度 " + lines + " 行，超过 50 行限制"
          });
        }
      }
    }
  }
  violations
"#;
        let rule = load_rhai_rule_str(yaml).unwrap();
        assert_eq!(rule.id, "J006");
        assert!(rule.script.contains("violations"));
    }
}
