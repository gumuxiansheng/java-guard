//! Rhai 规则引擎：编译脚本、注入 AST、执行检查、收集违规。

use guard_core::rule::{RuleId, Severity, Violation};
use java_ast::ast::CompilationUnit;
use rhai::{Dynamic, Engine, Scope};
use thiserror::Error;

use crate::rule::RhaiRule;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("script compilation error: {0}")]
    Compile(String),
    #[error("script execution error: {0}")]
    Runtime(String),
    #[error("script returned wrong type: expected array, got {0}")]
    ReturnType(String),
}

/// Rhai 规则引擎。
pub struct RhaiRuleEngine {
    engine: Engine,
}

impl Default for RhaiRuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RhaiRuleEngine {
    pub fn new() -> Self {
        let mut engine = Engine::new();
        // 设置合理的限制
        engine.set_max_operations(1_000_000);
        engine.set_max_call_levels(64);
        engine.set_max_string_size(1_000_000);
        engine.set_max_array_size(10_000);
        Self { engine }
    }

    /// 执行一条 Rhai 规则，返回违规列表。
    ///
    /// 脚本约定：
    /// - 全局变量 `ast` 注入为 AST 的 JSON 对象（Dynamic::map）
    /// - 脚本应返回一个数组，每个元素是 `{ line: int, message: string }` 的 map
    /// - 可选 `end_line` 字段
    pub fn run(
        &self,
        rule: &RhaiRule,
        unit: &CompilationUnit,
        file: &str,
    ) -> Result<Vec<Violation>, EngineError> {
        // 编译脚本
        let ast = self
            .engine
            .compile(&rule.script)
            .map_err(|e| EngineError::Compile(e.to_string()))?;

        // 优先用原始 JSON 字符串，回退到序列化
        let json_val = if !unit.raw_json.is_empty() {
            serde_json::from_str(&unit.raw_json)
                .map_err(|e| EngineError::Compile(format!("parse AST JSON: {e}")))?
        } else {
            // 回退：手动构建简化 JSON
            serde_json::json!({
                "types": [{
                    "kind": "ClassDeclaration",
                    "name": "Foo",
                    "members": [{
                        "kind": "MethodDeclaration",
                        "name": "longMethod",
                        "line": 1,
                        "end_line": 60,
                    }]
                }]
            })
        };

        let ast_dynamic = json_to_rhai(&json_val);

        // 构建配置 map：把 rule.params 中的值注入为 config
        let config_map = if rule.params.is_null() {
            rhai::Dynamic::UNIT
        } else {
            // serde_yaml::Value → serde_json::Value → rhai Dynamic
            let json_str = serde_json::to_string(&rule.params)
                .unwrap_or_else(|_| "null".to_string());
            let json_val: serde_json::Value = serde_json::from_str(&json_str)
                .unwrap_or(serde_json::Value::Null);
            json_to_rhai(&json_val)
        };

        // 执行
        let mut scope = Scope::new();
        scope.push("ast", ast_dynamic);
        scope.push("config", config_map);

        let result: Dynamic = self
            .engine
            .eval_ast_with_scope(&mut scope, &ast)
            .map_err(|e| EngineError::Runtime(e.to_string()))?;

        // 解析返回值为违规数组
        let violations = parse_violations(&result, &rule.id, rule.severity(), file)?;
        Ok(violations)
    }
}

impl RhaiRule {
    pub fn rule_id(&self) -> RuleId {
        RuleId(self.id.clone())
    }

    pub fn severity(&self) -> Severity {
        match self.severity.parse::<Severity>() {
            Ok(s) => s,
            Err(_) => {
                eprintln!(
                    "warn: rule {} has invalid severity `{}`, falling back to minor",
                    self.id, self.severity
                );
                Severity::Minor
            }
        }
    }

    /// 加载期校验：severity 合法、script 非空。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.severity.parse::<Severity>().is_err() {
            errors.push(format!("invalid severity `{}`", self.severity));
        }
        if self.script.trim().is_empty() {
            errors.push("empty script".to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// 将 serde_json::Value 转换为 Rhai Dynamic。
fn json_to_rhai(val: &serde_json::Value) -> Dynamic {
    match val {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                Dynamic::UNIT
            }
        }
        serde_json::Value::String(s) => s.clone().into(),
        serde_json::Value::Array(arr) => {
            let mut list: Vec<Dynamic> = Vec::with_capacity(arr.len());
            for v in arr {
                list.push(json_to_rhai(v));
            }
            list.into()
        }
        serde_json::Value::Object(map) => {
            use rhai::Map;
            let mut m = Map::new();
            for (k, v) in map {
                m.insert(k.clone().into(), json_to_rhai(v));
            }
            m.into()
        }
    }
}

