//! Pattern 匹配器：遍历 AST，根据 Pattern 定义收集违规。

use crate::rule::{Pattern, PatternKind};
use guard_core::rule::Violation;
use java_ast::ast::{
    Annotation, CompilationUnit, Expr, MemberDecl, MethodDecl, TypeDecl,
};
use regex::Regex;

/// 在编译单元中执行 pattern 匹配，返回违规列表。
///
/// `file` 是当前文件路径（用于 Violation），`rule_id` / `severity` / `message` 来自规则。
pub fn match_pattern(
    pattern: &Pattern,
    unit: &CompilationUnit,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    match pattern.kind {
        PatternKind::MethodCall => {
            for ty in &unit.types {
                match_type_for_method_call(ty, pattern, file, rule_id, severity, message, &mut violations);
            }
        }
        PatternKind::Import => {
            for imp in &unit.imports {
                if match_fields_import(imp, &pattern.match_fields) {
                    violations.push(Violation::new(rule_id, severity, file, imp.line, message));
                }
            }
        }
        PatternKind::Annotation => {
            for ty in &unit.types {
                match_type_for_annotation(ty, pattern, file, rule_id, severity, message, &mut violations);
            }
        }
        PatternKind::ClassDeclaration => {
            for ty in &unit.types {
                if let TypeDecl::ClassDeclaration(cd) = ty {
                    if match_fields_class(cd, &pattern.match_fields) {
                        violations.push(Violation::new(rule_id, severity, file, cd.line, message));
                    }
                    // 递归检查嵌套类
                    match_members_for_class_decl(&cd.members, pattern, file, rule_id, severity, message, &mut violations);
                }
            }
        }
        PatternKind::MethodDeclaration => {
            for ty in &unit.types {
                match_type_for_method_decl(ty, pattern, file, rule_id, severity, message, &mut violations);
            }
        }
        PatternKind::FieldDeclaration => {
            for ty in &unit.types {
                match_type_for_field_decl(ty, pattern, file, rule_id, severity, message, &mut violations);
            }
        }
    }

    violations
}

// ── MethodCall 匹配 ──

fn match_type_for_method_call(
    ty: &TypeDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    let members = type_members(ty);
    for m in members {
        match_member_for_method_call(m, pattern, file, rule_id, severity, message, out);
    }
}

fn match_member_for_method_call(
    m: &MemberDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    match m {
        MemberDecl::MethodDeclaration(md) => {
            if let Some(body) = &md.body {
                walk_block_for_method_call(body, pattern, file, rule_id, severity, message, out);
            }
        }
        MemberDecl::ConstructorDeclaration(cd) => {
            if let Some(body) = &cd.body {
                walk_block_for_method_call(body, pattern, file, rule_id, severity, message, out);
            }
        }
        MemberDecl::ClassDeclaration(inner) => {
            for m in &inner.members {
                match_member_for_method_call(m, pattern, file, rule_id, severity, message, out);
            }
        }
        _ => {}
    }
}

fn walk_block_for_method_call(
    block: &java_ast::ast::BlockStmt,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    for stmt in &block.statements {
        walk_stmt_for_method_call(stmt, pattern, file, rule_id, severity, message, out);
    }
}

