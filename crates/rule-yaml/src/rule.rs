//! YAML 规则定义与 Pattern 模型。

use guard_core::rule::{RuleId, Severity, SpanPolicy};
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
    /// 增量扫描时的报告策略：anchor（默认）/ intersect
    ///
    /// - `anchor`：锚点行落在 git diff 变更行范围才报告（默认，多数规则适用）
    /// - `intersect`：违规区间与变更行范围相交即报告（结构类规则，如方法超长）
    #[serde(default)]
    pub span_policy: SpanPolicy,
}

fn default_true() -> bool {
    true
}

/// match_fields 中某个字段的期望值。
///
/// - `Single`：单个匹配值（精确 / glob `*` / 正则）
/// - `Any`：多个取值的「或」列表，任意一个命中即匹配（`any_of` 语义）
///
/// 这样 J001 之类的规则可以写成：
/// ```yaml
/// match_fields:
///   callee: [System.out, System.err]
///   method: [print, println, printf]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MatchValue {
    /// 单个匹配值
    Single(String),
    /// 多个取值的「或」列表
    Any(Vec<String>),
}

impl MatchValue {
    /// 取用于布尔型字段（如 `is_wildcard`）的字符串值；列表取第一个。
    pub fn as_str(&self) -> Option<&str> {
        match self {
            MatchValue::Single(s) => Some(s.as_str()),
            MatchValue::Any(list) => list.first().map(|s| s.as_str()),
        }
    }
}

/// 匹配模式：描述要匹配的 AST 节点类型和条件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// pattern 类型
    #[serde(rename = "type")]
    pub kind: PatternKind,
    /// 限定条件（字段名 → 期望值），支持通配符 `*` 与 any_of 列表
    #[serde(default)]
    pub match_fields: std::collections::BTreeMap<String, MatchValue>,
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

    /// 校验规则定义，返回错误列表（非法的 match_fields 键 + 非法 severity）。
    ///
    /// matcher 对未知键会静默忽略（既不命中也不报错），导致规则「隐形失效」：
    /// 作者以为规则生效，实际从未匹配。severity 解析失败会静默降级为 Minor。
    /// 此处在加载期尽早暴露这两类配置错误。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let allowed = allowed_match_fields(self.pattern.kind.clone());
        for key in self.pattern.match_fields.keys() {
            if !allowed.contains(&key.as_str()) {
                errors.push(format!(
                    "unknown match_fields key `{key}` (allowed for {:?}: {:?})",
                    self.pattern.kind, allowed
                ));
            }
        }
        if self.severity.parse::<Severity>().is_err() {
            errors.push(format!("invalid severity `{}`", self.severity));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// 各 Pattern 类型允许使用的 match_fields 键。
///
/// 与 `matcher.rs` 中的取值逻辑保持一致；未知键会被 matcher 静默忽略，
/// 故通过 [`YamlRule::validate`] 在加载期提前暴露。
fn allowed_match_fields(kind: PatternKind) -> &'static [&'static str] {
    match kind {
        PatternKind::MethodCall => &["callee", "method", "method_name"],
        PatternKind::Import => &["package", "is_wildcard", "is_static"],
        PatternKind::Annotation => &["name", "type"],
        PatternKind::ClassDeclaration => &["name", "modifier", "modifiers"],
        PatternKind::MethodDeclaration => &["name", "return_type", "modifier", "modifiers"],
        PatternKind::FieldDeclaration => &["name", "field_type", "type", "modifier", "modifiers"],
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
        assert_eq!(rule.span_policy, SpanPolicy::Anchor);
        assert_eq!(
            rule.pattern.match_fields.get("callee").unwrap(),
            &MatchValue::Single("System.out".to_string())
        );
        assert_eq!(
            rule.pattern.match_fields.get("method").unwrap(),
            &MatchValue::Single("println".to_string())
        );
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

    #[test]
    fn deserialize_span_policy_intersect() {
        let yaml = r#"
id: J006
title: "方法超长"
severity: minor
span_policy: intersect
pattern:
  type: MethodDeclaration
  match_fields:
    name: ".*"
message: "x"
"#;
        let rule: YamlRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.span_policy, SpanPolicy::Intersect);
    }

    #[test]
    fn deserialize_unknown_span_policy_errors() {
        let yaml = r#"
id: J999
title: "x"
severity: minor
span_policy: nope
pattern:
  type: MethodCall
  match_fields:
    method: "println"
message: "x"
"#;
        assert!(serde_yaml::from_str::<YamlRule>(yaml).is_err());
    }
}
