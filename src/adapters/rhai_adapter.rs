//! RhaiRuleAdapter：将 RhaiRule 包装为 Rule<CompilationUnit>。

use guard_core::rule::{Rule, RuleId, Severity, Violation};
use java_ast::ast::CompilationUnit;
use rule_rhai::engine::RhaiRuleEngine;
use rule_rhai::rule::RhaiRule;

/// 将 RhaiRule 适配为 Rule<CompilationUnit>。
///
/// 不存储 Engine（非 Send+Sync），每次 check_unit 时创建。
pub struct RhaiRuleAdapter {
    rule: RhaiRule,
    rule_id: RuleId,
}

impl RhaiRuleAdapter {
    pub fn new(rule: RhaiRule) -> Self {
        let rule_id = rule.rule_id();
        Self { rule, rule_id }
    }
}

impl Rule<CompilationUnit> for RhaiRuleAdapter {
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
        let engine = RhaiRuleEngine::new();
        match engine.run(&self.rule, unit, file) {
            Ok(vs) => vs,
            Err(e) => {
                eprintln!(
                    "warn: rhai rule {} failed on {}: {e}",
                    self.rule.id, file
                );
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guard_core::rule::{Rule, Severity};
    use java_ast::ast::CompilationUnit;
    use rule_rhai::rule::RhaiRule;

    fn unit() -> CompilationUnit {
        CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    fn test_rule(script: &str) -> RhaiRule {
        RhaiRule {
            id: "J006".to_string(),
            title: "Rhai 测试规则".to_string(),
            severity: "minor".to_string(),
            category: "test".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            script: script.to_string(),
        }
    }

    #[test]
    fn adapter_metadata() {
        let adapter = RhaiRuleAdapter::new(test_rule("[]"));
        assert_eq!(adapter.id().0, "J006");
        assert_eq!(adapter.description(), "Rhai 测试规则");
        assert_eq!(adapter.severity(), Severity::Minor);
        assert!(adapter.enabled());
    }

    #[test]
    fn adapter_runs_script_and_reports() {
        let script =
            "let violations = []; violations.push(#{ line: 7, message: \"rhai hit\" }); violations";
        let adapter = RhaiRuleAdapter::new(test_rule(script));
        let vs = adapter.check_unit(&unit());
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].rule_id.0, "J006");
        assert_eq!(vs[0].line, 7);
        assert_eq!(vs[0].message, "rhai hit");
    }

    #[test]
    fn adapter_handles_script_failure_gracefully() {
        // 脚本语法错误时 check_unit 返回空（不 panic），由调用方兜底
        let adapter = RhaiRuleAdapter::new(test_rule("let x = ;"));
        let vs = adapter.check_unit(&unit());
        assert_eq!(vs.len(), 0);
    }
}