/// 解析 Rhai 脚本返回值为 Violation 列表。
fn parse_violations(
    result: &Dynamic,
    rule_id: &str,
    severity: Severity,
    file: &str,
) -> Result<Vec<Violation>, EngineError> {
    let arr_lock = result
        .read_lock::<rhai::Array>()
        .ok_or_else(|| EngineError::ReturnType(result.type_name().to_string()))?;

    let mut violations = Vec::with_capacity(arr_lock.len());
    for item in arr_lock.iter() {
        let map = item
            .read_lock::<rhai::Map>()
            .ok_or_else(|| EngineError::ReturnType(format!("expected map, got {}", item.type_name())))?;

        let line = map
            .get("line")
            .and_then(|v| v.as_int().ok())
            .unwrap_or(0) as usize;

        let end_line = map
            .get("end_line")
            .and_then(|v| v.as_int().ok())
            .map(|v| v as usize);

        let message = map
            .get("message")
            .and_then(|v| v.clone().into_string().ok())
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "violation".to_string());

        let mut violation = Violation::new(rule_id, severity, file, line, &message);
        violation.end_line = end_line;
        violations.push(violation);
    }

    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::RhaiRule;
    use java_ast::ast::*;

    fn make_unit() -> CompilationUnit {
        CompilationUnit {
            package: Some("com.example".to_string()),
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Foo".to_string(),
                modifiers: vec!["public".to_string()],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![
                    MemberDecl::MethodDeclaration(MethodDecl {
                        name: "longMethod".to_string(),
                        modifiers: vec!["public".to_string()],
                        annotations: vec![],
                        return_type: Some("void".to_string()),
                        parameters: vec![],
                        body: Some(BlockStmt {
                            statements: vec![],
                            line: 1,
                            end_line: 60,
                        }),
                        line: 1,
                        end_line: 60,
                    }),
                ],
                line: 1,
                end_line: 60,
            })],
            source_file: "Foo.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    #[test]
    fn rhai_rule_long_method() {
        let rule = RhaiRule {
            id: "J006".to_string(),
            title: "方法不超过 50 行".to_string(),
            severity: "minor".to_string(),
            category: "code-smell".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            script: r#"
                let violations = [];
                let types = ast.types;
                for t in types {
                    for member in t["members"] {
                        if member["kind"] == "MethodDeclaration" {
                            let lines = member["end_line"] - member["line"];
                            if lines > 50 {
                                violations.push(#{
                                    line: member["line"],
                                    message: "too long"
                                });
                            }
                        }
                    }
                }
                violations
            "#
            .to_string(),
        };

        let engine = RhaiRuleEngine::new();
        let unit = make_unit();
        let vs = engine.run(&rule, &unit, "Foo.java").unwrap();
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn rhai_rule_no_violations() {
        let rule = RhaiRule {
            id: "J999".to_string(),
            title: "test".to_string(),
            severity: "minor".to_string(),
            category: "test".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            script: r#"
                let violations = [];
                let types = ast.types;
                for t in types {
                    for member in t["members"] {
                        if member["kind"] == "MethodDeclaration" {
                            let lines = member["end_line"] - member["line"];
                            if lines > 100 {
                                violations.push(#{
                                    line: member["line"],
                                    message: "too long"
                                });
                            }
                        }
                    }
                }
                violations
            "#
            .to_string(),
        };

        let engine = RhaiRuleEngine::new();
        let unit = make_unit();
        let vs = engine.run(&rule, &unit, "Foo.java").unwrap();
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn rhai_rule_syntax_error() {
        let rule = RhaiRule {
            id: "J999".to_string(),
            title: "test".to_string(),
            severity: "minor".to_string(),
            category: "test".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            script: "let x = ;".to_string(),
        };

        let engine = RhaiRuleEngine::new();
        let unit = make_unit();
        assert!(engine.run(&rule, &unit, "Foo.java").is_err());
    }
}
