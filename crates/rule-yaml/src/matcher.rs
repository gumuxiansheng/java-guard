//! Pattern 匹配器。
//!
//! 实现策略：**一次遍历 + 统一字段匹配 + 上下文谓词**。
//!
//! 早期实现为每种 [`PatternKind`] 各写一套递归函数，带来三个问题：
//! 1. **遍历覆盖面不一致**——同一条 `MethodCall` 规则，写在方法体里能被检出，
//!    写在字段初始化器 / 静态初始化块 / lambda 体内却被漏掉（各 `walk_*` 分支不全）；
//! 2. 新增节点类型要同时改多处递归，容易再次漏分支；
//! 3. 节点没有「祖先」概念，无法表达 `within: [ForEachStmt]` 这类上下文约束。
//!
//! 现在统一为：
//! - [`walk_type`] 起做**唯一一次**深度优先遍历，覆盖全部类型 / 成员 / 语句 / 表达式，
//!   遍历过程中维护「祖先 kind 栈」与「最近的宿主类型 / 宿主方法」；
//! - [`visit`] 对每个访问到的节点做统一判定：kind → 祖先谓词 → 宿主谓词 → 字段匹配。
//!
//! 新增节点类型只需在遍历里补一条分支，匹配逻辑无须改动。

use std::collections::BTreeMap;

use guard_core::rule::{Severity, Violation};
use java_ast::ast::{
    Annotation, AnnotationDecl, BlockStmt, ClassDecl, CompilationUnit, ConstructorDecl,
    EnumConstant, EnumDecl, Expr, FieldDecl, ImportDecl, InitializerDecl, InterfaceDecl, MemberDecl,
    MethodDecl, ParamDecl, Stmt, TypeDecl,
};
use regex::Regex;

use crate::rule::{MatchValue, MethodPredicate, Pattern, TypePredicate};

/// 在编译单元中执行 pattern 匹配，返回违规列表。
///
/// `file` 是当前文件路径（用于 Violation），`rule_id` / `severity` / `message` 来自规则。
/// `message` 中的 `{callee}` / `{name}` / `{line}` 等占位符会被替换为实际匹配值。
pub fn match_pattern(
    pattern: &Pattern,
    unit: &CompilationUnit,
    file: &str,
    rule_id: &str,
    severity: Severity,
    message: &str,
) -> Vec<Violation> {
    let ctx = MatchCtx {
        pattern,
        file,
        rule_id,
        severity,
        message,
    };
    let mut out = Vec::new();

    // import 位于编译单元顶部、不在任何类型之内，单独处理（且无需遍历类型树）
    let mut path = Path::default();
    for imp in &unit.imports {
        visit(&MatchNode::Import(imp), &path, &ctx, &mut out);
    }

    if pattern.kind.needs_type_walk() {
        for ty in &unit.types {
            walk_type(ty, &mut path, &ctx, &mut out);
        }
    }

    out
}

/// 单条规则的匹配上下文（不含遍历状态）。
struct MatchCtx<'p> {
    pattern: &'p Pattern,
    file: &'p str,
    rule_id: &'p str,
    severity: Severity,
    message: &'p str,
}

/// 遍历路径：命中节点所处的位置信息。
#[derive(Default)]
struct Path<'a> {
    /// 祖先节点 kind 栈（外层 → 内层，不含当前节点）。
    /// `within` / `not_within` 都基于它判定，因此这里必须包含**所有**被遍历到的节点
    /// （类型、成员、语句、表达式），而不只是「可匹配」的那几种。
    ancestors: Vec<&'static str>,
    /// 最近的宿主类型（类 / 接口 / 枚举 / 注解声明）。
    owner_type: Option<OwnerType<'a>>,
    /// 最近的宿主方法 / 构造器。字段初始化器、初始化块中为 `None`。
    owner_method: Option<OwnerMethod<'a>>,
}