fn walk_stmt_for_method_call(
    stmt: &java_ast::ast::Stmt,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    use java_ast::ast::Stmt;
    match stmt {
        Stmt::ExpressionStmt(es) => {
            walk_expr_for_method_call(&es.expr, pattern, file, rule_id, severity, message, out);
        }
        Stmt::VariableDeclarationStmt(vds) => {
            for d in &vds.declarations {
                if let Some(init) = &d.initializer {
                    walk_expr_for_method_call(init, pattern, file, rule_id, severity, message, out);
                }
            }
        }
        Stmt::IfStmt(is) => {
            walk_expr_for_method_call(&is.condition, pattern, file, rule_id, severity, message, out);
            walk_stmt_for_method_call(&is.then_stmt, pattern, file, rule_id, severity, message, out);
            if let Some(else_stmt) = &is.else_stmt {
                walk_stmt_for_method_call(else_stmt, pattern, file, rule_id, severity, message, out);
            }
        }
        Stmt::ForStmt(fs) => {
            if let Some(init) = &fs.initialization {
                walk_expr_for_method_call(init, pattern, file, rule_id, severity, message, out);
            }
            if let Some(cond) = &fs.condition {
                walk_expr_for_method_call(cond, pattern, file, rule_id, severity, message, out);
            }
            walk_stmt_for_method_call(&fs.body, pattern, file, rule_id, severity, message, out);
        }
        Stmt::WhileStmt(ws) => {
            walk_expr_for_method_call(&ws.condition, pattern, file, rule_id, severity, message, out);
            walk_stmt_for_method_call(&ws.body, pattern, file, rule_id, severity, message, out);
        }
        Stmt::DoStmt(ds) => {
            walk_stmt_for_method_call(&ds.body, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&ds.condition, pattern, file, rule_id, severity, message, out);
        }
        Stmt::TryStmt(ts) => {
            walk_block_for_method_call(&ts.try_body, pattern, file, rule_id, severity, message, out);
            for cc in &ts.catch_clauses {
                walk_block_for_method_call(&cc.body, pattern, file, rule_id, severity, message, out);
            }
            if let Some(fin) = &ts.finally_body {
                walk_block_for_method_call(fin, pattern, file, rule_id, severity, message, out);
            }
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(expr) = &rs.expr {
                walk_expr_for_method_call(expr, pattern, file, rule_id, severity, message, out);
            }
        }
        Stmt::ThrowStmt(ts) => {
            walk_expr_for_method_call(&ts.expr, pattern, file, rule_id, severity, message, out);
        }
        Stmt::BlockStmt(bs) => {
            walk_block_for_method_call(bs, pattern, file, rule_id, severity, message, out);
        }
        Stmt::SwitchStmt(ss) => {
            walk_expr_for_method_call(&ss.selector, pattern, file, rule_id, severity, message, out);
            for case in &ss.cases {
                for s in &case.statements {
                    walk_stmt_for_method_call(s, pattern, file, rule_id, severity, message, out);
                }
            }
        }
        Stmt::SynchronizedStmt(ss) => {
            walk_expr_for_method_call(&ss.expr, pattern, file, rule_id, severity, message, out);
            walk_block_for_method_call(&ss.body, pattern, file, rule_id, severity, message, out);
        }
        _ => {}
    }
}

fn walk_expr_for_method_call(
    expr: &Expr,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    match expr {
        Expr::MethodCallExpr(mc) => {
            if match_fields_method_call(mc, &pattern.match_fields) {
                out.push(Violation::new(rule_id, severity, file, mc.line, message));
            }
            // 递归检查参数中的嵌套调用
            for arg in &mc.arguments {
                walk_expr_for_method_call(arg, pattern, file, rule_id, severity, message, out);
            }
        }
        Expr::BinaryExpr(be) => {
            walk_expr_for_method_call(&be.left, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&be.right, pattern, file, rule_id, severity, message, out);
        }
        Expr::UnaryExpr(ue) => {
            walk_expr_for_method_call(&ue.expr, pattern, file, rule_id, severity, message, out);
        }
        Expr::AssignExpr(ae) => {
            walk_expr_for_method_call(&ae.target, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&ae.value, pattern, file, rule_id, severity, message, out);
        }
        Expr::FieldAccessExpr(fa) => {
            walk_expr_for_method_call(&fa.target, pattern, file, rule_id, severity, message, out);
        }
        Expr::CastExpr(ce) => {
            walk_expr_for_method_call(&ce.expr, pattern, file, rule_id, severity, message, out);
        }
        Expr::ConditionalExpr(ce) => {
            walk_expr_for_method_call(&ce.condition, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&ce.then_expr, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&ce.else_expr, pattern, file, rule_id, severity, message, out);
        }
        Expr::ArrayAccessExpr(aa) => {
            walk_expr_for_method_call(&aa.array, pattern, file, rule_id, severity, message, out);
            walk_expr_for_method_call(&aa.index, pattern, file, rule_id, severity, message, out);
        }
        Expr::EnclosedExpr(inner) => {
            walk_expr_for_method_call(inner, pattern, file, rule_id, severity, message, out);
        }
        Expr::ObjectCreationExpr(oc) => {
            for arg in &oc.arguments {
                walk_expr_for_method_call(arg, pattern, file, rule_id, severity, message, out);
            }
        }
        _ => {}
    }
}

fn match_fields_method_call(
    mc: &java_ast::ast::MethodCallExpr,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let actual = match key.as_str() {
            "callee" => mc.callee.as_deref().unwrap_or(""),
            "method" | "method_name" => &mc.method_name,
            _ => continue,
        };
        if !glob_match(expected, actual) {
            return false;
        }
    }
    true
}

// ── Import 匹配 ──

