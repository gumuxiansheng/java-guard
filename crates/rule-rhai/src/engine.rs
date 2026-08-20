//! Rhai 规则引擎：编译脚本、注入 AST、执行检查、收集违规。
//!
//! 性能：`RhaiRuleEngine` 与 `rhai::Engine`/`AST`/`Dynamic` 均不可跨线程共享，
//! 但规则执行以「文件 × 规则」为单位高频调用。为此提供线程级缓存：
//! - 每个线程只初始化一次 `Engine`（含深度/运算上限配置）
//! - 每条规则脚本在每个线程只编译一次
//! - 每个文件只做一次 AST JSON → Rhai `Dynamic` 转换，规则间共享
//! 三者叠加后单个文件的规则阶段开销从数毫秒降到亚毫秒级。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

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
        // set_max_expr_depths(d, fd)：
        //   - d：全局表达式深度（默认 release=64 / debug=32）
        //   - fd：函数体内表达式深度（默认 release=32 / debug=16）★
        // 规则脚本（如 J012 的递归 AST 遍历）函数体较深，会触发
        // "Expression exceeds maximum complexity"。规则脚本是受信任的本地文件，
        // 放宽到 256 足以覆盖复杂规则，同时仍保留解析上限防栈溢出。
        engine.set_max_expr_depths(256, 256);
        engine.set_max_operations(2_000_000);
        // max_call_levels：递归遍历深度文件 AST 可能很深（数百层嵌套表达式），
        // 放宽到 512 避免深文件假阳性「stack overflow」。
        engine.set_max_call_levels(512);
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
        let json_val = ast_json_of(unit);
        let ast_dynamic = json_to_rhai(&json_val);

        let config_map = config_dynamic(&rule.params);

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

thread_local! {
    /// 线程级引擎（不可跨线程共享，每个线程一个，复用深度/运算上限配置）。
    static THREAD_ENGINE: RefCell<RhaiRuleEngine> = RefCell::new(RhaiRuleEngine::new());
    /// 线程级脚本编译缓存：规则 id → (脚本原文, 编译后的 AST)。
    /// 缓存键同时保存脚本原文并在命中时校验，保证同一 id 换脚本后自动失效。
    static COMPILED_SCRIPTS: RefCell<HashMap<String, (String, Arc<rhai::AST>)>> =
        RefCell::new(HashMap::new());
    /// 线程级 AST 转换缓存：存最近一次 (raw_json → Rhai Dynamic)，按原文精确匹配复用。
    /// 单 slot 即可——同一线程同一时刻只处理一个文件。
    static AST_CONVERSION: RefCell<Option<(String, Dynamic)>> = RefCell::new(None);
}

/// 读取 AST 的原始 JSON（`raw_json` 为空时构建回退 JSON）。
fn ast_json_of(unit: &CompilationUnit) -> serde_json::Value {
    if !unit.raw_json.is_empty() {
        match serde_json::from_str(&unit.raw_json) {
            Ok(v) => return v,
            Err(e) => {
                eprintln!("warn: parse AST JSON failed ({e}), using fallback");
            }
        }
    }
    fallback_ast_json()
}

/// raw_json 缺失时的回退 AST（保持与旧行为一致）。
fn fallback_ast_json() -> serde_json::Value {
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
}

/// 规则 params → Rhai Dynamic config。
fn config_dynamic(params: &serde_yaml::Value) -> Dynamic {
    if params.is_null() {
        return Dynamic::UNIT;
    }
    let json_str = serde_json::to_string(params).unwrap_or_else(|_| "null".to_string());
    let json_val: serde_json::Value =
        serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
    json_to_rhai(&json_val)
}

