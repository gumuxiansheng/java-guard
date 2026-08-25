//! 内置规则 J016：catch 块中抛出异常前必须先记录日志（SLF4J）。
//!
//! 「抛错不允许静默」：在 catch 中重新抛出异常时，若不先通过日志门面
//! （团队统一使用 SLF4J）记录原始异常，上游调用方将丢失根因上下文，
//! 排障时无法追溯最初的出错点。
//!
//! 判定逻辑：某个 catch 块内存在 `throw` 语句，但整个块内没有任何
//! SLF4J 风格的日志调用（`log/logger.error|warn|info|debug|trace`）
//! 时报违规。

use guard_core::rule::{Rule, RuleId, Severity, Violation};
use java_ast::ast::{BlockStmt, CompilationUnit, Expr, MemberDecl, Stmt, TypeDecl};

/// 视为「日志记录」的方法名（SLF4J Logger API）。
const LOG_METHODS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

pub struct ThrowMustLogRule {
    id: RuleId,
}

impl ThrowMustLogRule {
    pub fn new() -> Self {
        Self {
            id: RuleId("J016".to_string()),
        }
    }
}

impl Default for ThrowMustLogRule {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule<CompilationUnit> for ThrowMustLogRule {
    fn id(&self) -> &RuleId {
        &self.id
    }

    fn description(&self) -> &str {
        "Rethrowing in catch block without logging silently discards root cause"
    }

    fn severity(&self) -> Severity {
        Severity::Major
    }

    fn check_unit(&self, unit: &CompilationUnit) -> Vec<Violation> {
        let mut violations = Vec::new();
        for td in &unit.types {
            check_type_decl(td, &unit.source_file, &mut violations);
        }
        violations
    }
}

fn check_type_decl(td: &TypeDecl, file: &str, out: &mut Vec<Violation>) {
    let members = match td {
        TypeDecl::ClassDeclaration(c) => &c.members,
        TypeDecl::InterfaceDeclaration(i) => &i.members,
        TypeDecl::EnumDeclaration(e) => &e.members,
        TypeDecl::AnnotationDeclaration(a) => &a.members,
    };
    for m in members {
        check_member(m, file, out);
    }
}

fn check_member(m: &MemberDecl, file: &str, out: &mut Vec<Violation>) {
    match m {
        MemberDecl::MethodDeclaration(md) => {
            if let Some(body) = &md.body {
                check_block(body, file, out);
            }
        }
        MemberDecl::ConstructorDeclaration(cd) => {
            if let Some(body) = &cd.body {
                check_block(body, file, out);
            }
        }
        MemberDecl::ClassDeclaration(c) => {
            for m in &c.members { check_member(m, file, out); }
        }
        MemberDecl::InterfaceDeclaration(i) => {
            for m in &i.members { check_member(m, file, out); }
        }
        MemberDecl::EnumDeclaration(e) => {
            for m in &e.members { check_member(m, file, out); }
        }
        MemberDecl::AnnotationDeclaration(a) => {
            for m in &a.members { check_member(m, file, out); }
        }
        _ => {}
    }
}

fn check_block(block: &BlockStmt, file: &str, out: &mut Vec<Violation>) {
    for stmt in &block.statements {
        check_stmt(stmt, file, out);
    }
}

fn check_stmt(stmt: &Stmt, file: &str, out: &mut Vec<Violation>) {
    match stmt {
        Stmt::TryStmt(try_stmt) => {
            check_block(&try_stmt.try_body, file, out);
            for cc in &try_stmt.catch_clauses {
                // catch 块内抛错但没有任何日志记录 → 静默抛错
                if block_contains_throw(&cc.body) && !block_contains_log(&cc.body) {
                    out.push(Violation::new(
                        "J016",
                        Severity::Major,
                        file,
                        cc.line,
                        "rethrow without logging: record the caught exception via SLF4J before throwing",
                    ));
                }
                // 继续深入 catch 块，检查其中嵌套的 try-catch
                check_block(&cc.body, file, out);
            }
            if let Some(fin) = &try_stmt.finally_body {
                check_block(fin, file, out);
            }
        }
        Stmt::BlockStmt(b) => check_block(b, file, out),
        Stmt::IfStmt(if_stmt) => {
            check_stmt(&if_stmt.then_stmt, file, out);
            if let Some(else_stmt) = &if_stmt.else_stmt {
                check_stmt(else_stmt, file, out);
            }
        }
        Stmt::ForStmt(for_stmt) => check_stmt(&for_stmt.body, file, out),
        Stmt::ForEachStmt(fe) => check_stmt(&fe.body, file, out),
        Stmt::WhileStmt(while_stmt) => check_stmt(&while_stmt.body, file, out),
        Stmt::DoStmt(do_stmt) => check_stmt(&do_stmt.body, file, out),
        Stmt::SwitchStmt(sw) => {
            for case in &sw.cases {
                for s in &case.statements {
                    check_stmt(s, file, out);
                }
            }
        }
        Stmt::SynchronizedStmt(sync_stmt) => check_block(&sync_stmt.body, file, out),
        _ => {}
    }
}

/// 判断块内（含嵌套语句/lambda）是否存在 throw 语句。
fn block_contains_throw(block: &BlockStmt) -> bool {
    block.statements.iter().any(stmt_contains_throw)
}

fn stmt_contains_throw(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ThrowStmt(_) => true,
        Stmt::ExpressionStmt(es) => expr_contains_throw(&es.expr),
        Stmt::VariableDeclarationStmt(vds) => vds
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().is_some_and(expr_contains_throw)),
        Stmt::ReturnStmt(rs) => rs.expr.as_ref().is_some_and(expr_contains_throw),
        Stmt::IfStmt(is) => {
            expr_contains_throw(&is.condition)
                || stmt_contains_throw(&is.then_stmt)
                || is.else_stmt.as_ref().is_some_and(|s| stmt_contains_throw(s))
        }
        Stmt::ForStmt(fs) => {
            fs.initialization.as_ref().is_some_and(expr_contains_throw)
                || fs.condition.as_ref().is_some_and(expr_contains_throw)
                || fs.update.iter().any(expr_contains_throw)
                || stmt_contains_throw(&fs.body)
        }
        Stmt::ForEachStmt(fe) => {
            expr_contains_throw(&fe.variable)
                || expr_contains_throw(&fe.iterable)
                || stmt_contains_throw(&fe.body)
        }
        Stmt::WhileStmt(ws) => expr_contains_throw(&ws.condition) || stmt_contains_throw(&ws.body),
        Stmt::DoStmt(ds) => stmt_contains_throw(&ds.body) || expr_contains_throw(&ds.condition),
        Stmt::TryStmt(ts) => {
            block_contains_throw(&ts.try_body)
                || ts.catch_clauses.iter().any(|cc| block_contains_throw(&cc.body))
                || ts.finally_body.as_ref().is_some_and(block_contains_throw)
        }
        Stmt::BlockStmt(b) => block_contains_throw(b),
        Stmt::SwitchStmt(ss) => {
            expr_contains_throw(&ss.selector)
                || ss.cases.iter().any(|c| {
                    c.label.as_ref().is_some_and(expr_contains_throw)
                        || c.statements.iter().any(stmt_contains_throw)
                })
        }
        Stmt::SynchronizedStmt(ss) => {
            expr_contains_throw(&ss.expr) || block_contains_throw(&ss.body)
        }
        _ => false,
    }
}

