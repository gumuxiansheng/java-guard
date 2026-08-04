//! Pattern 匹配器：遍历 AST，根据 Pattern 定义收集违规。

use crate::rule::{MatchValue, Pattern, PatternKind};
use guard_core::rule::Violation;
use java_ast::ast::{
    Annotation, CompilationUnit, Expr, MemberDecl, MethodDecl, TypeDecl,
};
use regex::Regex;
use std::collections::BTreeMap;

/// 在编译单元中执行 pattern 匹配，返回违规列表。
///
/// `file` 是当前文件路径（用于 Violation），`rule_id` / `severity` / `message` 来自规则。
/// `message` 中的 `{callee}` / `{name}` / `{line}` 等占位符会被替换为实际匹配值。
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
                if let Some(mut ctx) = match_fields_import(imp, &pattern.match_fields) {
                    ctx.push(("line", imp.line.to_string()));
                    let msg = render_message(message, &ctx);
                    violations.push(Violation::new(rule_id, severity, file, imp.line, msg));
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
                    if let Some(mut ctx) = match_fields_class(cd, &pattern.match_fields) {
                        ctx.push(("line", cd.line.to_string()));
                        let msg = render_message(message, &ctx);
                        violations.push(Violation::new(rule_id, severity, file, cd.line, msg));
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
            if let Some(mut ctx) = match_fields_method_call(mc, &pattern.match_fields) {
                ctx.push(("line", mc.line.to_string()));
                let msg = render_message(message, &ctx);
                out.push(Violation::new(rule_id, severity, file, mc.line, msg));
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

/// 匹配 MethodCall 的字段；命中返回用于渲染 message 的上下文，否则返回 None。
fn match_fields_method_call(
    mc: &java_ast::ast::MethodCallExpr,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    let mut ctx: Vec<(&'static str, String)> = Vec::new();
    for (key, expected) in fields {
        let resolved: Option<(&str, &'static str)> = match key.as_str() {
            "callee" => Some((mc.callee.as_deref().unwrap_or(""), "callee")),
            "method" | "method_name" => Some((&mc.method_name, "method")),
            _ => None,
        };
        let (actual, label) = match resolved {
            Some(r) => r,
            // 未知键已在加载期校验拦截，这里安全跳过
            None => continue,
        };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
}

// ── Import 匹配 ──

/// 匹配 Import 的字段；命中返回上下文，否则 None。
fn match_fields_import(
    imp: &java_ast::ast::ImportDecl,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    let mut ctx: Vec<(&'static str, String)> = Vec::new();
    for (key, expected) in fields {
        let matched = match key.as_str() {
            "package" => value_matches(expected, &imp.package),
            "is_wildcard" => {
                let want = expected
                    .as_str()
                    .map(|s| s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                imp.is_wildcard == want
            }
            "is_static" => {
                let want = expected
                    .as_str()
                    .map(|s| s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                imp.is_static == want
            }
            _ => continue,
        };
        if !matched {
            return None;
        }
        if key == "package" {
            ctx.push(("package", imp.package.clone()));
        }
    }
    Some(ctx)
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
        if let Some(mut ctx) = match_fields_annotation(ann, &pattern.match_fields) {
            ctx.push(("line", ann.line.to_string()));
            let msg = render_message(message, &ctx);
            out.push(Violation::new(rule_id, severity, file, ann.line, msg));
        }
    }
    // 递归检查成员上的注解
    for m in members {
        let member_anns = member_annotations(m);
        for ann in member_anns {
            if let Some(mut ctx) = match_fields_annotation(ann, &pattern.match_fields) {
                ctx.push(("line", ann.line.to_string()));
                let msg = render_message(message, &ctx);
                out.push(Violation::new(rule_id, severity, file, ann.line, msg));
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

/// 匹配 Annotation 的字段；命中返回上下文，否则 None。
fn match_fields_annotation(
    ann: &Annotation,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    // 预置节点固有属性，保证消息模板占位符始终可渲染。
    let mut ctx: Vec<(&'static str, String)> = vec![("name", ann.name.clone())];
    for (key, expected) in fields {
        let (actual, label) = match key.as_str() {
            "name" | "type" => (&ann.name, "name"),
            _ => continue,
        };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
}

// ── ClassDeclaration 匹配 ──

/// 匹配 ClassDecl 的字段；命中返回上下文，否则 None。
fn match_fields_class(
    cd: &java_ast::ast::ClassDecl,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    // 预置节点固有属性，保证消息模板占位符始终可渲染。
    let mut ctx: Vec<(&'static str, String)> = vec![
        ("name", cd.name.clone()),
        ("modifier", cd.modifiers.join(", ")),
    ];
    for (key, expected) in fields {
        let resolved: Option<(&str, &'static str)> = match key.as_str() {
            "name" => Some((&cd.name, "name")),
            "modifier" | "modifiers" => {
                let found = cd.modifiers.iter().any(|m| value_matches(expected, m));
                ctx.push(("modifier", cd.modifiers.join(", ")));
                if !found {
                    return None;
                }
                continue;
            }
            _ => continue,
        };
        let (actual, label) = match resolved { Some(r) => r, None => continue };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
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
            if let Some(mut ctx) = match_fields_class(cd, &pattern.match_fields) {
                ctx.push(("line", cd.line.to_string()));
                let msg = render_message(message, &ctx);
                out.push(Violation::new(rule_id, severity, file, cd.line, msg));
            }
            match_members_for_class_decl(&cd.members, pattern, file, rule_id, severity, message, out);
        }
    }
}

// ── MethodDeclaration 匹配 ──
//
// 与 MethodCall 保持一致：递归进入嵌套类，确保深层嵌套类中的方法也能被检出。

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
        match_member_for_method_decl(m, pattern, file, rule_id, severity, message, out);
    }
}

fn match_member_for_method_decl(
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
            if let Some(mut ctx) = match_fields_method_decl(md, &pattern.match_fields) {
                ctx.push(("line", md.line.to_string()));
                let msg = render_message(message, &ctx);
                out.push(Violation::new(rule_id, severity, file, md.line, msg));
            }
        }
        MemberDecl::ClassDeclaration(cd) => {
            for m in &cd.members {
                match_member_for_method_decl(m, pattern, file, rule_id, severity, message, out);
            }
        }
        _ => {}
    }
}

/// 匹配 MethodDecl 的字段；命中返回上下文，否则 None。
fn match_fields_method_decl(
    md: &MethodDecl,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    // 预置节点固有属性：即使规则没有在该字段上做匹配，
    // 消息模板里的 `{name}` / `{return_type}` / `{modifier}` 也能正常渲染。
    let mut ctx: Vec<(&'static str, String)> = vec![
        ("name", md.name.clone()),
        ("return_type", md.return_type.clone().unwrap_or_default()),
        ("modifier", md.modifiers.join(", ")),
    ];
    for (key, expected) in fields {
        let resolved: Option<(&str, &'static str)> = match key.as_str() {
            "name" => Some((&md.name, "name")),
            "return_type" => Some((md.return_type.as_deref().unwrap_or(""), "return_type")),
            "modifier" | "modifiers" => {
                let found = md.modifiers.iter().any(|m| value_matches(expected, m));
                ctx.push(("modifier", md.modifiers.join(", ")));
                if !found {
                    return None;
                }
                continue;
            }
            _ => continue,
        };
        let (actual, label) = match resolved { Some(r) => r, None => continue };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
}

// ── FieldDeclaration 匹配 ──
//
// 与 MethodDeclaration 保持一致：递归进入嵌套类。

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
        match_member_for_field_decl(m, pattern, file, rule_id, severity, message, out);
    }
}

fn match_member_for_field_decl(
    m: &MemberDecl,
    pattern: &Pattern,
    file: &str,
    rule_id: &str,
    severity: guard_core::rule::Severity,
    message: &str,
    out: &mut Vec<Violation>,
) {
    match m {
        MemberDecl::FieldDeclaration(fd) => {
            if let Some(mut ctx) = match_fields_field_decl(fd, &pattern.match_fields) {
                ctx.push(("line", fd.line.to_string()));
                let msg = render_message(message, &ctx);
                out.push(Violation::new(rule_id, severity, file, fd.line, msg));
            }
        }
        MemberDecl::ClassDeclaration(cd) => {
            for m in &cd.members {
                match_member_for_field_decl(m, pattern, file, rule_id, severity, message, out);
            }
        }
        _ => {}
    }
}

/// 匹配 FieldDecl 的字段；命中返回上下文，否则 None。
fn match_fields_field_decl(
    fd: &java_ast::ast::FieldDecl,
    fields: &BTreeMap<String, MatchValue>,
) -> Option<Vec<(&'static str, String)>> {
    // 预置节点固有属性，保证消息模板占位符始终可渲染。
    let mut ctx: Vec<(&'static str, String)> = vec![
        ("name", fd.name.clone()),
        ("field_type", fd.field_type.clone().unwrap_or_default()),
        ("modifier", fd.modifiers.join(", ")),
    ];
    for (key, expected) in fields {
        let resolved: Option<(&str, &'static str)> = match key.as_str() {
            "name" => Some((&fd.name, "name")),
            "field_type" | "type" => Some((fd.field_type.as_deref().unwrap_or(""), "field_type")),
            "modifier" | "modifiers" => {
                let found = fd.modifiers.iter().any(|m| value_matches(expected, m));
                ctx.push(("modifier", fd.modifiers.join(", ")));
                if !found {
                    return None;
                }
                continue;
            }
            _ => continue,
        };
        let (actual, label) = match resolved { Some(r) => r, None => continue };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
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

/// 判断 MatchValue 是否匹配给定文本（Single 精确/glob/正则，Any 任意一个命中）。
fn value_matches(v: &MatchValue, text: &str) -> bool {
    match v {
        MatchValue::Single(s) => regex_match(s, text),
        MatchValue::Any(list) => list.iter().any(|s| regex_match(s, text)),
    }
}

/// 用 (key, value) 替换模板中的 `{key}` 占位符；未提供的占位符原样保留。
fn render_message(template: &str, ctx: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in ctx {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
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
    // 如果含正则元字符（^ $ [] + | ? 等），用正则匹配
    if pattern.starts_with('^')
        || pattern.ends_with('$')
        || pattern.contains('[')
        || pattern.contains('+')
        || pattern.contains('|')
        || pattern.contains("\\")
    {
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
                ("callee".to_string(), MatchValue::Single("System.out".to_string())),
                ("method".to_string(), MatchValue::Single("println".to_string())),
            ].into_iter().collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "call {callee}.{method} at {line}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 5);
        assert_eq!(vs[0].message, "call System.out.println at 5");
    }

    #[test]
    fn match_import_wildcard() {
        let unit = make_unit();
        let pattern = Pattern {
            kind: PatternKind::Import,
            match_fields: [("is_wildcard".to_string(), MatchValue::Single("true".to_string()))]
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
            match_fields: [("name".to_string(), MatchValue::Single("^[a-z]".to_string()))]
                .into_iter()
                .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J004", Severity::Minor, "test");
        assert_eq!(vs.len(), 1); // "badName" 以小写开头
    }

    #[test]
    fn match_value_any_of() {
        let unit = make_unit();
        // any_of：callee 为 System.out 或 System.err 都应命中
        let pattern = Pattern {
            kind: PatternKind::MethodCall,
            match_fields: [
                ("callee".to_string(), MatchValue::Any(vec!["System.out".to_string(), "System.err".to_string()])),
                ("method".to_string(), MatchValue::Any(vec!["print".to_string(), "println".to_string(), "printf".to_string()])),
            ].into_iter().collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "no sysout");
        assert_eq!(vs.len(), 1);
    }

    #[test]
    fn match_method_decl_nested_class() {
        // 深层嵌套类里的方法也应被 MethodDeclaration 检出（递归一致）
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Outer".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::ClassDeclaration(ClassDecl {
                    name: "Inner".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    extends: None,
                    implements: vec![],
                    members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                        name: "BADNAME".to_string(),
                        modifiers: vec![],
                        annotations: vec![],
                        return_type: None,
                        parameters: vec![],
                        body: None,
                        line: 10,
                        end_line: 10,
                    })],
                    line: 8,
                    end_line: 12,
                })],
                line: 1,
                end_line: 20,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::MethodDeclaration,
            match_fields: [("name".to_string(), MatchValue::Single("^[A-Z]+$".to_string()))]
                .into_iter()
                .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "T.java", "J005", Severity::Minor, "bad method {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 10);
        assert_eq!(vs[0].message, "bad method BADNAME");
    }

    // ── value_matches 单测（精确 / glob / 正则 / any_of）──

    #[test]
    fn value_matches_exact() {
        assert!(value_matches(&MatchValue::Single("System.out".to_string()), "System.out"));
        assert!(!value_matches(&MatchValue::Single("System.out".to_string()), "System.err"));
    }

    #[test]
    fn value_matches_glob() {
        assert!(value_matches(&MatchValue::Single("Sys*".to_string()), "System.out"));
        assert!(value_matches(&MatchValue::Single("*out".to_string()), "System.out"));
        assert!(!value_matches(&MatchValue::Single("Foo*".to_string()), "Bar"));
        // 裸 "*" 匹配任意
        assert!(value_matches(&MatchValue::Single("*".to_string()), "anything"));
    }

    #[test]
    fn value_matches_regex() {
        assert!(value_matches(&MatchValue::Single("^[a-z]".to_string()), "badName"));
        assert!(!value_matches(&MatchValue::Single("^[a-z]".to_string()), "GoodName"));
        // 含正则元字符才走正则；纯字符串走精确/通配
        assert!(value_matches(&MatchValue::Single("^J[0-9]+$".to_string()), "J007"));
        assert!(!value_matches(&MatchValue::Single("^J[0-9]+$".to_string()), "X007"));
    }

    #[test]
    fn value_matches_any_of() {
        let v = MatchValue::Any(vec!["System.out".to_string(), "System.err".to_string()]);
        assert!(value_matches(&v, "System.err"));
        assert!(value_matches(&v, "System.out"));
        assert!(!value_matches(&v, "java.lang"));
    }

    // ── render_message 单测 ──

    #[test]
    fn render_message_substitutes() {
        let ctx = vec![
            ("callee", "System.out".to_string()),
            ("method", "println".to_string()),
        ];
        assert_eq!(
            render_message("{callee}.{method} called", &ctx),
            "System.out.println called"
        );
    }

    #[test]
    fn render_message_keeps_unknown() {
        let ctx = vec![("callee", "X".to_string())];
        assert_eq!(
            render_message("{callee} {unknown}", &ctx),
            "X {unknown}"
        );
    }

    // ── 各 PatternKind 匹配单测 ──

    #[test]
    fn match_annotation_by_name() {
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Controller".to_string(),
                modifiers: vec![],
                annotations: vec![Annotation {
                    name: "RestController".to_string(),
                    members: vec![],
                    line: 1,
                }],
                extends: None,
                implements: vec![],
                members: vec![],
                line: 1,
                end_line: 5,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::Annotation,
            match_fields: [(
                "name".to_string(),
                MatchValue::Single("RestController".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "T.java", "J009", Severity::Minor, "no {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].message, "no RestController");
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn match_field_declaration() {
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "C".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::FieldDeclaration(FieldDecl {
                    name: "myField".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    field_type: Some("int".to_string()),
                    initializer: None,
                    line: 3,
                })],
                line: 1,
                end_line: 10,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::FieldDeclaration,
            match_fields: [(
                "field_type".to_string(),
                MatchValue::Single("int".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(
            &pattern,
            &unit,
            "T.java",
            "J007",
            Severity::Minor,
            "field {name} type {field_type}",
        );
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].message, "field myField type int");
    }

    #[test]
    fn match_class_decl_with_modifier() {
        let unit = make_unit(); // badName 类, public 修饰符
        let pattern = Pattern {
            kind: PatternKind::ClassDeclaration,
            match_fields: [(
                "modifier".to_string(),
                MatchValue::Single("public".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(
            &pattern,
            &unit,
            "Test.java",
            "J004",
            Severity::Minor,
            "class {name} mod {modifier}",
        );
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].message, "class badName mod public");
    }

    #[test]
    fn match_method_decl_return_type() {
        let unit = make_unit(); // doStuff 返回 void
        let pattern = Pattern {
            kind: PatternKind::MethodDeclaration,
            match_fields: [(
                "return_type".to_string(),
                MatchValue::Single("void".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(
            &pattern,
            &unit,
            "Test.java",
            "J005",
            Severity::Minor,
            "method returns {return_type}",
        );
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].message, "method returns void");
    }

    #[test]
    fn match_import_package_glob() {
        let unit = make_unit();
        let pattern = Pattern {
            kind: PatternKind::Import,
            match_fields: [(
                "package".to_string(),
                MatchValue::Single("java.util.*".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J003", Severity::Minor, "import {package}");
        // glob `java.util.*` 同时命中 `java.util.*` 与 `java.util.List`
        assert_eq!(vs.len(), 2);
        let msgs: Vec<&str> = vs.iter().map(|v| v.message.as_str()).collect();
        assert!(msgs.contains(&"import java.util.*"), "got: {msgs:?}");
        assert!(msgs.contains(&"import java.util.List"), "got: {msgs:?}");
    }

    #[test]
    fn match_method_call_in_arguments() {
        // System.out.println(foo()) —— 嵌套调用应被递归检出
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "C".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                    name: "m".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    return_type: None,
                    parameters: vec![],
                    body: Some(BlockStmt {
                        statements: vec![Stmt::ExpressionStmt(ExprStmt {
                            expr: Expr::MethodCallExpr(MethodCallExpr {
                                callee: Some("System.out".to_string()),
                                method_name: "println".to_string(),
                                arguments: vec![Expr::MethodCallExpr(MethodCallExpr {
                                    callee: None,
                                    method_name: "foo".to_string(),
                                    arguments: vec![],
                                    line: 6,
                                })],
                                line: 5,
                            }),
                            line: 5,
                        })],
                        line: 4,
                        end_line: 7,
                    }),
                    line: 4,
                    end_line: 7,
                })],
                line: 1,
                end_line: 8,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::MethodCall,
            match_fields: [
                ("callee".to_string(), MatchValue::Single("System.out".to_string())),
                ("method".to_string(), MatchValue::Single("println".to_string())),
            ]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 5);
    }

    #[test]
    fn match_method_call_in_nested_class() {
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Outer".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::ClassDeclaration(ClassDecl {
                    name: "Inner".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    extends: None,
                    implements: vec![],
                    members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                        name: "inner".to_string(),
                        modifiers: vec![],
                        annotations: vec![],
                        return_type: None,
                        parameters: vec![],
                        body: Some(BlockStmt {
                            statements: vec![Stmt::ExpressionStmt(ExprStmt {
                                expr: Expr::MethodCallExpr(MethodCallExpr {
                                    callee: Some("System.out".to_string()),
                                    method_name: "println".to_string(),
                                    arguments: vec![],
                                    line: 11,
                                }),
                                line: 11,
                            })],
                            line: 10,
                            end_line: 12,
                        }),
                        line: 10,
                        end_line: 12,
                    })],
                    line: 8,
                    end_line: 14,
                })],
                line: 1,
                end_line: 16,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::MethodCall,
            match_fields: [
                ("callee".to_string(), MatchValue::Single("System.out".to_string())),
                ("method".to_string(), MatchValue::Single("println".to_string())),
            ]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 11);
    }

    #[test]
    fn match_any_of_no_match_yields_no_violation() {
        let unit = make_unit(); // callee 是 System.out
        let pattern = Pattern {
            kind: PatternKind::MethodCall,
            match_fields: [
                (
                    "callee".to_string(),
                    MatchValue::Any(vec!["System.err".to_string()]),
                ),
                ("method".to_string(), MatchValue::Single("println".to_string())),
            ]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "x");
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn match_class_decl_nested_class() {
        // 嵌套类名称违反 PascalCase 也应被 ClassDeclaration 递归检出
        let unit = CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Outer".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::ClassDeclaration(ClassDecl {
                    name: "innerBad".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    extends: None,
                    implements: vec![],
                    members: vec![],
                    line: 8,
                    end_line: 14,
                })],
                line: 1,
                end_line: 16,
            })],
            source_file: "T.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        };
        let pattern = Pattern {
            kind: PatternKind::ClassDeclaration,
            match_fields: [(
                "name".to_string(),
                MatchValue::Single("^[a-z]".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let vs = match_pattern(&pattern, &unit, "T.java", "J004", Severity::Minor, "bad class {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 8);
        assert_eq!(vs[0].message, "bad class innerBad");
    }
}