/// 宿主类型视图：四种类型声明统一成一套只读访问器。
#[derive(Clone, Copy)]
enum OwnerType<'a> {
    Class(&'a ClassDecl),
    Interface(&'a InterfaceDecl),
    Enum(&'a EnumDecl),
    Annotation(&'a AnnotationDecl),
}

impl OwnerType<'_> {
    fn name(&self) -> &str {
        match self {
            OwnerType::Class(c) => &c.name,
            OwnerType::Interface(i) => &i.name,
            OwnerType::Enum(e) => &e.name,
            OwnerType::Annotation(a) => &a.name,
        }
    }

    fn annotations(&self) -> &[Annotation] {
        match self {
            OwnerType::Class(c) => &c.annotations,
            OwnerType::Interface(i) => &i.annotations,
            OwnerType::Enum(e) => &e.annotations,
            // AnnotationDecl 在 AST 模型中不带注解列表
            OwnerType::Annotation(_) => &[],
        }
    }

    fn modifiers(&self) -> &[String] {
        match self {
            OwnerType::Class(c) => &c.modifiers,
            OwnerType::Interface(i) => &i.modifiers,
            OwnerType::Enum(e) => &e.modifiers,
            OwnerType::Annotation(a) => &a.modifiers,
        }
    }

    fn is_interface(&self) -> bool {
        matches!(self, OwnerType::Interface(_))
    }
}

/// 宿主方法视图。
#[derive(Clone, Copy)]
enum OwnerMethod<'a> {
    Method(&'a MethodDecl),
    Constructor(&'a ConstructorDecl),
}

impl OwnerMethod<'_> {
    fn name(&self) -> &str {
        match self {
            OwnerMethod::Method(m) => &m.name,
            OwnerMethod::Constructor(c) => &c.name,
        }
    }

    fn return_type(&self) -> Option<&str> {
        match self {
            OwnerMethod::Method(m) => m.return_type.as_deref(),
            // 构造器没有返回类型
            OwnerMethod::Constructor(_) => None,
        }
    }

    fn annotations(&self) -> &[Annotation] {
        match self {
            OwnerMethod::Method(m) => &m.annotations,
            OwnerMethod::Constructor(c) => &c.annotations,
        }
    }

    fn modifiers(&self) -> &[String] {
        match self {
            OwnerMethod::Method(m) => &m.modifiers,
            OwnerMethod::Constructor(c) => &c.modifiers,
        }
    }

    fn parameter_types(&self) -> Vec<&str> {
        let params = match self {
            OwnerMethod::Method(m) => &m.parameters,
            OwnerMethod::Constructor(c) => &c.parameters,
        };
        params
            .iter()
            .map(|p| p.param_type.as_deref().unwrap_or(""))
            .collect()
    }

    fn parameter_names(&self) -> Vec<&str> {
        let params = match self {
            OwnerMethod::Method(m) => &m.parameters,
            OwnerMethod::Constructor(c) => &c.parameters,
        };
        params.iter().map(|p| p.name.as_str()).collect()
    }
}

/// 遍历访问到的一个 AST 节点。
///
/// 只有 6 种 `PatternKind` 对应的变体会真正参与字段匹配；
/// 其余变体存在的意义是「被访问到」——它们要作为祖先进入 [`Path::ancestors`]，
/// 并在未来新增 PatternKind 时直接可用。
enum MatchNode<'a> {
    Import(&'a ImportDecl),
    Annotation(&'a Annotation),
    Class(&'a ClassDecl),
    Interface(&'a InterfaceDecl),
    Enum(&'a EnumDecl),
    AnnotationDecl(&'a AnnotationDecl),
    Method(&'a MethodDecl),
    Constructor(&'a ConstructorDecl),
    Field(&'a FieldDecl),
    EnumConstant(&'a EnumConstant),
    Initializer(&'a InitializerDecl),
    Stmt(&'a Stmt),
    Expr(&'a Expr),
}

impl MatchNode<'_> {
    /// 节点 kind，取值与 `AstSerializer` 输出的 JSON `kind` 字段一致。
    fn kind(&self) -> &'static str {
        match self {
            MatchNode::Import(_) => "Import",
            MatchNode::Annotation(_) => "Annotation",
            MatchNode::Class(_) => "ClassDeclaration",
            MatchNode::Interface(_) => "InterfaceDeclaration",
            MatchNode::Enum(_) => "EnumDeclaration",
            MatchNode::AnnotationDecl(_) => "AnnotationDeclaration",
            MatchNode::Method(_) => "MethodDeclaration",
            MatchNode::Constructor(_) => "ConstructorDeclaration",
            MatchNode::Field(_) => "FieldDeclaration",
            MatchNode::EnumConstant(_) => "EnumConstant",
            MatchNode::Initializer(_) => "InitializerDeclaration",
            MatchNode::Stmt(s) => stmt_kind(s),
            MatchNode::Expr(e) => expr_kind(e),
        }
    }

    /// 节点行号（1-based）。`Parameter` 在 AST 模型中不带行号，返回 0。
    fn line(&self) -> usize {
        match self {
            MatchNode::Import(n) => n.line,
            MatchNode::Annotation(n) => n.line,
            MatchNode::Class(n) => n.line,
            MatchNode::Interface(n) => n.line,
            MatchNode::Enum(n) => n.line,
            MatchNode::AnnotationDecl(n) => n.line,
            MatchNode::Method(n) => n.line,
            MatchNode::Constructor(n) => n.line,
            MatchNode::Field(n) => n.line,
            MatchNode::EnumConstant(n) => n.line,
            MatchNode::Initializer(n) => n.line,
            MatchNode::Stmt(s) => stmt_line(s),
            MatchNode::Expr(e) => expr_line(e),
        }
    }
}

/// 语句 kind（与 Java 侧序列化器的 `kind` 取值一一对应）。
fn stmt_kind(s: &Stmt) -> &'static str {
    match s {
        Stmt::ExpressionStmt(_) => "ExpressionStmt",
        Stmt::VariableDeclarationStmt(_) => "VariableDeclarationStmt",
        Stmt::IfStmt(_) => "IfStmt",
        Stmt::ForStmt(_) => "ForStmt",
        Stmt::ForEachStmt(_) => "ForEachStmt",
        Stmt::WhileStmt(_) => "WhileStmt",
        Stmt::DoStmt(_) => "DoStmt",
        Stmt::TryStmt(_) => "TryStmt",
        Stmt::ReturnStmt(_) => "ReturnStmt",
        Stmt::ThrowStmt(_) => "ThrowStmt",
        Stmt::BreakStmt(_) => "BreakStmt",
        Stmt::ContinueStmt(_) => "ContinueStmt",
        Stmt::BlockStmt(_) => "BlockStmt",
        Stmt::SwitchStmt(_) => "SwitchStmt",
        Stmt::SynchronizedStmt(_) => "SynchronizedStmt",
        Stmt::EmptyStmt => "EmptyStmt",
        Stmt::UnknownStmt { .. } => "UnknownStmt",
    }
}

fn stmt_line(s: &Stmt) -> usize {
    match s {
        Stmt::ExpressionStmt(n) => n.line,
        Stmt::VariableDeclarationStmt(n) => n.line,
        Stmt::IfStmt(n) => n.line,
        Stmt::ForStmt(n) => n.line,
        Stmt::ForEachStmt(n) => n.line,
        Stmt::WhileStmt(n) => n.line,
        Stmt::DoStmt(n) => n.line,
        Stmt::TryStmt(n) => n.line,
        Stmt::ReturnStmt(n) => n.line,
        Stmt::ThrowStmt(n) => n.line,
        Stmt::BreakStmt(n) => n.line,
        Stmt::ContinueStmt(n) => n.line,
        Stmt::BlockStmt(n) => n.line,
        Stmt::SwitchStmt(n) => n.line,
        Stmt::SynchronizedStmt(n) => n.line,
        Stmt::EmptyStmt => 0,
        Stmt::UnknownStmt { line, .. } => *line,
    }
}

/// 表达式 kind（与 Java 侧序列化器的 `kind` 取值一一对应）。
fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::MethodCallExpr(_) => "MethodCallExpr",
        Expr::FieldAccessExpr(_) => "FieldAccessExpr",
        Expr::NameExpr(_) => "NameExpr",
        Expr::LiteralExpr(_) => "LiteralExpr",
        Expr::BinaryExpr(_) => "BinaryExpr",
        Expr::UnaryExpr(_) => "UnaryExpr",
        Expr::AssignExpr(_) => "AssignExpr",
        Expr::CastExpr(_) => "CastExpr",
        Expr::ConditionalExpr(_) => "ConditionalExpr",
        Expr::ArrayAccessExpr(_) => "ArrayAccessExpr",
        Expr::ArrayCreationExpr(_) => "ArrayCreationExpr",
        Expr::ObjectCreationExpr(_) => "ObjectCreationExpr",
        Expr::ThisExpr(_) => "ThisExpr",
        Expr::SuperExpr(_) => "SuperExpr",
        Expr::InstanceOfExpr(_) => "InstanceOfExpr",
        Expr::LambdaExpr(_) => "LambdaExpr",
        Expr::MethodReferenceExpr(_) => "MethodReferenceExpr",
        Expr::VariableDeclarationExpr(_) => "VariableDeclarationExpr",
        Expr::EnclosedExpr { .. } => "EnclosedExpr",
        Expr::UnknownExpr { .. } => "UnknownExpr",
    }
}

fn expr_line(e: &Expr) -> usize {
    match e {
        Expr::MethodCallExpr(n) => n.line,
        Expr::FieldAccessExpr(n) => n.line,
        Expr::NameExpr(n) => n.line,
        Expr::LiteralExpr(n) => n.line,
        Expr::BinaryExpr(n) => n.line,
        Expr::UnaryExpr(n) => n.line,
        Expr::AssignExpr(n) => n.line,
        Expr::CastExpr(n) => n.line,
        Expr::ConditionalExpr(n) => n.line,
        Expr::ArrayAccessExpr(n) => n.line,
        Expr::ArrayCreationExpr(n) => n.line,
        Expr::ObjectCreationExpr(n) => n.line,
        Expr::ThisExpr(n) => n.line,
        Expr::SuperExpr(n) => n.line,
        Expr::InstanceOfExpr(n) => n.line,
        Expr::LambdaExpr(n) => n.line,
        Expr::MethodReferenceExpr(n) => n.line,
        Expr::VariableDeclarationExpr(n) => n.line,
        Expr::EnclosedExpr { line, .. } => *line,
        Expr::UnknownExpr { line, .. } => *line,
    }
}

// ── 遍历 ──

fn walk_type<'a>(ty: &'a TypeDecl, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    match ty {
        TypeDecl::ClassDeclaration(cd) => walk_class(cd, path, ctx, out),
        TypeDecl::InterfaceDeclaration(id) => walk_interface(id, path, ctx, out),
        TypeDecl::EnumDeclaration(ed) => walk_enum(ed, path, ctx, out),
        TypeDecl::AnnotationDeclaration(ad) => walk_annotation_decl(ad, path, ctx, out),
    }
}

/// 类型声明的公共骨架：设置宿主类型 → 访问自身与注解 → 遍历成员 → 还原。
///
/// 进入类型时必须把 `owner_method` 清空：嵌套类的方法体内不属于外层方法。
fn walk_class<'a>(cd: &'a ClassDecl, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    let saved_type = path.owner_type;
    let saved_method = path.owner_method;
    path.owner_type = Some(OwnerType::Class(cd));
    path.owner_method = None;
    path.ancestors.push("ClassDeclaration");
    visit(&MatchNode::Class(cd), path, ctx, out);
    for ann in &cd.annotations {
        visit(&MatchNode::Annotation(ann), path, ctx, out);
    }
    for m in &cd.members {
        walk_member(m, path, ctx, out);
    }
    path.ancestors.pop();
    path.owner_type = saved_type;
    path.owner_method = saved_method;
}

fn walk_interface<'a>(
    id: &'a InterfaceDecl,
    path: &mut Path<'a>,
    ctx: &MatchCtx,
    out: &mut Vec<Violation>,
) {
    let saved_type = path.owner_type;
    let saved_method = path.owner_method;
    path.owner_type = Some(OwnerType::Interface(id));
    path.owner_method = None;
    path.ancestors.push("InterfaceDeclaration");
    visit(&MatchNode::Interface(id), path, ctx, out);
    for ann in &id.annotations {
        visit(&MatchNode::Annotation(ann), path, ctx, out);
    }
    for m in &id.members {
        walk_member(m, path, ctx, out);
    }
    path.ancestors.pop();
    path.owner_type = saved_type;
    path.owner_method = saved_method;
}

fn walk_enum<'a>(ed: &'a EnumDecl, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    let saved_type = path.owner_type;
    let saved_method = path.owner_method;
    path.owner_type = Some(OwnerType::Enum(ed));
    path.owner_method = None;
    path.ancestors.push("EnumDeclaration");
    visit(&MatchNode::Enum(ed), path, ctx, out);
    for ann in &ed.annotations {
        visit(&MatchNode::Annotation(ann), path, ctx, out);
    }
    for c in &ed.constants {
        path.ancestors.push("EnumConstant");
        visit(&MatchNode::EnumConstant(c), path, ctx, out);
        for ann in &c.annotations {
            visit(&MatchNode::Annotation(ann), path, ctx, out);
        }
        path.ancestors.pop();
    }
    for m in &ed.members {
        walk_member(m, path, ctx, out);
    }
    path.ancestors.pop();
    path.owner_type = saved_type;
    path.owner_method = saved_method;
}

fn walk_annotation_decl<'a>(
    ad: &'a AnnotationDecl,
    path: &mut Path<'a>,
    ctx: &MatchCtx,
    out: &mut Vec<Violation>,
) {
    let saved_type = path.owner_type;
    let saved_method = path.owner_method;
    path.owner_type = Some(OwnerType::Annotation(ad));
    path.owner_method = None;
    path.ancestors.push("AnnotationDeclaration");
    visit(&MatchNode::AnnotationDecl(ad), path, ctx, out);
    for m in &ad.members {
        walk_member(m, path, ctx, out);
    }
    path.ancestors.pop();
    path.owner_type = saved_type;
    path.owner_method = saved_method;
}

fn walk_member<'a>(
    m: &'a MemberDecl,
    path: &mut Path<'a>,
    ctx: &MatchCtx,
    out: &mut Vec<Violation>,
) {
    match m {
        MemberDecl::FieldDeclaration(fd) => {
            // 字段初始化器不属于任何方法
            let saved = path.owner_method;
            path.owner_method = None;
            path.ancestors.push("FieldDeclaration");
            visit(&MatchNode::Field(fd), path, ctx, out);
            for ann in &fd.annotations {
                visit(&MatchNode::Annotation(ann), path, ctx, out);
            }
            if let Some(init) = &fd.initializer {
                walk_expr(init, path, ctx, out);
            }
            path.ancestors.pop();
            path.owner_method = saved;
        }
        MemberDecl::MethodDeclaration(md) => {
            let saved = path.owner_method;
            // 方法自身即为宿主：方法签名上的注解同样属于「在方法内」
            path.owner_method = Some(OwnerMethod::Method(md));
            path.ancestors.push("MethodDeclaration");
            visit(&MatchNode::Method(md), path, ctx, out);
            for ann in &md.annotations {
                visit(&MatchNode::Annotation(ann), path, ctx, out);
            }
            for p in &md.parameters {
                walk_param(p, path, ctx, out);
            }
            if let Some(body) = &md.body {
                walk_block(body, path, ctx, out);
            }
            path.ancestors.pop();
            path.owner_method = saved;
        }
        MemberDecl::ConstructorDeclaration(cd) => {
            let saved = path.owner_method;
            path.owner_method = Some(OwnerMethod::Constructor(cd));
            path.ancestors.push("ConstructorDeclaration");
            visit(&MatchNode::Constructor(cd), path, ctx, out);
            for ann in &cd.annotations {
                visit(&MatchNode::Annotation(ann), path, ctx, out);
            }
            for p in &cd.parameters {
                walk_param(p, path, ctx, out);
            }
            if let Some(body) = &cd.body {
                walk_block(body, path, ctx, out);
            }
            path.ancestors.pop();
            path.owner_method = saved;
        }
        MemberDecl::InitializerDeclaration(id) => {
            let saved = path.owner_method;
            path.owner_method = None;
            path.ancestors.push("InitializerDeclaration");
            visit(&MatchNode::Initializer(id), path, ctx, out);
            walk_block(&id.body, path, ctx, out);
            path.ancestors.pop();
            path.owner_method = saved;
        }
        // 嵌套类型：直接进入对应的类型遍历
        MemberDecl::ClassDeclaration(cd) => walk_class(cd, path, ctx, out),
        MemberDecl::InterfaceDeclaration(id) => walk_interface(id, path, ctx, out),
        MemberDecl::EnumDeclaration(ed) => walk_enum(ed, path, ctx, out),
        MemberDecl::AnnotationDeclaration(ad) => walk_annotation_decl(ad, path, ctx, out),
    }
}

fn walk_param<'a>(p: &'a ParamDecl, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    path.ancestors.push("Parameter");
    for ann in &p.annotations {
        visit(&MatchNode::Annotation(ann), path, ctx, out);
    }
    path.ancestors.pop();
}

/// 遍历一个「块」（方法体 / try 体 / catch 体 / finally 体 / synchronized 体）。
///
/// 注意与 [`walk_stmt`] 中 `Stmt::BlockStmt` 分支的分工：块作为**语句**出现时，
/// kind 由 `walk_stmt` 统一压栈；这里只服务于「不以 `Stmt` 形态存在的裸块」，
/// 因此单独压一次 `BlockStmt`，保证两条路径的祖先栈形态一致。
fn walk_block<'a>(
    b: &'a BlockStmt,
    path: &mut Path<'a>,
    ctx: &MatchCtx,
    out: &mut Vec<Violation>,
) {
    path.ancestors.push("BlockStmt");
    for s in &b.statements {
        walk_stmt(s, path, ctx, out);
    }
    path.ancestors.pop();
}

fn walk_stmt<'a>(s: &'a Stmt, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    path.ancestors.push(stmt_kind(s));
    visit(&MatchNode::Stmt(s), path, ctx, out);

    match s {
        Stmt::ExpressionStmt(es) => walk_expr(&es.expr, path, ctx, out),
        Stmt::VariableDeclarationStmt(vds) => {
            for d in &vds.declarations {
                if let Some(init) = &d.initializer {
                    walk_expr(init, path, ctx, out);
                }
            }
        }
        Stmt::IfStmt(is) => {
            walk_expr(&is.condition, path, ctx, out);
            walk_stmt(&is.then_stmt, path, ctx, out);
            if let Some(else_stmt) = &is.else_stmt {
                walk_stmt(else_stmt, path, ctx, out);
            }
        }
        Stmt::ForStmt(fs) => {
            if let Some(init) = &fs.initialization {
                walk_expr(init, path, ctx, out);
            }
            if let Some(cond) = &fs.condition {
                walk_expr(cond, path, ctx, out);
            }
            for u in &fs.update {
                walk_expr(u, path, ctx, out);
            }
            walk_stmt(&fs.body, path, ctx, out);
        }
        Stmt::ForEachStmt(fe) => {
            walk_expr(&fe.variable, path, ctx, out);
            walk_expr(&fe.iterable, path, ctx, out);
            walk_stmt(&fe.body, path, ctx, out);
        }
        Stmt::WhileStmt(ws) => {
            walk_expr(&ws.condition, path, ctx, out);
            walk_stmt(&ws.body, path, ctx, out);
        }
        Stmt::DoStmt(ds) => {
            walk_stmt(&ds.body, path, ctx, out);
            walk_expr(&ds.condition, path, ctx, out);
        }
        Stmt::TryStmt(ts) => {
            walk_block(&ts.try_body, path, ctx, out);
            for cc in &ts.catch_clauses {
                // CatchClause 在 Rust 模型中是独立结构（不是 Stmt），单独压栈，
                // 使 `within: [CatchClause]` 可判；finally 块无对应节点类型，
                // 只能由 `within: [TryStmt]` 粗粒度覆盖。
                path.ancestors.push("CatchClause");
                walk_block(&cc.body, path, ctx, out);
                path.ancestors.pop();
            }
            if let Some(fin) = &ts.finally_body {
                walk_block(fin, path, ctx, out);
            }
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(expr) = &rs.expr {
                walk_expr(expr, path, ctx, out);
            }
        }
        Stmt::ThrowStmt(ts) => walk_expr(&ts.expr, path, ctx, out),
        // 块语句：kind 已在上方压栈，这里只遍历其内部语句
        Stmt::BlockStmt(bs) => {
            for st in &bs.statements {
                walk_stmt(st, path, ctx, out);
            }
        }
        Stmt::SwitchStmt(ss) => {
            walk_expr(&ss.selector, path, ctx, out);
            for case in &ss.cases {
                if let Some(label) = &case.label {
                    walk_expr(label, path, ctx, out);
                }
                for st in &case.statements {
                    walk_stmt(st, path, ctx, out);
                }
            }
        }
        Stmt::SynchronizedStmt(ss) => {
            walk_expr(&ss.expr, path, ctx, out);
            walk_block(&ss.body, path, ctx, out);
        }
        Stmt::BreakStmt(_)
        | Stmt::ContinueStmt(_)
        | Stmt::EmptyStmt
        | Stmt::UnknownStmt { .. } => {}
    }

    path.ancestors.pop();
}

