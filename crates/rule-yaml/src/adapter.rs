//! YamlRuleAdapter：将 YamlRule 包装为 Rule<CompilationUnit>。

use guard_core::rule::{Rule, RuleId, Severity, Violation};
use java_ast::ast::CompilationUnit;

use crate::matcher::match_pattern;
use crate::rule::YamlRule;

/// 将 YamlRule 适配为 Rule<CompilationUnit>。
pub struct YamlRuleAdapter {
    rule: YamlRule,
    rule_id: RuleId,
}

impl YamlRuleAdapter {
    pub fn new(rule: YamlRule) -> Self {
        let rule_id = rule.rule_id();
        Self { rule, rule_id }
    }
}

impl Rule<CompilationUnit> for YamlRuleAdapter {
    fn id(&self) -> &RuleId {
        &self.rule_id
    }

    fn description(&self) -> &str {
        &self.rule.title
    }

    fn severity(&self) -> Severity {
        self.rule.severity()
    }

    fn enabled(&self) -> bool {
        self.rule.enabled
    }

    fn check_unit(&self, unit: &CompilationUnit) -> Vec<Violation> {
        let file = if unit.source_file.is_empty() {
            "<unknown>"
        } else {
            &unit.source_file
        };
        match_pattern(
            &self.rule.pattern,
            unit,
            file,
            &self.rule.id,
            self.rule.severity(),
            &self.rule.message,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_rule_str;
    use guard_core::rule::{Rule, Severity};
    use java_ast::ast::*;

    fn sysout_unit() -> CompilationUnit {
        CompilationUnit {
            package: Some("com.example".to_string()),
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Demo".to_string(),
                modifiers: vec!["public".to_string()],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                    name: "run".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    return_type: Some("void".to_string()),
                    parameters: vec![],
                    body: Some(BlockStmt {
                        statements: vec![Stmt::ExpressionStmt(ExprStmt {
                            expr: Expr::MethodCallExpr(MethodCallExpr {
                                callee: Some("System.out".to_string()),
                                method_name: "println".to_string(),
                                arguments: vec![],
                                line: 5,
                            }),
                            line: 5,
                        })],
                        line: 4,
                        end_line: 6,
                    }),
                    line: 4,
                    end_line: 6,
                })],
                line: 1,
                end_line: 8,
            })],
            source_file: "Demo.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    #[test]
    fn adapter_exposes_rule_metadata() {
        let yaml = r#"
id: J001
title: "禁止 System.out.println"
severity: minor
pattern:
  type: MethodCall
  match_fields:
    callee: "System.out"
    method: "println"
message: "不要使用 System.out.println"
"#;
        let rule = load_rule_str(yaml).unwrap();
        let adapter = YamlRuleAdapter::new(rule);
        assert_eq!(adapter.id().0, "J001");
        assert_eq!(adapter.description(), "禁止 System.out.println");
        assert_eq!(adapter.severity(), Severity::Minor);
        assert!(adapter.enabled());
    }

    #[test]
    fn adapter_check_unit_produces_violation() {
        let yaml = r#"
id: J001
title: "禁止 System.out.println"
severity: minor
pattern:
  type: MethodCall
  match_fields:
    callee: "System.out"
    method: "println"
message: "不要使用 {callee}.{method}"
"#;
        let rule = load_rule_str(yaml).unwrap();
        let adapter = YamlRuleAdapter::new(rule);
        let vs = adapter.check_unit(&sysout_unit());
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].rule_id.0, "J001");
        assert_eq!(vs[0].line, 5);
        assert_eq!(vs[0].message, "不要使用 System.out.println");
    }

    #[test]
    fn adapter_skips_disabled_rule() {
        let yaml = r#"
id: J001
title: "禁止 System.out.println"
severity: minor
enabled: false
pattern:
  type: MethodCall
  match_fields:
    callee: "System.out"
    method: "println"
message: "x"
"#;
        let rule = load_rule_str(yaml).unwrap();
        let adapter = YamlRuleAdapter::new(rule);
        assert!(!adapter.enabled());
        // 即便有匹配，调用方也应跳过；这里验证 check_unit 仍返回（调用方负责 enabled 判断）
        let vs = adapter.check_unit(&sysout_unit());
        assert_eq!(vs.len(), 1);
    }
}