fn expr_contains_throw(expr: &Expr) -> bool {
    match expr {
        Expr::LambdaExpr(le) => stmt_contains_throw(&le.body),
        Expr::MethodCallExpr(mc) => mc.arguments.iter().any(expr_contains_throw),
        Expr::ObjectCreationExpr(oc) => oc.arguments.iter().any(expr_contains_throw),
        Expr::AssignExpr(ae) => {
            expr_contains_throw(&ae.target) || expr_contains_throw(&ae.value)
        }
        Expr::BinaryExpr(be) => expr_contains_throw(&be.left) || expr_contains_throw(&be.right),
        Expr::UnaryExpr(ue) => expr_contains_throw(&ue.expr),
        Expr::ConditionalExpr(ce) => {
            expr_contains_throw(&ce.condition)
                || expr_contains_throw(&ce.then_expr)
                || expr_contains_throw(&ce.else_expr)
        }
        Expr::EnclosedExpr { inner, .. } => expr_contains_throw(inner),
        Expr::CastExpr(ce) => expr_contains_throw(&ce.expr),
        Expr::FieldAccessExpr(fa) => expr_contains_throw(&fa.target),
        Expr::ArrayAccessExpr(aa) => {
            expr_contains_throw(&aa.array) || expr_contains_throw(&aa.index)
        }
        Expr::ArrayCreationExpr(ac) => ac.initializer.iter().any(expr_contains_throw),
        Expr::VariableDeclarationExpr(vde) => vde
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().is_some_and(expr_contains_throw)),
        Expr::InstanceOfExpr(io) => expr_contains_throw(&io.expr),
        _ => false,
    }
}

