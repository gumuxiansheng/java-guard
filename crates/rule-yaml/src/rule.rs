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
    /// 注解参数约束（仅 `type: Annotation` 有意义）：要求注解带有指定参数且取值匹配。
    ///
    /// 例如禁止「扩大组件扫描范围」：
    /// ```yaml
    /// pattern:
    ///   type: Annotation
    ///   match_fields:
    ///     name: [ComponentScan, SpringBootApplication]
    ///   match_members:
    ///     basePackages: "*"        # 带 basePackages 参数即命中
    ///     scanBasePackages: "*"
    /// ```
    /// 语义：每个键都必须在注解参数中出现且取值匹配（AND）；取值匹配规则同 `match_fields`。
    /// 单成员注解（`@Foo("x")`）统一记在键 `value` 下。
    #[serde(default)]
    pub match_members: std::collections::BTreeMap<String, MatchValue>,
    /// 祖先约束：命中节点**必须**位于下列 kind 中**任一**祖先之内（any_of）。
    ///
    /// 语义与 `match_fields` 的列表一致——列出的是「或」的关系，便于写
    /// `within: [ForStmt, ForEachStmt, WhileStmt, DoStmt]` 表达「在任意循环内」。
    /// 需要「同时嵌套在 A 与 B 内」这种 AND 组合时，请改用 Rhai 规则。
    #[serde(default)]
    pub within: Vec<AncestorPredicate>,
    /// 祖先排除：命中节点**不得**位于下列任一 kind 的祖先之内（none_of）。
    #[serde(default)]
    pub not_within: Vec<AncestorPredicate>,
    /// 宿主类型约束：**最近的**外层类/接口/枚举/注解声明必须满足该条件。
    #[serde(default)]
    pub in_type: Option<TypePredicate>,
    /// 宿主方法约束：**最近的**外层方法/构造器必须满足该条件。
    #[serde(default)]
    pub in_method: Option<MethodPredicate>,
}

impl Pattern {
    /// 构造一个只有 `type` 的空模式（供单测与编程式构造使用）。
    pub fn new(kind: PatternKind) -> Self {
        Pattern {
            kind,
            match_fields: std::collections::BTreeMap::new(),
            match_members: std::collections::BTreeMap::new(),
            within: Vec::new(),
            not_within: Vec::new(),
            in_type: None,
            in_method: None,
        }
    }

    /// 设置 `match_fields`（编程式构造用）。
    pub fn with_match_fields(
        mut self,
        fields: impl IntoIterator<Item = (String, MatchValue)>,
    ) -> Self {
        self.match_fields = fields.into_iter().collect();
        self
    }
}

/// 祖先节点约束：按节点 `kind` 判定（与 AST JSON 中的 `kind` 字段取值一致）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AncestorPredicate {
    /// 祖先节点的 kind，如 `ForEachStmt` / `TryStmt` / `MethodDeclaration`
    pub kind: String,
}

/// 宿主类型（类/接口/枚举/注解声明）约束。
///
/// 各项之间是 **AND** 关系；`annotations` / `modifiers` 列表内部是 **any_of**。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TypePredicate {
    /// 类型名匹配（精确 / glob / 正则，语义同 `match_fields`）
    pub name: Option<MatchValue>,
    /// 必须带有的注解（任一命中即可）。无通配符时按「简单名或全限定名尾段」匹配，
    /// 因此 `Controller` 既能匹配 `@Controller` 也能匹配 `@org.x.Controller`。
    pub annotations: Vec<String>,
    /// 必须带有的修饰符（任一命中即可）
    pub modifiers: Vec<String>,
    /// 是否必须是接口（`false` 表示必须是类/枚举/注解声明）
    pub is_interface: Option<bool>,
}

/// 宿主方法（方法/构造器）约束。
///
/// 各项之间是 **AND** 关系；列表项内部是 **any_of**。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MethodPredicate {
    /// 方法名匹配（精确 / glob / 正则）
    pub name: Option<MatchValue>,
    /// 返回值类型匹配（构造器无返回值，恒不命中）
    pub return_type: Option<MatchValue>,
    /// 必须带有的注解（任一命中即可）
    pub annotations: Vec<String>,
    /// 必须带有的修饰符（任一命中即可）
    pub modifiers: Vec<String>,
    /// 参数类型匹配（任一命中即可，按类型原文匹配，含泛型）
    pub parameter_types: Vec<String>,
    /// 参数名匹配（任一命中即可）
    pub parameter_names: Vec<String>,
}

