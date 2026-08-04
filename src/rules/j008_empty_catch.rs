//! 内置规则 J008：检测空 catch 块。
//! 空 catch 块会吞掉异常，是常见的代码缺陷。
use guard_core::rule::{Rule, RuleId, Severity, Violation};
use java_ast::ast::{CompilationUnit, MemberDecl, Stmt, TypeDecl};

pub struct EmptyCatchRule {
    id: RuleId,
}

impl EmptyCatchRule {
    pub fn new() -> Self {
        Self {
            id: RuleId("J008".to_string()),
        }
    }
}

impl Default for EmptyCatchRule {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule<CompilationUnit> for EmptyCatchRule {
    fn id(&self) -> &RuleId {
        &self.id
    }

    fn description(&self) -> &str {
        "Empty catch block silently swallows exceptions"
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

fn check_block(block: &java_ast::ast::BlockStmt, file: &str, out: &mut Vec<Violation>) {
    for stmt in &block.statements {
        check_stmt(stmt, file, out);
    }
}

fn check_stmt(stmt: &Stmt, file: &str, out: &mut Vec<Violation>) {
    match stmt {
        Stmt::TryStmt(try_stmt) => {
            check_block(&try_stmt.try_body, file, out);
            for cc in &try_stmt.catch_clauses {
                if cc.body.statements.is_empty() {
                    out.push(Violation::new(
                        "J008",
                        Severity::Major,
                        file,
                        cc.line,
                        "empty catch block: exception is silently swallowed",
                    ));
                } else {
                    check_block(&cc.body, file, out);
                }
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
        Stmt::WhileStmt(while_stmt) => check_stmt(&while_stmt.body, file, out),
        Stmt::DoStmt(do_stmt) => check_stmt(&do_stmt.body, file, out),
        Stmt::SynchronizedStmt(sync_stmt) => check_block(&sync_stmt.body, file, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use java_ast::ast::*;

    fn make_violation_file() -> String {
        "Test.java".to_string()
    }

    #[test]
    fn detects_empty_catch() {
        let unit = CompilationUnit {
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
                        statements: vec![Stmt::TryStmt(TryStmt {
                            resources: vec![],
                            try_body: BlockStmt {
                                statements: vec![],
                                line: 1,
                                end_line: 2,
                            },
                            catch_clauses: vec![CatchClause {
                                exception_type: Some("Exception".to_string()),
                                exception_name: Some("e".to_string()),
                                body: BlockStmt {
                                    statements: vec![],
                                    line: 3,
                                    end_line: 4,
                                },
                                line: 3,
                            }],
                            finally_body: None,
                            line: 1,
                        })],
                        line: 1,
                        end_line: 5,
                    }),
                    line: 1,
                    end_line: 5,
                })],
                line: 1,
                end_line: 5,
            })],
            source_file: make_violation_file(),
            source_lines: vec![],
            raw_json: String::new(),
        };

        let rule = EmptyCatchRule::new();
        let vs = rule.check_unit(&unit);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].rule_id.0, "J008");
        assert_eq!(vs[0].line, 3);
    }

    #[test]
    fn ignores_non_empty_catch() {
        let unit = CompilationUnit {
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
                        statements: vec![Stmt::TryStmt(TryStmt {
                            resources: vec![],
                            try_body: BlockStmt {
                                statements: vec![],
                                line: 1,
                                end_line: 2,
                            },
                            catch_clauses: vec![CatchClause {
                                exception_type: Some("Exception".to_string()),
                                exception_name: Some("e".to_string()),
                                body: BlockStmt {
                                    statements: vec![Stmt::ExpressionStmt(ExprStmt {
                                        expr: Expr::MethodCallExpr(MethodCallExpr {
                                            callee: Some("e".to_string()),
                                            method_name: "printStackTrace".to_string(),
                                            arguments: vec![],
                                            line: 4,
                                        }),
                                        line: 4,
                                    })],
                                    line: 3,
                                    end_line: 5,
                                },
                                line: 3,
                            }],
                            finally_body: None,
                            line: 1,
                        })],
                        line: 1,
                        end_line: 6,
                    }),
                    line: 1,
                    end_line: 6,
                })],
                line: 1,
                end_line: 6,
            })],
            source_file: make_violation_file(),
            source_lines: vec![],
            raw_json: String::new(),
        };

        let rule = EmptyCatchRule::new();
        let vs = rule.check_unit(&unit);
        assert_eq!(vs.len(), 0);
    }
}