/// 判断块内是否存在 SLF4J 风格的日志调用：
/// `<anything containing 'log'>.error|warn|info|debug|trace(...)`，
/// 如 `log.error(...)` / `logger.warn(...)` / `LOGGER.error(...)`。
fn block_contains_log(block: &BlockStmt) -> bool {
    block.statements.iter().any(stmt_contains_log)
}

fn stmt_contains_log(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ExpressionStmt(es) => expr_contains_log(&es.expr),
        Stmt::VariableDeclarationStmt(vds) => vds
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().is_some_and(expr_contains_log)),
        Stmt::IfStmt(is) => {
            expr_contains_log(&is.condition)
                || stmt_contains_log(&is.then_stmt)
                || is.else_stmt.as_ref().is_some_and(|s| stmt_contains_log(s))
        }
        Stmt::ForStmt(fs) => {
            fs.initialization.as_ref().is_some_and(expr_contains_log)
                || fs.condition.as_ref().is_some_and(expr_contains_log)
                || fs.update.iter().any(expr_contains_log)
                || stmt_contains_log(&fs.body)
        }
        Stmt::ForEachStmt(fe) => {
            expr_contains_log(&fe.variable)
                || expr_contains_log(&fe.iterable)
                || stmt_contains_log(&fe.body)
        }
        Stmt::WhileStmt(ws) => expr_contains_log(&ws.condition) || stmt_contains_log(&ws.body),
        Stmt::DoStmt(ds) => stmt_contains_log(&ds.body) || expr_contains_log(&ds.condition),
        Stmt::ReturnStmt(rs) => rs.expr.as_ref().is_some_and(expr_contains_log),
        Stmt::ThrowStmt(ts) => expr_contains_log(&ts.expr),
        Stmt::TryStmt(ts) => {
            block_contains_log(&ts.try_body)
                || ts.catch_clauses.iter().any(|cc| block_contains_log(&cc.body))
                || ts.finally_body.as_ref().is_some_and(block_contains_log)
        }
        Stmt::BlockStmt(b) => block_contains_log(b),
        Stmt::SwitchStmt(ss) => {
            expr_contains_log(&ss.selector)
                || ss.cases.iter().any(|c| {
                    c.label.as_ref().is_some_and(expr_contains_log)
                        || c.statements.iter().any(stmt_contains_log)
                })
        }
        Stmt::SynchronizedStmt(ss) => {
            expr_contains_log(&ss.expr) || block_contains_log(&ss.body)
        }
        _ => false,
    }
}

fn expr_contains_log(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCallExpr(mc) => {
            is_log_call(mc) || mc.arguments.iter().any(expr_contains_log)
        }
        Expr::LambdaExpr(le) => stmt_contains_log(&le.body),
        Expr::ObjectCreationExpr(oc) => oc.arguments.iter().any(expr_contains_log),
        Expr::AssignExpr(ae) => {
            expr_contains_log(&ae.target) || expr_contains_log(&ae.value)
        }
        Expr::BinaryExpr(be) => expr_contains_log(&be.left) || expr_contains_log(&be.right),
        Expr::UnaryExpr(ue) => expr_contains_log(&ue.expr),
        Expr::ConditionalExpr(ce) => {
            expr_contains_log(&ce.condition)
                || expr_contains_log(&ce.then_expr)
                || expr_contains_log(&ce.else_expr)
        }
        Expr::EnclosedExpr { inner, .. } => expr_contains_log(inner),
        Expr::CastExpr(ce) => expr_contains_log(&ce.expr),
        Expr::FieldAccessExpr(fa) => expr_contains_log(&fa.target),
        Expr::ArrayAccessExpr(aa) => {
            expr_contains_log(&aa.array) || expr_contains_log(&aa.index)
        }
        Expr::ArrayCreationExpr(ac) => ac.initializer.iter().any(expr_contains_log),
        Expr::VariableDeclarationExpr(vde) => vde
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().is_some_and(expr_contains_log)),
        Expr::InstanceOfExpr(io) => expr_contains_log(&io.expr),
        _ => false,
    }
}