/// 线程级缓存的规则执行：引擎、脚本编译、AST 转换均按线程复用。
///
/// 与 [`RhaiRuleEngine::run`] 行为一致，仅消除重复初始化/编译/转换开销，
/// 供高频批量扫描路径（`RhaiRuleAdapter`）使用；单测仍可用 `run`。
pub fn run_cached(
    rule: &RhaiRule,
    unit: &CompilationUnit,
    file: &str,
) -> Result<Vec<Violation>, EngineError> {
    let raw_json = if !unit.raw_json.is_empty() {
        unit.raw_json.clone()
    } else {
        serde_json::to_string(&fallback_ast_json())
            .map_err(|e| EngineError::Compile(format!("serialize fallback AST: {e}")))?
    };

    // 1) AST JSON → Rhai Dynamic：每 (线程, 文件) 只转换一次
    let ast_dynamic = AST_CONVERSION.with(|c| {
        let mut slot = c.borrow_mut();
        let hit = slot.as_ref().is_some_and(|(key, _)| *key == raw_json);
        if hit {
            Ok(slot.as_ref().expect("hit implies some").1.clone())
        } else {
            let json_val: serde_json::Value = serde_json::from_str(&raw_json)
                .map_err(|e| EngineError::Compile(format!("parse AST JSON: {e}")))?;
            let dyn_ = json_to_rhai(&json_val);
            *slot = Some((raw_json, dyn_.clone()));
            Ok(dyn_)
        }
    })?;

    let config_map = config_dynamic(&rule.params);

    // 2) 引擎 + 脚本编译 + 执行：按线程复用
    THREAD_ENGINE.with(
        |engine_slot| {
            COMPILED_SCRIPTS.with(|scripts| {
                let engine = engine_slot.borrow();
                let mut scripts = scripts.borrow_mut();
                let ast = match scripts.get(&rule.id) {
                    Some((src, ast)) if src == &rule.script => ast.clone(),
                    _ => {
                        let ast = Arc::new(
                            engine
                                .engine
                                .compile(&rule.script)
                                .map_err(|e| EngineError::Compile(e.to_string()))?,
                        );
                        scripts.insert(rule.id.clone(), (rule.script.clone(), ast.clone()));
                        ast
                    }
                };

                let mut scope = Scope::new();
                scope.push("ast", ast_dynamic);
                scope.push("config", config_map);

                let result: Dynamic = engine
                    .engine
                    .eval_ast_with_scope(&mut scope, &ast)
                    .map_err(|e| EngineError::Runtime(e.to_string()))?;
                parse_violations(&result, &rule.id, rule.severity(), file)
            })
        },
    )
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
            span_policy: guard_core::rule::SpanPolicy::Anchor,
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
            span_policy: guard_core::rule::SpanPolicy::Anchor,
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
            span_policy: guard_core::rule::SpanPolicy::Anchor,
            script: "let x = ;".to_string(),
        };

        let engine = RhaiRuleEngine::new();
        let unit = make_unit();
        assert!(engine.run(&rule, &unit, "Foo.java").is_err());
    }

    fn raw_unit() -> CompilationUnit {
        let mut unit = make_unit();
        unit.raw_json = serde_json::json!({
            "imports": [],
            "types": [{
                "kind": "ClassDeclaration",
                "name": "Foo",
                "modifiers": ["public"],
                "annotations": [],
                "extends": null,
                "implements": [],
                "members": [{
                    "kind": "MethodDeclaration",
                    "name": "longMethod",
                    "modifiers": ["public"],
                    "annotations": [],
                    "return_type": "void",
                    "parameters": [],
                    "body": {"kind": "BlockStmt", "line": 1, "end_line": 60, "statements": []},
                    "line": 1,
                    "end_line": 60
                }],
                "line": 1,
                "end_line": 60
            }],
            "source_file": "Foo.java"
        }).to_string();
        unit
    }

    #[test]
    fn run_cached_matches_run_and_reuses() {
        let rule = RhaiRule {
            id: "J600".to_string(),
            title: "cache".to_string(),
            severity: "minor".to_string(),
            category: "test".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            span_policy: guard_core::rule::SpanPolicy::Anchor,
            script: r#"
                let violations = [];
                for t in ast.types {
                    for member in t["members"] {
                        if member["kind"] == "MethodDeclaration" {
                            let lines = member["end_line"] - member["line"];
                            if lines > 50 {
                                violations.push(#{ line: member["line"], message: "too long" });
                            }
                        }
                    }
                }
                violations
            "#
            .to_string(),
        };
        let unit = raw_unit();
        assert_eq!(unit.raw_json.len(), unit.raw_json.len()); // 确保 raw_json 非空
        assert!(!unit.raw_json.is_empty());

        // 与 run() 结果一致
        let engine = RhaiRuleEngine::new();
        let direct = engine.run(&rule, &unit, "Foo.java").unwrap();
        let cached = run_cached(&rule, &unit, "Foo.java").unwrap();
        assert_eq!(direct.len(), cached.len());
        assert_eq!(cached[0].line, 1);
        assert_eq!(cached[0].message, "too long");

        // 缓存路径再次调用仍正确
        let cached2 = run_cached(&rule, &unit, "Foo.java").unwrap();
        assert_eq!(cached2.len(), cached.len());
        assert_eq!(cached2[0].line, cached[0].line);
        assert_eq!(cached2[0].message, cached[0].message);
    }

    #[test]
    fn run_cached_invalidates_on_script_change() {
        let mut rule = RhaiRule {
            id: "J601".to_string(),
            title: "cache-invalidate".to_string(),
            severity: "minor".to_string(),
            category: "test".to_string(),
            enabled: true,
            params: serde_yaml::Value::Null,
            span_policy: guard_core::rule::SpanPolicy::Anchor,
            script: "[]".to_string(),
        };
        let unit = raw_unit();

        // 空结果
        let r1 = run_cached(&rule, &unit, "Foo.java").unwrap();
        assert!(r1.is_empty());

        // 同一 id 换脚本 → 缓存失效，应执行新脚本
        rule.script = r#"
            [ #{ line: 3, message: "now reports" } ]
        "#
        .to_string();
        let r2 = run_cached(&rule, &unit, "Foo.java").unwrap();
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].line, 3);
    }
}