/// 匹配器会在 `within` / `not_within` 中作为祖先暴露的节点 kind 白名单。
///
/// 取值与 `AstSerializer` 输出的 JSON `kind` 字段完全一致；
/// 加载期据此校验，避免作者写出 `ForEachStatement` 这类拼写错误后规则静默失效。
pub const KNOWN_ANCESTOR_KINDS: &[&str] = &[
    // 类型与成员
    "ClassDeclaration",
    "InterfaceDeclaration",
    "EnumDeclaration",
    "AnnotationDeclaration",
    "EnumConstant",
    "MethodDeclaration",
    "ConstructorDeclaration",
    "FieldDeclaration",
    "InitializerDeclaration",
    "Parameter",
    // 语句
    "ExpressionStmt",
    "VariableDeclarationStmt",
    "IfStmt",
    "ForStmt",
    "ForEachStmt",
    "WhileStmt",
    "DoStmt",
    "TryStmt",
    "CatchClause",
    "ReturnStmt",
    "ThrowStmt",
    "BreakStmt",
    "ContinueStmt",
    "BlockStmt",
    "SwitchStmt",
    "SynchronizedStmt",
    "EmptyStmt",
    "UnknownStmt",
    // 表达式
    "MethodCallExpr",
    "FieldAccessExpr",
    "NameExpr",
    "LiteralExpr",
    "BinaryExpr",
    "UnaryExpr",
    "AssignExpr",
    "CastExpr",
    "ConditionalExpr",
    "ArrayAccessExpr",
    "ArrayCreationExpr",
    "ObjectCreationExpr",
    "ThisExpr",
    "SuperExpr",
    "InstanceOfExpr",
    "LambdaExpr",
    "MethodReferenceExpr",
    "VariableDeclarationExpr",
    "EnclosedExpr",
    "UnknownExpr",
];

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

