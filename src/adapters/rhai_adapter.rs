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