fn walk_expr<'a>(e: &'a Expr, path: &mut Path<'a>, ctx: &MatchCtx, out: &mut Vec<Violation>) {
    path.ancestors.push(expr_kind(e));
    visit(&MatchNode::Expr(e), path, ctx, out);

    match e {
        Expr::MethodCallExpr(mc) => {
            for arg in &mc.arguments {
                walk_expr(arg, path, ctx, out);
            }
        }
        Expr::FieldAccessExpr(fa) => walk_expr(&fa.target, path, ctx, out),
        Expr::BinaryExpr(be) => {
            walk_expr(&be.left, path, ctx, out);
            walk_expr(&be.right, path, ctx, out);
        }
        Expr::UnaryExpr(ue) => walk_expr(&ue.expr, path, ctx, out),
        Expr::AssignExpr(ae) => {
            walk_expr(&ae.target, path, ctx, out);
            walk_expr(&ae.value, path, ctx, out);
        }
        Expr::CastExpr(ce) => walk_expr(&ce.expr, path, ctx, out),
        Expr::ConditionalExpr(ce) => {
            walk_expr(&ce.condition, path, ctx, out);
            walk_expr(&ce.then_expr, path, ctx, out);
            walk_expr(&ce.else_expr, path, ctx, out);
        }
        Expr::ArrayAccessExpr(aa) => {
            walk_expr(&aa.array, path, ctx, out);
            walk_expr(&aa.index, path, ctx, out);
        }
        Expr::ArrayCreationExpr(ac) => {
            for v in &ac.initializer {
                walk_expr(v, path, ctx, out);
            }
        }
        Expr::ObjectCreationExpr(oc) => {
            for arg in &oc.arguments {
                walk_expr(arg, path, ctx, out);
            }
        }
        Expr::InstanceOfExpr(io) => walk_expr(&io.expr, path, ctx, out),
        // lambda 体：早期实现漏了这一支，导致 lambda 内的调用完全不被检测
        Expr::LambdaExpr(le) => walk_stmt(&le.body, path, ctx, out),
        Expr::VariableDeclarationExpr(vde) => {
            for d in &vde.declarations {
                if let Some(init) = &d.initializer {
                    walk_expr(init, path, ctx, out);
                }
            }
        }
        Expr::EnclosedExpr { inner, .. } => walk_expr(inner, path, ctx, out),
        Expr::NameExpr(_)
        | Expr::LiteralExpr(_)
        | Expr::ThisExpr(_)
        | Expr::SuperExpr(_)
        | Expr::MethodReferenceExpr(_)
        | Expr::UnknownExpr { .. } => {}
    }

    path.ancestors.pop();
}