fn is_log_call(mc: &java_ast::ast::MethodCallExpr) -> bool {
    LOG_METHODS.contains(&mc.method_name.as_str())
        && mc
            .callee
            .as_deref()
            .is_some_and(|c| c.to_lowercase().contains("log"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use java_ast::ast::*;

    fn unit_with_method(statements: Vec<Stmt>) -> CompilationUnit {
        CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Test".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                    name: "foo".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    return_type: Some("void".to_string()),
                    parameters: vec![],
                    body: Some(BlockStmt {
                        statements,
                        line: 1,
                        end_line: 20,
                    }),
                    line: 1,
                    end_line: 20,
                })],
                line: 1,
                end_line: 20,
            })],
            source_file: "Test.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    fn try_with_catch(catch_statements: Vec<Stmt>) -> Stmt {
        Stmt::TryStmt(TryStmt {
            resources: vec![],
            try_body: BlockStmt { statements: vec![], line: 1, end_line: 2 },
            catch_clauses: vec![CatchClause {
                exception_type: Some("Exception".to_string()),
                exception_name: Some("e".to_string()),
                body: BlockStmt {
                    statements: catch_statements,
                    line: 3,
                    end_line: 6,
                },
                line: 3,
            }],
            finally_body: None,
            line: 1,
        })
    }

    #[test]
    fn detects_silent_rethrow() {
        let unit = unit_with_method(vec![try_with_catch(vec![Stmt::ThrowStmt(ThrowStmt {
            expr: Expr::NameExpr(NameExpr { name: "e".to_string(), line: 4 }),
            line: 4,
        })])]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].rule_id.0, "J016");
        assert_eq!(vs[0].line, 3);
    }

    #[test]
    fn accepts_rethrow_with_sl4j_log() {
        let log_then_throw = vec![
            Stmt::ExpressionStmt(ExprStmt {
                expr: Expr::MethodCallExpr(MethodCallExpr {
                    callee: Some("log".to_string()),
                    method_name: "error".to_string(),
                    arguments: vec![],
                    line: 4,
                }),
                line: 4,
            }),
            Stmt::ThrowStmt(ThrowStmt {
                expr: Expr::NameExpr(NameExpr { name: "e".to_string(), line: 5 }),
                line: 5,
            }),
        ];
        let unit = unit_with_method(vec![try_with_catch(log_then_throw)]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn accepts_catch_without_throw() {
        let unit = unit_with_method(vec![try_with_catch(vec![Stmt::EmptyStmt])]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn accepts_lombok_logger_uppercase() {
        // LOGGER.error(...) —— 大写常量形式的 logger 也应被认可
        let log_then_throw = vec![
            Stmt::ExpressionStmt(ExprStmt {
                expr: Expr::MethodCallExpr(MethodCallExpr {
                    callee: Some("LOGGER".to_string()),
                    method_name: "warn".to_string(),
                    arguments: vec![],
                    line: 4,
                }),
                line: 4,
            }),
            Stmt::ThrowStmt(ThrowStmt {
                expr: Expr::NameExpr(NameExpr { name: "e".to_string(), line: 5 }),
                line: 5,
            }),
        ];
        let unit = unit_with_method(vec![try_with_catch(log_then_throw)]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn non_logging_call_does_not_count() {
        // e.printStackTrace() 不算日志记录
        let print_then_throw = vec![
            Stmt::ExpressionStmt(ExprStmt {
                expr: Expr::MethodCallExpr(MethodCallExpr {
                    callee: Some("e".to_string()),
                    method_name: "printStackTrace".to_string(),
                    arguments: vec![],
                    line: 4,
                }),
                line: 4,
            }),
            Stmt::ThrowStmt(ThrowStmt {
                expr: Expr::NameExpr(NameExpr { name: "e".to_string(), line: 5 }),
                line: 5,
            }),
        ];
        let unit = unit_with_method(vec![try_with_catch(print_then_throw)]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1);
    }

    #[test]
    fn throw_outside_catch_is_not_reported() {
        // 方法体直接 throw（如参数校验），不在本规则范围
        let unit = unit_with_method(vec![Stmt::ThrowStmt(ThrowStmt {
            expr: Expr::NameExpr(NameExpr { name: "x".to_string(), line: 2 }),
            line: 2,
        })]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn detects_throw_inside_nested_if_in_catch() {
        let nested = try_with_catch(vec![Stmt::IfStmt(IfStmt {
            condition: Expr::LiteralExpr(LiteralExpr {
                value: "true".to_string(),
                literal_type: None,
                line: 4,
            }),
            then_stmt: Box::new(Stmt::ThrowStmt(ThrowStmt {
                expr: Expr::NameExpr(NameExpr { name: "e".to_string(), line: 5 }),
                line: 5,
            })),
            else_stmt: None,
            line: 4,
        })]);
        let unit = unit_with_method(vec![nested]);
        let vs = ThrowMustLogRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1);
    }
}