fn match_fields_import(
    imp: &java_ast::ast::ImportDecl,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let matched = match key.as_str() {
            "package" => glob_match(expected, &imp.package),
            "is_wildcard" => {
                let want = expected.eq_ignore_ascii_case("true");
                imp.is_wildcard == want
            }
            "is_static" => {
                let want = expected.eq_ignore_ascii_case("true");
                imp.is_static == want
            }
            _ => true,
        };
        if !matched {
            return false;
        }
    }
    true
}

// ── Annotation 匹配 ──

fn match_type_for_annotation(
    ty: &TypeDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    // 检查类型上的注解
    let (annotations, members) = type_annotations_and_members(ty);
    for ann in annotations {
        if match_fields_annotation(ann, &pattern.match_fields) {
            out.push(Violation::new(rule_id, severity, file, ann.line, message));
        }
    }
    // 递归检查成员上的注解
    for m in members {
        let member_anns = member_annotations(m);
        for ann in member_anns {
            if match_fields_annotation(ann, &pattern.match_fields) {
                out.push(Violation::new(rule_id, severity, file, ann.line, message));
            }
        }
        // 递归嵌套类型
        if let MemberDecl::ClassDeclaration(cd) = m {
            match_type_for_annotation(
                &TypeDecl::ClassDeclaration(cd.clone()),
                pattern, file, rule_id, severity, message, out,
            );
        }
    }
}

fn match_fields_annotation(
    ann: &Annotation,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let actual = match key.as_str() {
            "name" | "type" => &ann.name,
            _ => continue,
        };
        if !glob_match(expected, actual) {
            return false;
        }
    }
    true
}

// ── ClassDeclaration 匹配 ──

fn match_fields_class(
    cd: &java_ast::ast::ClassDecl,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let actual = match key.as_str() {
            "name" => &cd.name,
            "modifier" | "modifiers" => {
                // 修饰符匹配：期望值在修饰符列表中
                let found = cd.modifiers.iter().any(|m| glob_match(expected, m));
                if !found {
                    return false;
                }
                continue;
            }
            _ => continue,
        };
        if !regex_match(expected, actual) {
            return false;
        }
    }
    true
}

fn match_members_for_class_decl(
    members: &[MemberDecl],
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    for m in members {
        if let MemberDecl::ClassDeclaration(cd) = m {
            if match_fields_class(cd, &pattern.match_fields) {
                out.push(Violation::new(rule_id, severity, file, cd.line, message));
            }
            match_members_for_class_decl(&cd.members, pattern, file, rule_id, severity, message, out);
        }
    }
}

// ── MethodDeclaration 匹配 ──