impl PatternKind {
    /// 该 pattern 在遍历中实际匹配的节点 `kind`。
    pub fn node_kind(&self) -> &'static str {
        match self {
            PatternKind::MethodCall => "MethodCallExpr",
            PatternKind::Import => "Import",
            PatternKind::Annotation => "Annotation",
            PatternKind::ClassDeclaration => "ClassDeclaration",
            PatternKind::MethodDeclaration => "MethodDeclaration",
            PatternKind::FieldDeclaration => "FieldDeclaration",
        }
    }

    /// 是否需要在整棵类型树上遍历。
    ///
    /// `Import` 只出现在编译单元顶部，遍历 types 纯属浪费——用它短路可保住
    /// 「import 类规则」的原有性能特征（J003/J010/J014/J017 属此类）。
    pub fn needs_type_walk(&self) -> bool {
        !matches!(self, PatternKind::Import)
    }
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

    /// 校验规则定义，返回错误列表。
    ///
    /// 覆盖四类作者易犯且**运行时静默失效**的错误：
    /// 1. `match_fields` 使用了该 pattern 不支持的字段名；
    /// 2. `severity` 非法（运行时会被静默降级为 Minor）；
    /// 3. `within` / `not_within` 里的 `kind` 拼写错误或为空；
    /// 4. 在 `Import` 这类无上下文节点上写了上下文谓词（永远不可能命中）。
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

        if !self.pattern.match_members.is_empty() && self.pattern.kind != PatternKind::Annotation {
            errors.push(format!(
                "`match_members` is only meaningful for `Annotation` patterns, but this rule \
                 uses `{:?}`",
                self.pattern.kind
            ));
        }

        for (label, list) in [
            ("within", &self.pattern.within),
            ("not_within", &self.pattern.not_within),
        ] {
            for p in list {
                if p.kind.trim().is_empty() {
                    errors.push(format!("`{label}` contains an empty `kind`"));
                } else if !KNOWN_ANCESTOR_KINDS.contains(&p.kind.as_str()) {
                    errors.push(format!(
                        "unknown `{label}` kind `{}` (must be one of the AST JSON `kind` values)",
                        p.kind
                    ));
                }
            }
        }

        let has_context = !self.pattern.within.is_empty()
            || !self.pattern.not_within.is_empty()
            || self.pattern.in_type.is_some()
            || self.pattern.in_method.is_some();
        if has_context && !self.pattern.kind.needs_type_walk() {
            errors.push(format!(
                "`{:?}` pattern has no surrounding context, so `within`/`not_within`/\
                 `in_type`/`in_method` can never match",
                self.pattern.kind
            ));
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

    // ── 上下文谓词（within / not_within / in_type / in_method）──

    fn rule_with_pattern(pattern_body: &str) -> String {
        format!(
            "id: J900\ntitle: \"t\"\nseverity: minor\npattern:\n{pattern_body}\nmessage: \"m\"\n"
        )
    }

    #[test]
    fn deserialize_context_predicates() {
        let yaml = rule_with_pattern(
            r#"  type: MethodCall
  match_fields:
    method: [remove, add]
  within:
    - kind: ForEachStmt
    - kind: WhileStmt
  not_within:
    - kind: SynchronizedStmt
  in_type:
    name: ".*Controller$"
    annotations: [RestController]
    is_interface: false
  in_method:
    name: "^get"
    return_type: void
    modifiers: [public]
    parameter_types: ["java.util.List"]
    parameter_names: [ids]"#,
        );
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        let p = &rule.pattern;
        assert_eq!(p.within.len(), 2);
        assert_eq!(p.within[0].kind, "ForEachStmt");
        assert_eq!(p.within[1].kind, "WhileStmt");
        assert_eq!(p.not_within.len(), 1);
        assert_eq!(p.not_within[0].kind, "SynchronizedStmt");
        let it = p.in_type.as_ref().unwrap();
        assert_eq!(
            it.name,
            Some(MatchValue::Single(".*Controller$".to_string()))
        );
        assert_eq!(it.annotations, vec!["RestController".to_string()]);
        assert_eq!(it.is_interface, Some(false));
        let im = p.in_method.as_ref().unwrap();
        assert_eq!(im.name, Some(MatchValue::Single("^get".to_string())));
        assert_eq!(im.return_type, Some(MatchValue::Single("void".to_string())));
        assert_eq!(im.modifiers, vec!["public".to_string()]);
        assert_eq!(im.parameter_types, vec!["java.util.List".to_string()]);
        assert_eq!(im.parameter_names, vec!["ids".to_string()]);
        rule.validate().unwrap();
    }

    /// 既有规则（不含任何上下文键）必须原样解析，默认值为「无约束」。
    #[test]
    fn deserialize_context_fields_default_to_unconstrained() {
        let yaml = rule_with_pattern("  type: Import\n  match_fields:\n    is_wildcard: \"true\"");
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        assert!(rule.pattern.within.is_empty());
        assert!(rule.pattern.not_within.is_empty());
        assert!(rule.pattern.in_type.is_none());
        assert!(rule.pattern.in_method.is_none());
        rule.validate().unwrap();
    }

    #[test]
    fn validate_accepts_known_ancestor_kinds() {
        let yaml = rule_with_pattern(
            "  type: MethodCall\n  within:\n    - kind: ForEachStmt\n    - kind: TryStmt\n    - kind: LambdaExpr",
        );
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn validate_rejects_unknown_ancestor_kind() {
        // 拼写错误若不放行到运行期，规则会永远不命中且毫无提示
        let yaml = rule_with_pattern("  type: MethodCall\n  within:\n    - kind: ForEachStatement");
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        let errs = rule.validate().unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("ForEachStatement"), "got: {errs:?}");
    }

    #[test]
    fn validate_rejects_empty_ancestor_kind() {
        let yaml = rule_with_pattern("  type: MethodCall\n  not_within:\n    - kind: \"\"");
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        let errs = rule.validate().unwrap_err();
        assert!(errs[0].contains("empty"), "got: {errs:?}");
    }

    #[test]
    fn validate_rejects_context_on_import_pattern() {
        // import 位于任何类型之外，上下文谓词永远不会命中——属于确定性写错
        let yaml = rule_with_pattern(
            "  type: Import\n  in_type:\n    annotations: [RestController]",
        );
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        let errs = rule.validate().unwrap_err();
        assert!(errs[0].contains("no surrounding context"), "got: {errs:?}");
    }

    #[test]
    fn validate_still_rejects_unknown_match_fields() {
        let yaml = rule_with_pattern("  type: MethodCall\n  match_fields:\n    nope: x");
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        assert!(rule.validate().is_err());
    }

    // ── 注解参数匹配（match_members）──

    #[test]
    fn deserialize_match_members() {
        let yaml = rule_with_pattern(
            "  type: Annotation\n  match_fields:\n    name: ComponentScan\n  match_members:\n    basePackages: \"*\"",
        );
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            rule.pattern.match_members.get("basePackages"),
            Some(&MatchValue::Single("*".to_string()))
        );
        rule.validate().unwrap();
    }

    #[test]
    fn validate_rejects_match_members_on_non_annotation_pattern() {
        let yaml = rule_with_pattern(
            "  type: MethodCall\n  match_members:\n    basePackages: \"*\"",
        );
        let rule: YamlRule = serde_yaml::from_str(&yaml).unwrap();
        let errs = rule.validate().unwrap_err();
        assert!(
            errs[0].contains("match_members"),
            "got: {errs:?}"
        );
    }

    #[test]
    fn pattern_new_is_unconstrained() {
        let p = Pattern::new(PatternKind::MethodCall);
        assert!(p.match_fields.is_empty());
        assert!(p.within.is_empty());
        assert!(p.not_within.is_empty());
        assert!(p.in_type.is_none());
        assert!(p.in_method.is_none());
    }
}
