//! YAML 规则定义与 Pattern 模型。

use guard_core::rule::{RuleId, Severity};
use serde::{Deserialize, Serialize};

/// 一条 YAML 声明式规则的完整定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlRule {
    /// 规则 ID（如 "J001"）
    pub id: String,
    /// 规则标题
    pub title: String,
    /// 严重级别：info / minor / major / critical
    pub severity: String,
    /// 分类（code-smell / bug / security / style）
    #[serde(default)]
    pub category: String,
    /// 匹配模式
    pub pattern: Pattern,
    /// 违规消息模板（可用 {matched} 等占位符）
    pub message: String,
    /// 是否默认启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 规则参数（如 max_lines 等，供 Rhai 规则用，YAML 规则不用）
    #[serde(default)]
    pub params: serde_yaml::Value,
}

fn default_true() -> bool {
    true
}

/// 匹配模式：描述要匹配的 AST 节点类型和条件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// pattern 类型
    #[serde(rename = "type")]
    pub kind: PatternKind,
    /// 限定条件（字段名 → 期望值），支持通配符 `*`
    #[serde(default)]
    pub match_fields: std::collections::BTreeMap<String, String>,
}

/// 支持的 pattern 类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum PatternKind {
    /// 方法调用
    MethodCall,
    /// import 语句
    Import,
    /// 注解
    Annotation,
    /// 类声明
    ClassDeclaration,
    /// 方法声明
    MethodDeclaration,
    /// 字段声明
    FieldDeclaration,
}

impl YamlRule {
    pub fn rule_id(&self) -> RuleId {
        RuleId(self.id.clone())
    }

    pub fn severity(&self) -> Severity {
        self.severity
            .parse()
            .unwrap_or(Severity::Minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_method_call_rule() {
        let yaml = r#"
id: J001
title: "禁止使用 System.out.println"
severity: minor
category: code-smell
pattern:
  type: MethodCall
  match_fields:
    callee: "System.out"
    method: "println"
message: "不要使用 System.out.println，请使用日志框架（SLF4J）"
"#;
        let rule: YamlRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.id, "J001");
        assert_eq!(rule.pattern.kind, PatternKind::MethodCall);
        assert_eq!(rule.pattern.match_fields.get("callee").unwrap(), "System.out");
        assert_eq!(rule.pattern.match_fields.get("method").unwrap(), "println");
    }

    #[test]
    fn deserialize_import_rule() {
        let yaml = r#"
id: J003
title: "import 不使用通配符"
severity: minor
pattern:
  type: Import
  match_fields:
    is_wildcard: "true"
message: "禁止使用通配符 import"
"#;
        let rule: YamlRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.pattern.kind, PatternKind::Import);
        assert!(rule.enabled);
    }

    #[test]
    fn deserialize_class_decl_rule() {
        let yaml = r#"
id: J004
title: "类名使用 PascalCase"
severity: minor
pattern:
  type: ClassDeclaration
  match_fields:
    name: "^[a-z]" 
message: "类名应使用 PascalCase"
"#;
        let rule: YamlRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.pattern.kind, PatternKind::ClassDeclaration);
    }
}