// ── 统一判定 ──

/// 判定单个节点是否命中规则，命中则产出违规。
///
/// 判定顺序（全部通过才命中）：
/// 1. 节点 kind 是否等于 pattern 的目标 kind；
/// 2. `within` / `not_within` 祖先约束；
/// 3. `in_type` / `in_method` 宿主约束；
/// 4. `match_fields` 字段匹配。
fn visit<'a>(
    node: &MatchNode<'a>,
    path: &Path<'a>,
    ctx: &MatchCtx,
    out: &mut Vec<Violation>,
) {
    if node.kind() != ctx.pattern.kind.node_kind() {
        return;
    }
    if !ancestor_predicates_match(ctx.pattern, path) {
        return;
    }
    if !owner_predicates_match(ctx.pattern, path) {
        return;
    }
    let Some(mut fields) = match_fields_node(node, ctx.pattern) else {
        return;
    };
    let line = node.line();
    fields.push(("line", line.to_string()));
    let msg = render_message(ctx.message, &fields);
    out.push(Violation::new(ctx.rule_id, ctx.severity, ctx.file, line, msg));
}

// ── 上下文谓词 ──

/// 祖先约束：`within` 为 any_of（命中任一祖先即通过），`not_within` 为 none_of。
fn ancestor_predicates_match(pattern: &Pattern, path: &Path) -> bool {
    if !pattern.within.is_empty()
        && !pattern
            .within
            .iter()
            .any(|p| path.ancestors.iter().any(|k| *k == p.kind))
    {
        return false;
    }
    if pattern
        .not_within
        .iter()
        .any(|p| path.ancestors.iter().any(|k| *k == p.kind))
    {
        return false;
    }
    true
}

