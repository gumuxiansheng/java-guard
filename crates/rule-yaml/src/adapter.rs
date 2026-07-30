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