fn match_type_for_method_decl(
    ty: &TypeDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    let members = type_members(ty);
    for m in members {
        match m {
            MemberDecl::MethodDeclaration(md) => {
                if match_fields_method_decl(md, &pattern.match_fields) {
                    out.push(Violation::new(rule_id, severity, file, md.line, message));
                }
            }
            MemberDecl::ClassDeclaration(cd) => {
                for m in &cd.members {
                    if let MemberDecl::MethodDeclaration(md) = m {
                        if match_fields_method_decl(md, &pattern.match_fields) {
                            out.push(Violation::new(rule_id, severity, file, md.line, message));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn match_fields_method_decl(
    md: &MethodDecl,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let actual = match key.as_str() {
            "name" => &md.name,
            "return_type" => md.return_type.as_deref().unwrap_or(""),
            "modifier" | "modifiers" => {
                let found = md.modifiers.iter().any(|m| glob_match(expected, m));
                if !found {
                    return false;
                }
                continue;
            }
            _ => continue,
        };
        if !regex_match(expected, actual) {
            return false;
        }
    }
    true
}

// ── FieldDeclaration 匹配 ──

fn match_type_for_field_decl(
    ty: &TypeDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    let members = type_members(ty);
    for m in members {
        if let MemberDecl::FieldDeclaration(fd) = m {
            if match_fields_field_decl(fd, &pattern.match_fields) {
                out.push(Violation::new(rule_id, severity, file, fd.line, message));
            }
        }
    }
}

fn match_fields_field_decl(
    fd: &java_ast::ast::FieldDecl,
    fields: &std::collections::BTreeMap<String, String>,
) -> bool {
    for (key, expected) in fields {
        let actual = match key.as_str() {
            "name" => &fd.name,
            "field_type" | "type" => fd.field_type.as_deref().unwrap_or(""),
            "modifier" | "modifiers" => {
                let found = fd.modifiers.iter().any(|m| glob_match(expected, m));
                if !found {
                    return false;
                }
                continue;
            }
            _ => continue,
        };
        if !regex_match(expected, actual) {
            return false;
        }
    }
    true
}

// ── 辅助函数 ──

fn type_members(ty: &TypeDecl) -> &[MemberDecl] {
    match ty {
        TypeDecl::ClassDeclaration(c) => &c.members,
        TypeDecl::InterfaceDeclaration(i) => &i.members,
        TypeDecl::EnumDeclaration(e) => &e.members,
        TypeDecl::AnnotationDeclaration(a) => &a.members,
    }
}

fn type_annotations_and_members(ty: &TypeDecl) -> (&[Annotation], &[MemberDecl]) {
    match ty {
        TypeDecl::ClassDeclaration(c) => (&c.annotations, &c.members),
        TypeDecl::InterfaceDeclaration(i) => (&i.annotations, &i.members),
        TypeDecl::EnumDeclaration(e) => (&e.annotations, &e.members),
        TypeDecl::AnnotationDeclaration(_) => (&[], &[]),
    }
}

fn member_annotations(m: &MemberDecl) -> &[Annotation] {
    match m {
        MemberDecl::FieldDeclaration(f) => &f.annotations,
        MemberDecl::MethodDeclaration(md) => &md.annotations,
        MemberDecl::ConstructorDeclaration(cd) => &cd.annotations,
        _ => &[],
    }
}

/// 通配符匹配：`*` 匹配任意字符序列。
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    // 转换为 regex
    let regex_str = pattern
        .replace('.', "\\.")
        .replace('*', ".*");
    if let Ok(re) = Regex::new(&format!("^{regex_str}$")) {
        re.is_match(text)
    } else {
        pattern == text
    }
}

/// 正则匹配：如果 pattern 以 `^` 开头或 `$` 结尾，视为正则；否则精确匹配。
fn regex_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // 如果含正则元字符，用正则匹配
    if pattern.starts_with('^') || pattern.ends_with('$') {
        if let Ok(re) = Regex::new(pattern) {
            return re.is_match(text);
        }
    }
    // 否则用通配符匹配
    glob_match(pattern, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Pattern, PatternKind};
    use guard_core::rule::Severity;
    use java_ast::ast::*;

    fn make_unit() -> CompilationUnit {
        CompilationUnit {
            package: Some("com.example".to_string()),
            imports: vec![
                ImportDecl { package: "java.util.*".to_string(), is_wildcard: true, is_static: false, line: 1 },
                ImportDecl { package: "java.util.List".to_string(), is_wildcard: false, is_static: false, line: 2 },
            ],
            types: vec![
                TypeDecl::ClassDeclaration(ClassDecl {
                    name: "badName".to_string(),
                    modifiers: vec!["public".to_string()],
                    annotations: vec![],
                    extends: None,
                    implements: vec![],
                    members: vec![
                        MemberDecl::MethodDeclaration(MethodDecl {
                            name: "doStuff".to_string(),
                            modifiers: vec!["public".to_string()],
                            annotations: vec![],
                            return_type: Some("void".to_string()),
                            parameters: vec![],
                            body: Some(BlockStmt {
                                statements: vec![
                                    Stmt::ExpressionStmt(ExprStmt {
                                        expr: Expr::MethodCallExpr(MethodCallExpr {
                                            callee: Some("System.out".to_string()),
                                            method_name: "println".to_string(),
                                            arguments: vec![],
                                            line: 5,
                                        }),
                                        line: 5,
                                    }),
                                ],
                                line: 4,
                                end_line: 6,
                            }),
                            line: 4,
                            end_line: 6,
                        }),
                    ],
                    line: 3,
                    end_line: 7,
                }),
            ],
            source_file: "Test.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    #[test]
    fn match_method_call() {
        let unit = make_unit();
        let pattern = Pattern {
            kind: PatternKind::MethodCall,
            match_fields: [
                ("callee".to_string(), "System.out".to_string()),
                ("method".to_string(), "println".to_string()),
            ].into_iter().collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "test");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 5);
    }

    #[test]
    fn match_import_wildcard() {
        let unit = make_unit();
        let pattern = Pattern {
            kind: PatternKind::Import,
            match_fields: [("is_wildcard".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J003", Severity::Minor, "test");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn match_class_name_regex() {
        let unit = make_unit();
        let pattern = Pattern {
            kind: PatternKind::ClassDeclaration,
            match_fields: [("name".to_string(), "^[a-z]".to_string())]
                .into_iter()
                .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J004", Severity::Minor, "test");
        assert_eq!(vs.len(), 1); // "badName" 以小写开头
    }
}