/// 宿主约束：`in_type` / `in_method` 都是「最近宿主必须满足」。
///
/// 若规则声明了 `in_type` 而当前节点根本不在任何类型内（例如字段初始化器之外的
/// 顶层语句在 Java 中不存在，但 import 节点确实如此），则视为不命中。
fn owner_predicates_match(pattern: &Pattern, path: &Path) -> bool {
    if let Some(tp) = &pattern.in_type {
        match path.owner_type {
            Some(ot) if type_predicate_matches(tp, &ot) => {}
            _ => return false,
        }
    }
    if let Some(mp) = &pattern.in_method {
        match path.owner_method {
            Some(om) if method_predicate_matches(mp, &om) => {}
            _ => return false,
        }
    }
    true
}

fn type_predicate_matches(tp: &TypePredicate, ot: &OwnerType) -> bool {
    if let Some(name) = &tp.name {
        if !value_matches(name, ot.name()) {
            return false;
        }
    }
    if !tp.annotations.is_empty()
        && !tp
            .annotations
            .iter()
            .any(|want| ot.annotations().iter().any(|a| annotation_name_matches(want, &a.name)))
    {
        return false;
    }
    if !tp.modifiers.is_empty()
        && !tp
            .modifiers
            .iter()
            .any(|want| ot.modifiers().iter().any(|m| value_matches_str(want, m)))
    {
        return false;
    }
    if let Some(want) = tp.is_interface {
        if ot.is_interface() != want {
            return false;
        }
    }
    true
}

fn method_predicate_matches(mp: &MethodPredicate, om: &OwnerMethod) -> bool {
    if let Some(name) = &mp.name {
        if !value_matches(name, om.name()) {
            return false;
        }
    }
    if let Some(rt) = &mp.return_type {
        match om.return_type() {
            Some(actual) if value_matches(rt, actual) => {}
            _ => return false,
        }
    }
    if !mp.annotations.is_empty()
        && !mp
            .annotations
            .iter()
            .any(|want| om.annotations().iter().any(|a| annotation_name_matches(want, &a.name)))
    {
        return false;
    }
    if !mp.modifiers.is_empty()
        && !mp
            .modifiers
            .iter()
            .any(|want| om.modifiers().iter().any(|m| value_matches_str(want, m)))
    {
        return false;
    }
    if !mp.parameter_types.is_empty() {
        let actual = om.parameter_types();
        if !mp
            .parameter_types
            .iter()
            .any(|want| actual.iter().any(|t| value_matches_str(want, t)))
        {
            return false;
        }
    }
    if !mp.parameter_names.is_empty() {
        let actual = om.parameter_names();
        if !mp
            .parameter_names
            .iter()
            .any(|want| actual.iter().any(|n| value_matches_str(want, n)))
        {
            return false;
        }
    }
    true
}

/// 注解名匹配。
///
/// 含通配符 / 正则元字符时走 [`value_matches`]；
/// 否则既匹配简单名（`Controller`），也匹配全限定名的尾段（`org.x.Controller`），
/// 这样规则作者不必为「用没用 import」而写两条。
fn annotation_name_matches(want: &str, actual: &str) -> bool {
    if want.contains('*') || want.starts_with('^') || want.ends_with('$') {
        return value_matches_str(want, actual);
    }
    actual == want || actual.ends_with(&format!(".{want}"))
}

// ── 字段匹配 ──

/// 按节点类型解析 `match_fields` / `match_members`；命中返回渲染上下文，否则 None。
fn match_fields_node(
    node: &MatchNode<'_>,
    pattern: &Pattern,
) -> Option<Vec<(&'static str, String)>> {
    match node {
        MatchNode::Import(imp) => match_fields_import(imp, &pattern.match_fields),
        MatchNode::Annotation(ann) => {
            match_fields_annotation(ann, &pattern.match_fields, &pattern.match_members)
        }
        MatchNode::Class(cd) => match_fields_class(cd, &pattern.match_fields),
        MatchNode::Method(md) => match_fields_method_decl(md, &pattern.match_fields),
        MatchNode::Field(fd) => match_fields_field_decl(fd, &pattern.match_fields),
        MatchNode::Expr(Expr::MethodCallExpr(mc)) => {
            match_fields_method_call(mc, &pattern.match_fields)
        }
        _ => None,
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

/// 匹配 Annotation 的字段与参数；命中返回上下文，否则 None。
///
/// `members` 语义：每个键都必须**出现在注解参数中**且取值匹配（AND）。
fn match_fields_annotation(
    ann: &Annotation,
    fields: &BTreeMap<String, MatchValue>,
    members: &BTreeMap<String, MatchValue>,
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
    for (key, expected) in members {
        // 参数不存在 → 不命中（`?` 直接把 None 透出）
        let actual = ann
            .members
            .iter()
            .find(|m| m.key == *key)
            .map(|m| m.value.as_str())?;
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push(("member", actual.to_string()));
    }
    Some(ctx)
}

/// 匹配 ClassDecl 的字段；命中返回上下文，否则 None。
fn match_fields_class(
    cd: &ClassDecl,
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
        let (actual, label) = match resolved {
            Some(r) => r,
            None => continue,
        };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
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
        let (actual, label) = match resolved {
            Some(r) => r,
            None => continue,
        };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
}

/// 匹配 FieldDecl 的字段；命中返回上下文，否则 None。
fn match_fields_field_decl(
    fd: &FieldDecl,
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
        let (actual, label) = match resolved {
            Some(r) => r,
            None => continue,
        };
        if !value_matches(expected, actual) {
            return None;
        }
        ctx.push((label, actual.to_string()));
    }
    Some(ctx)
}

// ── 辅助函数 ──

/// 判断 MatchValue 是否匹配给定文本（Single 精确/glob/正则，Any 任意一个命中）。
fn value_matches(v: &MatchValue, text: &str) -> bool {
    match v {
        MatchValue::Single(s) => regex_match(s, text),
        MatchValue::Any(list) => list.iter().any(|s| regex_match(s, text)),
    }
}

/// 便捷包装：对裸字符串走同一套「精确 / glob / 正则」判定。
fn value_matches_str(pattern: &str, text: &str) -> bool {
    regex_match(pattern, text)
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
    use crate::rule::{AncestorPredicate, PatternKind};
    use guard_core::rule::Severity;
    use java_ast::ast::*;

    /// 构造模式：`pat(PatternKind::MethodCall, &[("method", "println")])`
    fn pat(kind: PatternKind, fields: &[(&str, &str)]) -> Pattern {
        Pattern::new(kind).with_match_fields(
            fields
                .iter()
                .map(|(k, v)| (k.to_string(), MatchValue::Single(v.to_string()))),
        )
    }

    fn anc(kind: &str) -> AncestorPredicate {
        AncestorPredicate {
            kind: kind.to_string(),
        }
    }

    fn class(name: &str, members: Vec<MemberDecl>) -> TypeDecl {
        TypeDecl::ClassDeclaration(ClassDecl {
            name: name.to_string(),
            modifiers: vec!["public".to_string()],
            annotations: vec![],
            extends: None,
            implements: vec![],
            members,
            line: 1,
            end_line: 100,
        })
    }

    fn method(name: &str, line: usize, statements: Vec<Stmt>) -> MemberDecl {
        MemberDecl::MethodDeclaration(MethodDecl {
            name: name.to_string(),
            modifiers: vec!["public".to_string()],
            annotations: vec![],
            return_type: Some("void".to_string()),
            parameters: vec![],
            body: Some(BlockStmt {
                statements,
                line,
                end_line: line + 1,
            }),
            line,
            end_line: line + 1,
        })
    }

    /// 一次调用语句：`<callee>.<method>(...)`
    fn call(callee: Option<&str>, name: &str, line: usize) -> Stmt {
        Stmt::ExpressionStmt(ExprStmt {
            expr: Expr::MethodCallExpr(MethodCallExpr {
                callee: callee.map(|s| s.to_string()),
                method_name: name.to_string(),
                arguments: vec![],
                line,
            }),
            line,
        })
    }

    fn block(statements: Vec<Stmt>, line: usize, end_line: usize) -> BlockStmt {
        BlockStmt {
            statements,
            line,
            end_line,
        }
    }

    fn unit_of(types: Vec<TypeDecl>) -> CompilationUnit {
        CompilationUnit {
            package: Some("com.example".to_string()),
            imports: vec![],
            types,
            source_file: "T.java".to_string(),
            source_lines: vec![],
            source_text: String::new(),
            raw_json: String::new(),
        }
    }

    fn make_unit() -> CompilationUnit {
        unit_of(vec![class("badName", vec![method("doStuff", 4, vec![call(Some("System.out"), "println", 5)])])])
    }

    /// 覆盖多种上下文的编译单元：
    ///
    /// ```java
    /// @RestController
    /// public class C {                                   // 1
    ///     private int x = compute();                      // 2  字段初始化器
    ///     static { init(); }                              // 3  静态初始化块
    ///     public void run() {                             // 4
    ///         for (String s : list) {                     // 5
    ///             System.out.println(s);                  // 6  for-each 内
    ///         }
    ///         try {                                       // 7
    ///             risky();                                // 8  try 体内
    ///         } catch (Exception e) {                     // 9
    ///             handle();                               // 10 catch 体内
    ///         }
    ///         Runnable r = () -> lambdaCall();            // 11 lambda 体内
    ///     }
    /// }
    /// ```
    fn context_unit() -> CompilationUnit {
        let field = MemberDecl::FieldDeclaration(FieldDecl {
            name: "x".to_string(),
            modifiers: vec!["private".to_string()],
            annotations: vec![],
            field_type: Some("int".to_string()),
            initializer: Some(Expr::MethodCallExpr(MethodCallExpr {
                callee: None,
                method_name: "compute".to_string(),
                arguments: vec![],
                line: 2,
            })),
            line: 2,
        });

        let static_init = MemberDecl::InitializerDeclaration(InitializerDecl {
            is_static: true,
            body: block(vec![call(None, "init", 3)], 3, 3),
            line: 3,
        });

        let foreach = Stmt::ForEachStmt(ForEachStmt {
            variable: Expr::VariableDeclarationExpr(VarDeclStmt {
                var_type: Some("String".to_string()),
                declarations: vec![VarDeclarator {
                    name: "s".to_string(),
                    initializer: None,
                }],
                line: 5,
            }),
            iterable: Expr::NameExpr(NameExpr {
                name: "list".to_string(),
                line: 5,
            }),
            body: Box::new(Stmt::BlockStmt(block(
                vec![call(Some("System.out"), "println", 6)],
                5,
                7,
            ))),
            line: 5,
        });

        let try_stmt = Stmt::TryStmt(TryStmt {
            resources: vec![],
            try_body: block(vec![call(None, "risky", 8)], 7, 9),
            catch_clauses: vec![CatchClause {
                exception_type: Some("Exception".to_string()),
                exception_name: Some("e".to_string()),
                body: block(vec![call(None, "handle", 10)], 9, 11),
                line: 9,
            }],
            finally_body: None,
            line: 7,
        });

        let lambda_stmt = Stmt::VariableDeclarationStmt(VarDeclStmt {
            var_type: Some("Runnable".to_string()),
            declarations: vec![VarDeclarator {
                name: "r".to_string(),
                initializer: Some(Expr::LambdaExpr(LambdaExpr {
                    parameters: vec![],
                    body: Box::new(call(None, "lambdaCall", 11)),
                    line: 11,
                })),
            }],
            line: 11,
        });

        let mut cls = match class("C", vec![field, static_init, method("run", 4, vec![foreach, try_stmt, lambda_stmt])]) {
            TypeDecl::ClassDeclaration(c) => c,
            _ => unreachable!(),
        };
        cls.annotations = vec![Annotation {
            name: "RestController".to_string(),
            members: vec![],
            line: 1,
        }];

        unit_of(vec![TypeDecl::ClassDeclaration(cls)])
    }

    fn lines_of(vs: &[Violation]) -> Vec<usize> {
        let mut v: Vec<usize> = vs.iter().map(|x| x.line).collect();
        v.sort_unstable();
        v
    }

    // ── 基础匹配 ──

    #[test]
    fn match_method_call() {
        let unit = make_unit();
        let pattern = pat(PatternKind::MethodCall, &[("callee", "System.out"), ("method", "println")]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "call {callee}.{method} at {line}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 5);
        assert_eq!(vs[0].message, "call System.out.println at 5");
    }

    #[test]
    fn match_import_wildcard() {
        let unit = CompilationUnit {
            imports: vec![
                ImportDecl {
                    package: "java.util.*".to_string(),
                    is_wildcard: true,
                    is_static: false,
                    line: 1,
                },
                ImportDecl {
                    package: "java.util.List".to_string(),
                    is_wildcard: false,
                    is_static: false,
                    line: 2,
                },
            ],
            ..unit_of(vec![])
        };
        let pattern = pat(PatternKind::Import, &[("is_wildcard", "true")]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J003", Severity::Minor, "test");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn match_import_package_glob() {
        let unit = CompilationUnit {
            imports: vec![
                ImportDecl {
                    package: "java.util.*".to_string(),
                    is_wildcard: true,
                    is_static: false,
                    line: 1,
                },
                ImportDecl {
                    package: "java.util.List".to_string(),
                    is_wildcard: false,
                    is_static: false,
                    line: 2,
                },
            ],
            ..unit_of(vec![])
        };
        let pattern = pat(PatternKind::Import, &[("package", "java.util.*")]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J003", Severity::Minor, "import {package}");
        // glob `java.util.*` 同时命中 `java.util.*` 与 `java.util.List`
        assert_eq!(vs.len(), 2);
        let msgs: Vec<&str> = vs.iter().map(|v| v.message.as_str()).collect();
        assert!(msgs.contains(&"import java.util.*"), "got: {msgs:?}");
        assert!(msgs.contains(&"import java.util.List"), "got: {msgs:?}");
    }

    #[test]
    fn match_class_name_regex() {
        let unit = make_unit();
        let pattern = pat(PatternKind::ClassDeclaration, &[("name", "^[a-z]")]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J004", Severity::Minor, "test");
        assert_eq!(vs.len(), 1); // "badName" 以小写开头
    }

    #[test]
    fn match_class_decl_with_modifier() {
        let unit = make_unit();
        let pattern = pat(PatternKind::ClassDeclaration, &[("modifier", "public")]);
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
        let unit = make_unit();
        let pattern = pat(PatternKind::MethodDeclaration, &[("return_type", "void")]);
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
    fn match_field_declaration() {
        let unit = unit_of(vec![class(
            "C",
            vec![MemberDecl::FieldDeclaration(FieldDecl {
                name: "myField".to_string(),
                modifiers: vec![],
                annotations: vec![],
                field_type: Some("int".to_string()),
                initializer: None,
                line: 3,
            })],
        )]);
        let pattern = pat(PatternKind::FieldDeclaration, &[("field_type", "int")]);
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
    fn match_annotation_by_name() {
        let unit = context_unit();
        let pattern = pat(PatternKind::Annotation, &[("name", "RestController")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J009", Severity::Minor, "no {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].message, "no RestController");
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn match_value_any_of() {
        let unit = make_unit();
        let pattern = Pattern::new(PatternKind::MethodCall).with_match_fields([
            (
                "callee".to_string(),
                MatchValue::Any(vec!["System.out".to_string(), "System.err".to_string()]),
            ),
            (
                "method".to_string(),
                MatchValue::Any(vec![
                    "print".to_string(),
                    "println".to_string(),
                    "printf".to_string(),
                ]),
            ),
        ]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "no sysout");
        assert_eq!(vs.len(), 1);
    }

    #[test]
    fn match_any_of_no_match_yields_no_violation() {
        let unit = make_unit();
        let pattern = Pattern::new(PatternKind::MethodCall).with_match_fields([
            (
                "callee".to_string(),
                MatchValue::Any(vec!["System.err".to_string()]),
            ),
            (
                "method".to_string(),
                MatchValue::Single("println".to_string()),
            ),
        ]);
        let vs = match_pattern(&pattern, &unit, "Test.java", "J001", Severity::Minor, "x");
        assert_eq!(vs.len(), 0);
    }

    #[test]
    fn match_method_call_in_arguments() {
        // System.out.println(foo()) —— 嵌套调用应被递归检出，且只报一次
        let unit = unit_of(vec![class(
            "C",
            vec![method(
                "m",
                4,
                vec![Stmt::ExpressionStmt(ExprStmt {
                    expr: Expr::MethodCallExpr(MethodCallExpr {
                        callee: Some("System.out".to_string()),
                        method_name: "println".to_string(),
                        arguments: vec![Expr::MethodCallExpr(MethodCallExpr {
                            callee: None,
                            method_name: "foo".to_string(),
                            arguments: vec![],
                            line: 5,
                        })],
                        line: 5,
                    }),
                    line: 5,
                })],
            )],
        )]);
        let pattern = pat(PatternKind::MethodCall, &[("callee", "System.out"), ("method", "println")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 5);
    }

    #[test]
    fn match_method_decl_nested_class() {
        // 深层嵌套类里的方法也应被 MethodDeclaration 检出（递归一致）
        let inner = TypeDecl::ClassDeclaration(ClassDecl {
            name: "Inner".to_string(),
            modifiers: vec![],
            annotations: vec![],
            extends: None,
            implements: vec![],
            members: vec![method("BADNAME", 10, vec![])],
            line: 8,
            end_line: 12,
        });
        let unit = unit_of(vec![class("Outer", vec![MemberDecl::ClassDeclaration(
            match inner {
                TypeDecl::ClassDeclaration(c) => c,
                _ => unreachable!(),
            },
        )])]);
        let pattern = pat(PatternKind::MethodDeclaration, &[("name", "^[A-Z]+$")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J005", Severity::Minor, "bad method {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 10);
        assert_eq!(vs[0].message, "bad method BADNAME");
    }

    #[test]
    fn match_class_decl_nested_class() {
        // 嵌套类名称违反 PascalCase 也应被 ClassDeclaration 检出
        let unit = unit_of(vec![class(
            "Outer",
            vec![MemberDecl::ClassDeclaration(ClassDecl {
                name: "innerBad".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![],
                line: 8,
                end_line: 14,
            })],
        )]);
        let pattern = pat(PatternKind::ClassDeclaration, &[("name", "^[a-z]")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J004", Severity::Minor, "bad class {name}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 8);
        assert_eq!(vs[0].message, "bad class innerBad");
    }

    #[test]
    fn match_method_call_in_nested_class() {
        let unit = unit_of(vec![class(
            "Outer",
            vec![MemberDecl::ClassDeclaration(ClassDecl {
                name: "Inner".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![method("inner", 10, vec![call(Some("System.out"), "println", 11)])],
                line: 8,
                end_line: 14,
            })],
        )]);
        let pattern = pat(PatternKind::MethodCall, &[("callee", "System.out"), ("method", "println")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 11);
    }

    #[test]
    fn match_method_call_in_for_init_and_update() {
        // for (int i = next(); i < n; i = advance()) { } —— 初始化/更新子句中的调用应被检出
        let unit = unit_of(vec![class(
            "C",
            vec![method(
                "m",
                3,
                vec![Stmt::ForStmt(ForStmt {
                    initialization: Some(Expr::VariableDeclarationExpr(VarDeclStmt {
                        var_type: Some("int".to_string()),
                        declarations: vec![VarDeclarator {
                            name: "i".to_string(),
                            initializer: Some(Expr::MethodCallExpr(MethodCallExpr {
                                callee: None,
                                method_name: "next".to_string(),
                                arguments: vec![],
                                line: 4,
                            })),
                        }],
                        line: 4,
                    })),
                    condition: Some(Expr::BinaryExpr(BinaryExpr {
                        left: Box::new(Expr::NameExpr(NameExpr {
                            name: "i".to_string(),
                            line: 4,
                        })),
                        op: "<".to_string(),
                        right: Box::new(Expr::NameExpr(NameExpr {
                            name: "n".to_string(),
                            line: 4,
                        })),
                        line: 4,
                    })),
                    update: vec![Expr::AssignExpr(AssignExpr {
                        target: Box::new(Expr::NameExpr(NameExpr {
                            name: "i".to_string(),
                            line: 5,
                        })),
                        op: "=".to_string(),
                        value: Box::new(Expr::MethodCallExpr(MethodCallExpr {
                            callee: None,
                            method_name: "advance".to_string(),
                            arguments: vec![],
                            line: 5,
                        })),
                        line: 5,
                    })],
                    body: Box::new(Stmt::BlockStmt(block(vec![], 5, 5))),
                    line: 4,
                })],
            )],
        )]);
        let pattern = Pattern::new(PatternKind::MethodCall).with_match_fields([(
            "method".to_string(),
            MatchValue::Any(vec!["next".to_string(), "advance".to_string()]),
        )]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 2, "for 初始化与更新子句中的调用都应检出, got: {vs:?}");
        assert_eq!(lines_of(&vs), vec![4, 5]);
    }

    // ── 遍历覆盖面（早期实现的盲区回归）──

    #[test]
    fn call_in_field_initializer_is_detected() {
        // 早期实现不遍历字段初始化器，这里的 compute() 会被漏掉
        let unit = context_unit();
        let pattern = pat(PatternKind::MethodCall, &[("method", "compute")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![2], "字段初始化器中的调用应被检出");
    }

    #[test]
    fn call_in_static_initializer_is_detected() {
        let unit = context_unit();
        let pattern = pat(PatternKind::MethodCall, &[("method", "init")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![3], "静态初始化块中的调用应被检出");
    }

    #[test]
    fn call_in_lambda_body_is_detected() {
        let unit = context_unit();
        let pattern = pat(PatternKind::MethodCall, &[("method", "lambdaCall")]);
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![11], "lambda 体内的调用应被检出");
    }

    // ── 上下文谓词 ──

    #[test]
    fn within_for_foreach_limits_to_loop_body() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "println")]);
        pattern.within = vec![anc("ForEachStmt")];
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![6]);
    }

    #[test]
    fn within_try_covers_try_and_catch_scope() {
        let unit = context_unit();
        let mut pattern = pat(
            PatternKind::MethodCall,
            &[("method", "risky")],
        );
        pattern.within = vec![anc("TryStmt")];
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![8]);
    }

    #[test]
    fn within_catch_clause_isolates_catch_body() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.within = vec![anc("CatchClause")];
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![10], "只有 catch 体内的调用应命中");
    }

    #[test]
    fn within_is_any_of_not_all_of() {
        // within 列表是 any_of：列出多个容器表示「在任一容器内」
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.within = vec![anc("ForEachStmt"), anc("CatchClause")];
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![6, 10]);
    }

    #[test]
    fn not_within_excludes_loop_body() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.not_within = vec![anc("ForEachStmt"), anc("TryStmt")];
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        // 剩下：字段初始化器(2)、静态块(3)、lambda(11)
        assert_eq!(lines_of(&vs), vec![2, 3, 11]);
    }

    #[test]
    fn in_type_annotation_constrains_scope() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_type = Some(TypePredicate {
            annotations: vec!["RestController".to_string()],
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        // 全部调用都在 @RestController 类内
        assert_eq!(lines_of(&vs), vec![2, 3, 6, 8, 10, 11]);
    }

    #[test]
    fn in_type_annotation_mismatch_yields_nothing() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_type = Some(TypePredicate {
            annotations: vec!["Controller".to_string()],
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        // 简单名精确匹配：RestController ≠ Controller
        assert!(vs.is_empty(), "got: {vs:?}");
    }

    #[test]
    fn in_type_is_interface_filter() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_type = Some(TypePredicate {
            is_interface: Some(true),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert!(vs.is_empty(), "C 是类而非接口，不应命中");

        pattern.in_type = Some(TypePredicate {
            is_interface: Some(false),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 6);
    }

    #[test]
    fn in_type_name_pattern_constrains_scope() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_type = Some(TypePredicate {
            name: Some(MatchValue::Single("^C$".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 6);

        pattern.in_type = Some(TypePredicate {
            name: Some(MatchValue::Single("^Other$".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert!(vs.is_empty());
    }

    #[test]
    fn in_method_constrains_to_method_body() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_method = Some(MethodPredicate {
            name: Some(MatchValue::Single("run".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        // run() 体内：for-each(6) / try(8) / catch(10) / lambda(11)
        // 字段初始化器(2) 与静态块(3) 不属于任何方法，应被排除
        assert_eq!(lines_of(&vs), vec![6, 8, 10, 11]);
    }

    #[test]
    fn in_method_return_type_constrains() {
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.in_method = Some(MethodPredicate {
            return_type: Some(MatchValue::Single("void".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(vs.len(), 4);

        pattern.in_method = Some(MethodPredicate {
            return_type: Some(MatchValue::Single("String".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert!(vs.is_empty());
    }

    #[test]
    fn context_predicates_combine_with_and() {
        // within 与 in_method 之间是 AND
        let unit = context_unit();
        let mut pattern = pat(PatternKind::MethodCall, &[("method", "*")]);
        pattern.within = vec![anc("TryStmt")];
        pattern.in_method = Some(MethodPredicate {
            name: Some(MatchValue::Single("run".to_string())),
            ..Default::default()
        });
        let vs = match_pattern(&pattern, &unit, "T.java", "J001", Severity::Minor, "call");
        assert_eq!(lines_of(&vs), vec![8, 10]);
    }

    // ── value_matches 单测（精确 / glob / 正则 / any_of）──

    #[test]
    fn value_matches_exact() {
        assert!(value_matches(
            &MatchValue::Single("System.out".to_string()),
            "System.out"
        ));
        assert!(!value_matches(
            &MatchValue::Single("System.out".to_string()),
            "System.err"
        ));
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

    #[test]
    fn annotation_name_matches_simple_and_qualified() {
        assert!(annotation_name_matches("Controller", "Controller"));
        assert!(annotation_name_matches("Controller", "org.springframework.stereotype.Controller"));
        assert!(!annotation_name_matches("Controller", "RestController"));
        // 含通配符时退回 value_matches
        assert!(annotation_name_matches("*Controller", "RestController"));
    }

    // ── 注解参数匹配（match_members）──

    fn ann_unit() -> CompilationUnit {
        let mut cls = match class("Ann", vec![]) {
            TypeDecl::ClassDeclaration(c) => c,
            _ => unreachable!(),
        };
        cls.annotations = vec![
            Annotation {
                name: "ComponentScan".to_string(),
                members: vec![AnnotationMember {
                    key: "basePackages".to_string(),
                    value: "{\"com.a\", \"com.b\"}".to_string(),
                }],
                line: 1,
            },
            Annotation {
                name: "Deprecated".to_string(),
                members: vec![],
                line: 2,
            },
        ];
        unit_of(vec![TypeDecl::ClassDeclaration(cls)])
    }

    fn ann_pattern(members: &[(&str, &str)]) -> Pattern {
        let mut p = pat(PatternKind::Annotation, &[("name", "ComponentScan")]);
        p.match_members = members
            .iter()
            .map(|(k, v)| (k.to_string(), MatchValue::Single(v.to_string())))
            .collect();
        p
    }

    #[test]
    fn annotation_member_present_matches() {
        let unit = ann_unit();
        let p = ann_pattern(&[("basePackages", "*")]);
        let vs = match_pattern(&p, &unit, "T.java", "J700", Severity::Major, "{name} -> {member}");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 1);
        assert_eq!(vs[0].message, "ComponentScan -> {\"com.a\", \"com.b\"}");
    }

    #[test]
    fn annotation_member_absent_does_not_match() {
        let unit = ann_unit();
        // 注解存在、但参数名不同 → 不命中
        let p = ann_pattern(&[("scanBasePackages", "*")]);
        assert!(match_pattern(&p, &unit, "T.java", "J700", Severity::Major, "x").is_empty());
    }

    #[test]
    fn annotation_member_value_must_match() {
        let unit = ann_unit();
        assert!(
            match_pattern(&ann_pattern(&[("basePackages", "*com.a*")]), &unit, "T.java", "J700", Severity::Major, "x").len() == 1
        );
        assert!(
            match_pattern(&ann_pattern(&[("basePackages", "*com.z*")]), &unit, "T.java", "J700", Severity::Major, "x").is_empty()
        );
    }

    #[test]
    fn annotation_members_are_and_combined() {
        let unit = ann_unit();
        // 两个键都必须存在，basePackages 在、another 不在 → 不命中
        let p = ann_pattern(&[("basePackages", "*"), ("another", "*")]);
        assert!(match_pattern(&p, &unit, "T.java", "J700", Severity::Major, "x").is_empty());
    }

    /// 没写 `match_members` 时，注解规则的行为与改造前一致（只看 name）。
    #[test]
    fn annotation_without_members_keeps_old_behaviour() {
        let unit = ann_unit();
        let p = pat(PatternKind::Annotation, &[("name", "Deprecated")]);
        let vs = match_pattern(&p, &unit, "T.java", "J700", Severity::Minor, "deprecated");
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].line, 2);
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
        assert_eq!(render_message("{callee} {unknown}", &ctx), "X {unknown}");
    }
}

