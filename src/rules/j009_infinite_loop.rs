//! 内置规则 J009：检测确定/可能死循环的循环。
//!
//! 判定分三层，由强到弱：
//!
//! 1. **确定死循环（definite）**：循环条件在编译期即可确定为 `true`
//!    （字面量 `true`、`!(false)`、或经常量传播后的等效真，如 `while (T)` 其中
//!    `T` 为 `final boolean T = true`；或 `for (int i = 0; i < 10; )` 这种常量边界），
//!    且条件所引用的变量在循环体内**不被修改**（条件值跨迭代不变），
//!    且循环体内无可达的退出路径（针对本循环的 `break`、或方法级 `return`/`throw`）→
//!    报告。
//!
//! 2. **#1 for 计数器不推进**：`for (int i = 0; i < n; )` 这类循环**没有更新表达式**，
//!    且条件依赖的循环变量在循环体内**从未被修改**，且无退出路径 → 计数器永不推进，
//!    若条件在入口为真则死循环 → 报告（potential）。
//!
//! 3. **#3 条件变量从不更新**：`while (flag) { ... }` / `do { ... } while (flag)`，
//!    条件仅依赖单个变量，该变量在循环体内**从未被修改**，既非已知常量也未被重赋值，
//!    且无退出路径 → 可能是事件循环误用或死循环 → 报告（potential）。
//!
//! 设计边界（重要）：
//! - 不试图「证明」一般终止性（停机问题的近亲，不可判定），只拦截明显的高危模式。
//! - 非恒定真、且无法静态判定入口值的普通条件（如 `while (flag)` 中 flag 可能为外部
//!   字段）一律保守不报，避免对合法事件循环误报；仅当能确认条件变量在循环内不变且无
//!   退出时才提示 #3。
//! - 循环体存在 `if (cond) break;` 这类「条件性退出」视为可能存在退出路径，不报。
//! - 嵌套循环的 `break` 只退出最内层；带标签 `break` 因 AST 未保留循环标签，保守视为
//!   可能退出，不报。
//! - `break`/`return` 出现在 lambda 体内时不视为外层循环的退出（作用域限制），但
//!   lambda 内抛异常仍会传播出循环，故 `throw` 始终视为退出。

use std::collections::{HashMap, HashSet};

use guard_core::rule::{Rule, RuleId, Severity, SpanPolicy, Violation};
use java_ast::ast::*;

pub struct InfiniteLoopRule {
    id: RuleId,
}

impl InfiniteLoopRule {
    pub fn new() -> Self {
        Self {
            id: RuleId("J009".to_string()),
        }
    }
}

impl Default for InfiniteLoopRule {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule<CompilationUnit> for InfiniteLoopRule {
    fn id(&self) -> &RuleId {
        &self.id
    }

    fn description(&self) -> &str {
        "Loop that may never terminate (definite/potential infinite loop)"
    }

    fn severity(&self) -> Severity {
        Severity::Major
    }

    /// 死循环的「成因」可能位于循环体内任意位置（如删除更新表达式），
    /// 增量扫描时按区间相交判定，避免漏报「锚点行未变、但体内变更引入死循环」。
    fn span_policy(&self) -> SpanPolicy {
        SpanPolicy::Intersect
    }

    fn check_unit(&self, unit: &CompilationUnit) -> Vec<Violation> {
        let mut out = Vec::new();
        for td in &unit.types {
            visit_type_decl(td, &unit.source_file, &mut out);
        }
        out
    }
}

/// 常量上下文：来自字段（final 布尔常量）与方法内有效终态（effectively-final）局部布尔常量。
#[derive(Clone, Default)]
struct ConstCtx {
    /// 名称 -> 已知布尔常量值（true/false）。
    bools: HashMap<String, bool>,
    /// 方法体内被重赋值的名称（含字段名），用于排除「非有效终态」的局部变量。
    reassigned: HashSet<String>,
}

// ---------------------------------------------------------------------------
// AST 遍历：找出所有循环节点并逐一检查
// ---------------------------------------------------------------------------

fn type_members(td: &TypeDecl) -> &[MemberDecl] {
    match td {
        TypeDecl::ClassDeclaration(c) => &c.members,
        TypeDecl::InterfaceDeclaration(i) => &i.members,
        TypeDecl::EnumDeclaration(e) => &e.members,
        TypeDecl::AnnotationDeclaration(a) => &a.members,
    }
}

/// 收集当前类型中 `final boolean X = true/false;` 形式的字段常量。
fn build_field_consts(td: &TypeDecl) -> HashMap<String, bool> {
    let mut m = HashMap::new();
    for mem in type_members(td) {
        if let MemberDecl::FieldDeclaration(fd) = mem {
            if fd.modifiers.iter().any(|x| x == "final") {
                if let Some(init) = &fd.initializer {
                    if let Some(b) = literal_bool(init) {
                        m.insert(fd.name.clone(), b);
                    }
                }
            }
        }
    }
    m
}

fn visit_type_decl(td: &TypeDecl, file: &str, out: &mut Vec<Violation>) {
    let base = ConstCtx {
        bools: build_field_consts(td),
        reassigned: HashSet::new(),
    };
    for m in type_members(td) {
        visit_member(m, file, out, &base);
    }
}

fn visit_member(m: &MemberDecl, file: &str, out: &mut Vec<Violation>, base: &ConstCtx) {
    match m {
        MemberDecl::MethodDeclaration(md) => {
            let mut ctx = base.clone();
            if let Some(body) = &md.body {
                collect_method_facts(Some(body), &mut ctx);
                visit_block(body, file, out, &ctx);
            }
        }
        MemberDecl::ConstructorDeclaration(cd) => {
            let mut ctx = base.clone();
            if let Some(body) = &cd.body {
                collect_method_facts(Some(body), &mut ctx);
                visit_block(body, file, out, &ctx);
            }
        }
        MemberDecl::FieldDeclaration(fd) => {
            if let Some(init) = &fd.initializer {
                visit_expr(init, file, out);
            }
        }
        MemberDecl::InitializerDeclaration(init) => {
            let mut ctx = base.clone();
            collect_block_facts(&init.body, &mut ctx);
            visit_block(&init.body, file, out, &ctx);
        }
        MemberDecl::ClassDeclaration(c) => {
            visit_type_decl(&TypeDecl::ClassDeclaration(c.clone()), file, out)
        }
        MemberDecl::InterfaceDeclaration(i) => {
            visit_type_decl(&TypeDecl::InterfaceDeclaration(i.clone()), file, out)
        }
        MemberDecl::EnumDeclaration(e) => {
            visit_type_decl(&TypeDecl::EnumDeclaration(e.clone()), file, out)
        }
        MemberDecl::AnnotationDeclaration(a) => {
            visit_type_decl(&TypeDecl::AnnotationDeclaration(a.clone()), file, out)
        }
    }
}

/// 收集方法体中的有效终态布尔局部常量与重赋值集合，写入 ctx。
fn collect_method_facts(body: Option<&BlockStmt>, ctx: &mut ConstCtx) {
    let mut bools: Vec<(String, bool)> = Vec::new();
    let mut re: HashSet<String> = HashSet::new();
    if let Some(b) = body {
        collect_facts_block(b, &mut bools, &mut re);
    }
    // 仅把「未被重赋值」的局部布尔量当作常量，避免 `boolean r = true; r = false;` 误判。
    for (n, v) in bools {
        if !re.contains(&n) {
            ctx.bools.insert(n, v);
        }
    }
    ctx.reassigned.extend(re);
}

fn collect_block_facts(b: &BlockStmt, ctx: &mut ConstCtx) {
    let mut bools: Vec<(String, bool)> = Vec::new();
    let mut re: HashSet<String> = HashSet::new();
    collect_facts_block(b, &mut bools, &mut re);
    for (n, v) in bools {
        if !re.contains(&n) {
            ctx.bools.insert(n, v);
        }
    }
    ctx.reassigned.extend(re);
}

/// 遍历语句块，收集局部布尔字面量声明与重赋值名称（供 #2 常量传播使用）。
fn collect_facts_block(b: &BlockStmt, bools: &mut Vec<(String, bool)>, re: &mut HashSet<String>) {
    for s in &b.statements {
        collect_facts_stmt(s, bools, re);
    }
}

/// 遍历单条语句，递归收集局部常量声明与重赋值。
fn collect_facts_stmt(s: &Stmt, bools: &mut Vec<(String, bool)>, re: &mut HashSet<String>) {
    match s {
        Stmt::BlockStmt(b) => collect_facts_block(b, bools, re),
        Stmt::VariableDeclarationStmt(vs) => {
            for d in &vs.declarations {
                if let Some(init) = &d.initializer {
                    if let Some(b) = literal_bool(init) {
                        bools.push((d.name.clone(), b));
                    }
                }
            }
        }
        Stmt::ExpressionStmt(es) => collect_facts_expr(&es.expr, bools, re),
        Stmt::IfStmt(is) => {
            collect_facts_stmt(&is.then_stmt, bools, re);
            if let Some(e) = &is.else_stmt {
                collect_facts_stmt(e, bools, re);
            }
            collect_facts_expr(&is.condition, bools, re);
        }
        Stmt::ForStmt(fs) => {
            collect_facts_stmt(&fs.body, bools, re);
            if let Some(i) = &fs.initialization {
                collect_facts_expr(i, bools, re);
            }
            if let Some(c) = &fs.condition {
                collect_facts_expr(c, bools, re);
            }
            for u in &fs.update {
                collect_facts_expr(u, bools, re);
            }
        }
        Stmt::ForEachStmt(fe) => {
            collect_facts_stmt(&fe.body, bools, re);
            collect_facts_expr(&fe.iterable, bools, re);
        }
        Stmt::WhileStmt(w) => {
            collect_facts_stmt(&w.body, bools, re);
            collect_facts_expr(&w.condition, bools, re);
        }
        Stmt::DoStmt(d) => {
            collect_facts_stmt(&d.body, bools, re);
            collect_facts_expr(&d.condition, bools, re);
        }
        Stmt::TryStmt(ts) => {
            collect_facts_block(&ts.try_body, bools, re);
            for cc in &ts.catch_clauses {
                collect_facts_block(&cc.body, bools, re);
            }
            if let Some(f) = &ts.finally_body {
                collect_facts_block(f, bools, re);
            }
        }
        Stmt::SwitchStmt(ss) => {
            collect_facts_expr(&ss.selector, bools, re);
            for c in &ss.cases {
                for s in &c.statements {
                    collect_facts_stmt(s, bools, re);
                }
            }
        }
        Stmt::SynchronizedStmt(sy) => {
            collect_facts_expr(&sy.expr, bools, re);
            collect_facts_block(&sy.body, bools, re);
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(e) = &rs.expr {
                collect_facts_expr(e, bools, re);
            }
        }
        Stmt::ThrowStmt(ts) => collect_facts_expr(&ts.expr, bools, re),
        _ => {}
    }
}

/// 遍历表达式，记录被重赋值（含自增/自减）的变量名。
fn collect_facts_expr(e: &Expr, bools: &mut Vec<(String, bool)>, re: &mut HashSet<String>) {
    match e {
        Expr::AssignExpr(ae) => {
            record_assign_target(&ae.target, re);
            collect_facts_expr(&ae.value, bools, re);
        }
        Expr::UnaryExpr(ue) => {
            if ue.op == "++" || ue.op == "--" {
                record_assign_target(&ue.expr, re);
            }
            collect_facts_expr(&ue.expr, bools, re);
        }
        Expr::BinaryExpr(be) => {
            collect_facts_expr(&be.left, bools, re);
            collect_facts_expr(&be.right, bools, re);
        }
        Expr::MethodCallExpr(mc) => {
            for a in &mc.arguments {
                collect_facts_expr(a, bools, re);
            }
        }
        Expr::FieldAccessExpr(fa) => collect_facts_expr(&fa.target, bools, re),
        Expr::CastExpr(ce) => collect_facts_expr(&ce.expr, bools, re),
        Expr::ConditionalExpr(ce) => {
            collect_facts_expr(&ce.condition, bools, re);
            collect_facts_expr(&ce.then_expr, bools, re);
            collect_facts_expr(&ce.else_expr, bools, re);
        }
        Expr::ArrayAccessExpr(aa) => {
            collect_facts_expr(&aa.array, bools, re);
            collect_facts_expr(&aa.index, bools, re);
        }
        Expr::ArrayCreationExpr(ac) => {
            for i in &ac.initializer {
                collect_facts_expr(i, bools, re);
            }
        }
        Expr::ObjectCreationExpr(oc) => {
            for a in &oc.arguments {
                collect_facts_expr(a, bools, re);
            }
        }
        Expr::InstanceOfExpr(io) => collect_facts_expr(&io.expr, bools, re),
        Expr::LambdaExpr(le) => collect_facts_stmt(&le.body, bools, re),
        Expr::EnclosedExpr { inner, .. } => collect_facts_expr(inner, bools, re),
        _ => {}
    }
}

fn visit_stmt(stmt: &Stmt, file: &str, out: &mut Vec<Violation>, ctx: &ConstCtx) {
    match stmt {
        Stmt::ForStmt(fs) => {
            check_loop(file, out, LoopRef::For(fs), ctx);
            visit_stmt(&fs.body, file, out, ctx);
            if let Some(init) = &fs.initialization {
                visit_expr(init, file, out);
            }
            if let Some(cond) = &fs.condition {
                visit_expr(cond, file, out);
            }
            for u in &fs.update {
                visit_expr(u, file, out);
            }
        }
        Stmt::ForEachStmt(fe) => {
            // for-each 本身不因条件恒定而死循环（迭代器耗尽即退出），只递归检查体与迭代表达式。
            visit_expr(&fe.iterable, file, out);
            visit_stmt(&fe.body, file, out, ctx);
        }
        Stmt::WhileStmt(ws) => {
            check_loop(file, out, LoopRef::While(ws), ctx);
            visit_stmt(&ws.body, file, out, ctx);
            visit_expr(&ws.condition, file, out);
        }
        Stmt::DoStmt(ds) => {
            check_loop(file, out, LoopRef::Do(ds), ctx);
            visit_stmt(&ds.body, file, out, ctx);
            visit_expr(&ds.condition, file, out);
        }
        Stmt::BlockStmt(b) => {
            for s in &b.statements {
                visit_stmt(s, file, out, ctx);
            }
        }
        Stmt::IfStmt(is) => {
            visit_stmt(&is.then_stmt, file, out, ctx);
            if let Some(e) = &is.else_stmt {
                visit_stmt(e, file, out, ctx);
            }
            visit_expr(&is.condition, file, out);
        }
        Stmt::TryStmt(ts) => {
            visit_block(&ts.try_body, file, out, ctx);
            for cc in &ts.catch_clauses {
                visit_block(&cc.body, file, out, ctx);
            }
            if let Some(f) = &ts.finally_body {
                visit_block(f, file, out, ctx);
            }
        }
        Stmt::SwitchStmt(ss) => {
            visit_expr(&ss.selector, file, out);
            for c in &ss.cases {
                for s in &c.statements {
                    visit_stmt(s, file, out, ctx);
                }
            }
        }
        Stmt::SynchronizedStmt(sy) => {
            visit_expr(&sy.expr, file, out);
            visit_block(&sy.body, file, out, ctx);
        }
        Stmt::ExpressionStmt(es) => {
            visit_expr(&es.expr, file, out);
        }
        Stmt::VariableDeclarationStmt(vs) => {
            for d in &vs.declarations {
                if let Some(init) = &d.initializer {
                    visit_expr(init, file, out);
                }
            }
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(e) = &rs.expr {
                visit_expr(e, file, out);
            }
        }
        Stmt::ThrowStmt(ts) => {
            visit_expr(&ts.expr, file, out);
        }
        // BreakStmt / ContinueStmt / EmptyStmt / UnknownStmt：无需要递归的子语句/表达式
        _ => {}
    }
}

fn visit_block(b: &BlockStmt, file: &str, out: &mut Vec<Violation>, ctx: &ConstCtx) {
    for s in &b.statements {
        visit_stmt(s, file, out, ctx);
    }
}

fn visit_expr(expr: &Expr, file: &str, out: &mut Vec<Violation>) {
    match expr {
        Expr::MethodCallExpr(mc) => {
            for a in &mc.arguments {
                visit_expr(a, file, out);
            }
        }
        Expr::FieldAccessExpr(fa) => visit_expr(&fa.target, file, out),
        Expr::BinaryExpr(be) => {
            visit_expr(&be.left, file, out);
            visit_expr(&be.right, file, out);
        }
        Expr::UnaryExpr(ue) => visit_expr(&ue.expr, file, out),
        Expr::AssignExpr(ae) => {
            visit_expr(&ae.target, file, out);
            visit_expr(&ae.value, file, out);
        }
        Expr::CastExpr(ce) => visit_expr(&ce.expr, file, out),
        Expr::ConditionalExpr(ce) => {
            visit_expr(&ce.condition, file, out);
            visit_expr(&ce.then_expr, file, out);
            visit_expr(&ce.else_expr, file, out);
        }
        Expr::ArrayAccessExpr(aa) => {
            visit_expr(&aa.array, file, out);
            visit_expr(&aa.index, file, out);
        }
        Expr::ArrayCreationExpr(ac) => {
            for i in &ac.initializer {
                visit_expr(i, file, out);
            }
        }
        Expr::ObjectCreationExpr(oc) => {
            for a in &oc.arguments {
                visit_expr(a, file, out);
            }
        }
        Expr::InstanceOfExpr(io) => visit_expr(&io.expr, file, out),
        Expr::LambdaExpr(le) => {
            // lambda 体是独立作用域，用空 ctx 递归（不继承外层局部常量，避免误判）
            visit_stmt(&le.body, file, out, &ConstCtx::default());
        }
        Expr::VariableDeclarationExpr(vde) => {
            for d in &vde.declarations {
                if let Some(init) = &d.initializer {
                    visit_expr(init, file, out);
                }
            }
        }
        Expr::EnclosedExpr { inner, .. } => visit_expr(inner, file, out),
        Expr::NameExpr(_)
        | Expr::LiteralExpr(_)
        | Expr::ThisExpr(_)
        | Expr::SuperExpr(_)
        | Expr::MethodReferenceExpr(_)
        | Expr::UnknownExpr { .. } => {}
    }
}

// ---------------------------------------------------------------------------
// 循环检查核心
// ---------------------------------------------------------------------------

enum LoopRef<'a> {
    For(&'a ForStmt),
    While(&'a WhileStmt),
    Do(&'a DoStmt),
}

fn check_loop(file: &str, out: &mut Vec<Violation>, loop_ref: LoopRef, ctx: &ConstCtx) {
    let (cond, body, line, is_for, update, init): (Option<&Expr>, &Stmt, usize, bool, &[Expr], &Option<Expr>) =
        match loop_ref {
            LoopRef::For(f) => (
                f.condition.as_ref(),
                &f.body,
                f.line,
                true,
                &f.update,
                &f.initialization,
            ),
            LoopRef::While(w) => (Some(&w.condition), &w.body, w.line, false, &[], &None),
            LoopRef::Do(d) => (Some(&d.condition), &d.body, d.line, false, &[], &None),
        };

    let init_vars = extract_init_vars(init);
    let init_ints = extract_init_ints(init);
    let cond_val: Option<bool> = match cond {
        None => Some(true), // for (;;) 无 condition 视作恒定真
        Some(c) => eval_expr_bool(c, ctx, &init_ints),
    };
    let cond_names: HashSet<String> = match cond {
        None => HashSet::new(),
        Some(c) => names_in_expr(c),
    };

    let mut body_mut = mutated_in_stmt(body);
    for u in update {
        body_mut.extend(mutated_in_expr(u));
    }
    let invariant = cond_names.is_disjoint(&body_mut);
    let has_exit = loop_has_exit_stmt(body, 0, false);
    // 循环区间结束行（供增量扫描 intersect 策略使用）；无块体时退化为锚点行判定
    let end_line = match body {
        Stmt::BlockStmt(b) => Some(b.end_line),
        _ => None,
    };

    // 1) 确定死循环：条件恒定真 + 条件变量在循环内不变 + 无退出
    if cond_val == Some(true) && invariant && !has_exit {
        push_loop_violation(
            out,
            file,
            line,
            end_line,
            "循环条件在编译期即可确定为 true（字面量或常量传播），且条件所用变量在循环体内不被修改，且无 break/return/throw 退出：该循环将永不终止（死循环）",
        );
        return;
    }
    // 已存在退出路径则其余启发式不再适用
    if has_exit {
        return;
    }

    // 2) #1：for 缺少更新表达式，且条件依赖的循环变量在循环体内未被修改
    if is_for && update.is_empty() {
        let dependent = init_vars.iter().any(|v| cond_names.contains(v));
        // 检查条件中出现的**所有**变量（含 init 之外的变量）：若 `n` 在循环体内被修改，
        // `for (int i = 0; i < n; ) { n--; }` 仍可终止，不应误报。
        let none_mutated = cond_names.iter().all(|v| !body_mut.contains(v));
        if dependent && none_mutated {
            let vars = init_vars.join(", ");
            let msg = format!(
                "for 循环缺少更新表达式（update），循环变量 [{vars}] 在条件中使用且条件涉及的所有变量在循环体内均未被修改，且无 break/return/throw 退出：计数器不会推进，可能死循环"
            );
            push_loop_violation(out, file, line, end_line, msg);
            return;
        }
    }

    // 2b) #1b：for 有 update 但更新无效（i = i / i = 0 / i += 0）
    if is_for && !update.is_empty() && cond.is_some() && cond_val != Some(false) {
        // 涉及条件变量的更新条目（其它变量的更新不影响循环条件）。
        let cond_updates: Vec<&Expr> = update
            .iter()
            .filter(|u| update_target(u).is_some_and(|v| cond_names.contains(v.as_str())))
            .collect();
        // 仅当「涉及条件变量的所有更新」都无效时才报：
        // - `i = 0, i++` 中 i++ 仍能让循环推进，不能因为 i = 0 而误报；
        // - `for (int i = 10; i > 0; i = 0)` 中 i = 0 使条件立即为 false，循环正常终止。
        if !cond_updates.is_empty()
            && cond_updates
                .iter()
                .all(|u| is_ineffective_update(u, &init_ints))
        {
            let vars = cond_updates
                .iter()
                .filter_map(|u| update_target(u))
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let msg = format!(
                "for 循环的更新表达式对循环变量 [{vars}] 无实际效果（如 i = i / i = 0 / i += 0），条件永远不会因更新而变为 false，且无 break/return/throw 退出：可能死循环"
            );
            push_loop_violation(out, file, line, end_line, msg);
            return;
        }
    }

    // 2c) #1c：for update 方向与条件矛盾（i < N 但 i-- / i > 0 但 i++）
    if is_for && !update.is_empty() {
        if let Some(cond_expr) = cond {
            if let Some(violation) = check_update_direction_conflict(cond_expr, update, &init_ints) {
                push_loop_violation(out, file, line, end_line, violation);
                return;
            }
        }
    }

    // 3) #3：while/do-while 条件为单个变量，循环体内从不修改、且非已知常量/非重赋值
    if !is_for {
        if let Some(Expr::NameExpr(n)) = cond {
            let name = &n.name;
            if !ctx.bools.contains_key(name)
                && !ctx.reassigned.contains(name)
                && !body_mut.contains(name)
            {
                let msg = format!(
                    "while/do-while 的循环条件仅依赖变量 `{name}`，该变量在循环体内从未被修改且无 break/return/throw 退出：可能为事件循环误用或死循环（请确认存在外部修改或退出机制）"
                );
                push_loop_violation(out, file, line, end_line, msg);
            }
        }
    }
}

/// 推送一条 J009 违规，附循环区间结束行（供增量扫描 intersect 策略使用）。
fn push_loop_violation(
    out: &mut Vec<Violation>,
    file: &str,
    line: usize,
    end_line: Option<usize>,
    msg: impl Into<String>,
) {
    let mut v = Violation::new("J009", Severity::Major, file, line, msg);
    v.end_line = end_line;
    out.push(v);
}

// ---------------------------------------------------------------------------
// 常量传播 / 入口条件求值
// ---------------------------------------------------------------------------

/// 字面量布尔值（`true`/`false` 字符串）。
fn literal_bool(e: &Expr) -> Option<bool> {
    if let Expr::LiteralExpr(l) = e {
        if l.value == "true" {
            return Some(true);
        } else if l.value == "false" {
            return Some(false);
        }
    }
    None
}

/// 在入口处对条件表达式求值（带常量替换与 for 初始化整型绑定）。
fn eval_expr_bool(e: &Expr, ctx: &ConstCtx, init_ints: &HashMap<String, i64>) -> Option<bool> {
    match e {
        Expr::LiteralExpr(l) => {
            if l.value == "true" {
                Some(true)
            } else if l.value == "false" {
                Some(false)
            } else {
                None
            }
        }
        Expr::NameExpr(n) => ctx.bools.get(&n.name).copied(),
        Expr::FieldAccessExpr(fa) => {
            // 仅处理 `this.field` 形式的字段常量读取
            if matches!(fa.target.as_ref(), Expr::ThisExpr(_)) {
                ctx.bools.get(&fa.field).copied()
            } else {
                None
            }
        }
        Expr::EnclosedExpr { inner, .. } => eval_expr_bool(inner, ctx, init_ints),
        Expr::UnaryExpr(u) if u.op == "!" => eval_expr_bool(&u.expr, ctx, init_ints).map(|b| !b),
        Expr::BinaryExpr(be) => match be.op.as_str() {
            "&&" => {
                let l = eval_expr_bool(&be.left, ctx, init_ints)?;
                let r = eval_expr_bool(&be.right, ctx, init_ints)?;
                Some(l && r)
            }
            "||" => {
                let l = eval_expr_bool(&be.left, ctx, init_ints)?;
                let r = eval_expr_bool(&be.right, ctx, init_ints)?;
                Some(l || r)
            }
            _ => {
                // 先尝试整型比较（如 `i < 10`）
                if let (Some(l), Some(r)) = (eval_int(&be.left, init_ints), eval_int(&be.right, init_ints))
                {
                    Some(compare_op(be.op.as_str(), l, r))
                } else {
                    match be.op.as_str() {
                        "==" => {
                            let l = eval_expr_bool(&be.left, ctx, init_ints);
                            let r = eval_expr_bool(&be.right, ctx, init_ints);
                            match (l, r) {
                                (Some(a), Some(b)) => Some(a == b),
                                _ => None,
                            }
                        }
                        "!=" => {
                            let l = eval_expr_bool(&be.left, ctx, init_ints);
                            let r = eval_expr_bool(&be.right, ctx, init_ints);
                            match (l, r) {
                                (Some(a), Some(b)) => Some(a != b),
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                }
            }
        },
        _ => None,
    }
}

fn compare_op(op: &str, l: i64, r: i64) -> bool {
    match op {
        "<" => l < r,
        ">" => l > r,
        "<=" => l <= r,
        ">=" => l >= r,
        "==" => l == r,
        "!=" => l != r,
        _ => false,
    }
}

/// 在入口处对整型表达式求值（用于 for 初始化常量边界，如 `i < 10`）。
fn eval_int(e: &Expr, init_ints: &HashMap<String, i64>) -> Option<i64> {
    match e {
        Expr::LiteralExpr(l) => l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok(),
        Expr::NameExpr(n) => init_ints.get(&n.name).copied(),
        Expr::EnclosedExpr { inner, .. } => eval_int(inner, init_ints),
        Expr::UnaryExpr(u) if u.op == "-" => eval_int(&u.expr, init_ints).map(|v| -v),
        Expr::BinaryExpr(be) => {
            let l = eval_int(&be.left, init_ints)?;
            let r = eval_int(&be.right, init_ints)?;
            match be.op.as_str() {
                "+" => Some(l + r),
                "-" => Some(l - r),
                "*" => Some(l * r),
                "/" => {
                    if r == 0 {
                        None
                    } else {
                        Some(l / r)
                    }
                }
                "%" => {
                    if r == 0 {
                        None
                    } else {
                        Some(l % r)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 名称 / 修改集合收集
// ---------------------------------------------------------------------------

fn names_in_expr(e: &Expr) -> HashSet<String> {
    let mut s = HashSet::new();
    collect_names(e, &mut s);
    s
}

fn collect_names(e: &Expr, s: &mut HashSet<String>) {
    match e {
        Expr::NameExpr(n) => {
            s.insert(n.name.clone());
        }
        Expr::FieldAccessExpr(fa) => {
            s.insert(fa.field.clone());
            collect_names(&fa.target, s);
        }
        Expr::MethodCallExpr(mc) => {
            for a in &mc.arguments {
                collect_names(a, s);
            }
        }
        Expr::BinaryExpr(be) => {
            collect_names(&be.left, s);
            collect_names(&be.right, s);
        }
        Expr::UnaryExpr(ue) => collect_names(&ue.expr, s),
        Expr::AssignExpr(ae) => {
            collect_names(&ae.target, s);
            collect_names(&ae.value, s);
        }
        Expr::CastExpr(ce) => collect_names(&ce.expr, s),
        Expr::ConditionalExpr(ce) => {
            collect_names(&ce.condition, s);
            collect_names(&ce.then_expr, s);
            collect_names(&ce.else_expr, s);
        }
        Expr::ArrayAccessExpr(aa) => {
            collect_names(&aa.array, s);
            collect_names(&aa.index, s);
        }
        Expr::ArrayCreationExpr(ac) => {
            for i in &ac.initializer {
                collect_names(i, s);
            }
        }
        Expr::ObjectCreationExpr(oc) => {
            for a in &oc.arguments {
                collect_names(a, s);
            }
        }
        Expr::InstanceOfExpr(io) => collect_names(&io.expr, s),
        Expr::LambdaExpr(le) => collect_names_stmt(&le.body, s),
        Expr::VariableDeclarationExpr(vde) => {
            for d in &vde.declarations {
                if let Some(i) = &d.initializer {
                    collect_names(i, s);
                }
            }
        }
        Expr::EnclosedExpr { inner, .. } => collect_names(inner, s),
        _ => {}
    }
}

fn collect_names_stmt(s: &Stmt, set: &mut HashSet<String>) {
    match s {
        Stmt::BlockStmt(b) => {
            for x in &b.statements {
                collect_names_stmt(x, set);
            }
        }
        Stmt::IfStmt(is) => {
            collect_names_stmt(&is.then_stmt, set);
            if let Some(e) = &is.else_stmt {
                collect_names_stmt(e, set);
            }
            collect_names(&is.condition, set);
        }
        Stmt::ForStmt(fs) => {
            collect_names_stmt(&fs.body, set);
            if let Some(c) = &fs.condition {
                collect_names(c, set);
            }
            if let Some(i) = &fs.initialization {
                collect_names(i, set);
            }
            for u in &fs.update {
                collect_names(u, set);
            }
        }
        Stmt::ForEachStmt(fe) => {
            collect_names_stmt(&fe.body, set);
            collect_names(&fe.iterable, set);
        }
        Stmt::WhileStmt(w) => {
            collect_names_stmt(&w.body, set);
            collect_names(&w.condition, set);
        }
        Stmt::DoStmt(d) => {
            collect_names_stmt(&d.body, set);
            collect_names(&d.condition, set);
        }
        Stmt::TryStmt(ts) => {
            collect_names_block(&ts.try_body, set);
            for cc in &ts.catch_clauses {
                collect_names_block(&cc.body, set);
            }
            if let Some(f) = &ts.finally_body {
                collect_names_block(f, set);
            }
        }
        Stmt::SwitchStmt(ss) => {
            collect_names(&ss.selector, set);
            for c in &ss.cases {
                for x in &c.statements {
                    collect_names_stmt(x, set);
                }
            }
        }
        Stmt::SynchronizedStmt(sy) => {
            collect_names(&sy.expr, set);
            collect_names_block(&sy.body, set);
        }
        Stmt::ExpressionStmt(es) => collect_names(&es.expr, set),
        Stmt::VariableDeclarationStmt(vs) => {
            for d in &vs.declarations {
                if let Some(i) = &d.initializer {
                    collect_names(i, set);
                }
            }
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(e) = &rs.expr {
                collect_names(e, set);
            }
        }
        Stmt::ThrowStmt(ts) => collect_names(&ts.expr, set),
        _ => {}
    }
}

fn collect_names_block(b: &BlockStmt, set: &mut HashSet<String>) {
    for s in &b.statements {
        collect_names_stmt(s, set);
    }
}

fn mutated_in_stmt(s: &Stmt) -> HashSet<String> {
    let mut set = HashSet::new();
    collect_mutated_stmt(s, &mut set);
    set
}

fn mutated_in_expr(e: &Expr) -> HashSet<String> {
    let mut set = HashSet::new();
    collect_mutated_expr(e, &mut set);
    set
}

fn collect_mutated_stmt(s: &Stmt, set: &mut HashSet<String>) {
    match s {
        Stmt::BlockStmt(b) => {
            for x in &b.statements {
                collect_mutated_stmt(x, set);
            }
        }
        Stmt::IfStmt(is) => {
            collect_mutated_stmt(&is.then_stmt, set);
            if let Some(e) = &is.else_stmt {
                collect_mutated_stmt(e, set);
            }
            collect_mutated_expr(&is.condition, set);
        }
        Stmt::ForStmt(fs) => {
            collect_mutated_stmt(&fs.body, set);
            if let Some(c) = &fs.condition {
                collect_mutated_expr(c, set);
            }
            if let Some(i) = &fs.initialization {
                collect_mutated_expr(i, set);
            }
            for u in &fs.update {
                collect_mutated_expr(u, set);
            }
        }
        Stmt::ForEachStmt(fe) => {
            collect_mutated_stmt(&fe.body, set);
            collect_mutated_expr(&fe.iterable, set);
        }
        Stmt::WhileStmt(w) => {
            collect_mutated_stmt(&w.body, set);
            collect_mutated_expr(&w.condition, set);
        }
        Stmt::DoStmt(d) => {
            collect_mutated_stmt(&d.body, set);
            collect_mutated_expr(&d.condition, set);
        }
        Stmt::TryStmt(ts) => {
            collect_mutated_block(&ts.try_body, set);
            for cc in &ts.catch_clauses {
                collect_mutated_block(&cc.body, set);
            }
            if let Some(f) = &ts.finally_body {
                collect_mutated_block(f, set);
            }
        }
        Stmt::SwitchStmt(ss) => {
            collect_mutated_expr(&ss.selector, set);
            for c in &ss.cases {
                for x in &c.statements {
                    collect_mutated_stmt(x, set);
                }
            }
        }
        Stmt::SynchronizedStmt(sy) => {
            collect_mutated_expr(&sy.expr, set);
            collect_mutated_block(&sy.body, set);
        }
        Stmt::ExpressionStmt(es) => collect_mutated_expr(&es.expr, set),
        Stmt::VariableDeclarationStmt(vs) => {
            for d in &vs.declarations {
                if let Some(i) = &d.initializer {
                    collect_mutated_expr(i, set);
                }
            }
        }
        Stmt::ReturnStmt(rs) => {
            if let Some(e) = &rs.expr {
                collect_mutated_expr(e, set);
            }
        }
        Stmt::ThrowStmt(ts) => collect_mutated_expr(&ts.expr, set),
        _ => {}
    }
}

fn collect_mutated_block(b: &BlockStmt, set: &mut HashSet<String>) {
    for s in &b.statements {
        collect_mutated_stmt(s, set);
    }
}

fn collect_mutated_expr(e: &Expr, set: &mut HashSet<String>) {
    match e {
        Expr::MethodCallExpr(mc) => {
            for a in &mc.arguments {
                collect_mutated_expr(a, set);
            }
        }
        Expr::FieldAccessExpr(fa) => collect_mutated_expr(&fa.target, set),
        Expr::BinaryExpr(be) => {
            collect_mutated_expr(&be.left, set);
            collect_mutated_expr(&be.right, set);
        }
        Expr::UnaryExpr(ue) => {
            if ue.op == "++" || ue.op == "--" {
                record_assign_target(&ue.expr, set);
            }
            collect_mutated_expr(&ue.expr, set);
        }
        Expr::AssignExpr(ae) => {
            record_assign_target(&ae.target, set);
            collect_mutated_expr(&ae.target, set);
            collect_mutated_expr(&ae.value, set);
        }
        Expr::CastExpr(ce) => collect_mutated_expr(&ce.expr, set),
        Expr::ConditionalExpr(ce) => {
            collect_mutated_expr(&ce.condition, set);
            collect_mutated_expr(&ce.then_expr, set);
            collect_mutated_expr(&ce.else_expr, set);
        }
        Expr::ArrayAccessExpr(aa) => {
            collect_mutated_expr(&aa.array, set);
            collect_mutated_expr(&aa.index, set);
        }
        Expr::ArrayCreationExpr(ac) => {
            for i in &ac.initializer {
                collect_mutated_expr(i, set);
            }
        }
        Expr::ObjectCreationExpr(oc) => {
            for a in &oc.arguments {
                collect_mutated_expr(a, set);
            }
        }
        Expr::InstanceOfExpr(io) => collect_mutated_expr(&io.expr, set),
        Expr::LambdaExpr(le) => collect_mutated_stmt(&le.body, set),
        Expr::VariableDeclarationExpr(vde) => {
            for d in &vde.declarations {
                if let Some(i) = &d.initializer {
                    collect_mutated_expr(i, set);
                }
            }
        }
        Expr::EnclosedExpr { inner, .. } => collect_mutated_expr(inner, set),
        _ => {}
    }
}

fn record_assign_target(e: &Expr, set: &mut HashSet<String>) {
    match e {
        Expr::NameExpr(n) => {
            set.insert(n.name.clone());
        }
        Expr::FieldAccessExpr(fa) => {
            set.insert(fa.field.clone());
        }
        _ => {}
    }
}

fn extract_init_vars(init: &Option<Expr>) -> Vec<String> {
    if let Some(Expr::VariableDeclarationExpr(vde)) = init {
        vde.declarations.iter().map(|d| d.name.clone()).collect()
    } else {
        Vec::new()
    }
}

fn extract_init_ints(init: &Option<Expr>) -> HashMap<String, i64> {
    let mut m = HashMap::new();
    if let Some(Expr::VariableDeclarationExpr(vde)) = init {
        let empty: HashMap<String, i64> = HashMap::new();
        for d in &vde.declarations {
            if let Some(init_e) = &d.initializer {
                if let Some(i) = eval_int(init_e, &empty) {
                    m.insert(d.name.clone(), i);
                }
            }
        }
    }
    m
}

// ---------------------------------------------------------------------------
// 无效 update 检测
// ---------------------------------------------------------------------------

/// 判断 update 表达式是否对变量无实际效果。
/// 检测模式：`i = i` / `i = <初始值>`（重置回初始值）/ `i += 0` / `i -= 0` / `i *= 1` / `i /= 1`。
/// `i = <const>` 仅在 const 等于循环变量初始值时才是无效更新：
/// `for (int i = 0; i < 10; i = 0)` 重置回 0 无进展；而 `for (int i = 10; i > 0; i = 0)`
/// 使条件立即为 false，循环正常终止，不能算无效。
fn is_ineffective_update(u: &Expr, init_ints: &HashMap<String, i64>) -> bool {
    match u {
        // i = i  (自赋值)
        Expr::AssignExpr(ae) if ae.op == "=" => {
            if let (Expr::NameExpr(target), Expr::NameExpr(value)) =
                (ae.target.as_ref(), ae.value.as_ref())
            {
                return target.name == value.name;
            }
            // i = <const>：仅当重置为初始值才无效
            if let (Expr::NameExpr(target), Expr::LiteralExpr(l)) =
                (ae.target.as_ref(), ae.value.as_ref())
            {
                if let Ok(n) = l.value.trim_end_matches(['L', 'l']).parse::<i64>() {
                    return init_ints.get(&target.name).copied() == Some(n);
                }
            }
            false
        }
        // i += 0 / i -= 0
        Expr::AssignExpr(ae) if ae.op == "+=" || ae.op == "-=" => {
            if let Expr::LiteralExpr(l) = ae.value.as_ref() {
                if let Ok(n) = l.value.trim_end_matches(['L', 'l']).parse::<i64>() {
                    return n == 0;
                }
            }
            false
        }
        // i *= 1 / i /= 1
        Expr::AssignExpr(ae) if ae.op == "*=" || ae.op == "/=" => {
            if let Expr::LiteralExpr(l) = ae.value.as_ref() {
                if let Ok(n) = l.value.trim_end_matches(['L', 'l']).parse::<i64>() {
                    return n == 1;
                }
            }
            false
        }
        // i++ / i-- 不会无效
        _ => false,
    }
}

/// 返回 update 表达式修改的变量名（赋值或自增/自减），非变量更新返回 None。
fn update_target(u: &Expr) -> Option<&String> {
    match u {
        Expr::AssignExpr(ae) => {
            if let Expr::NameExpr(n) = ae.target.as_ref() {
                Some(&n.name)
            } else {
                None
            }
        }
        Expr::UnaryExpr(ue) if ue.op == "++" || ue.op == "--" => {
            if let Expr::NameExpr(n) = ue.expr.as_ref() {
                Some(&n.name)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// update 方向与条件矛盾检测
// ---------------------------------------------------------------------------

/// 检测 for 循环的 update 方向是否与条件矛盾。
/// 例如：`for (int i = 0; i < 10; i--)` — i 递减但条件是 i < 10
///       `for (int i = 10; i > 0; i++)` — i 递增但条件是 i > 0
fn check_update_direction_conflict(
    cond: &Expr,
    update: &[Expr],
    init_ints: &HashMap<String, i64>,
) -> Option<String> {
    // 只处理简单的二元比较条件：var < N / var > N / var <= N / var >= N
    if let Expr::BinaryExpr(be) = cond {
        let (var_name, op, bound_val) = match (be.left.as_ref(), be.right.as_ref()) {
            (Expr::NameExpr(n), Expr::LiteralExpr(l)) => {
                let v = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                (n.name.clone(), be.op.as_str(), v)
            }
            (Expr::LiteralExpr(l), Expr::NameExpr(n)) => {
                let v = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                // 翻转操作符：5 > i 等价于 i < 5
                let flipped = match be.op.as_str() {
                    "<" => ">",
                    ">" => "<",
                    "<=" => ">=",
                    ">=" => "<=",
                    other => other,
                };
                (n.name.clone(), flipped, v)
            }
            _ => return None,
        };

        let init_val = init_ints.get(&var_name)?;

        // 找到对该变量的 update 表达式；无关变量的更新（如 `j++`）直接跳过，
        // 不能提前中止整个检查（否则 `for (int i = 0; i < 10; j++, i--)` 会漏报）。
        for u in update {
            let Some((delta, _is_relevant)) = analyze_update_delta(u, &var_name) else {
                continue;
            };

            // 判断方向是否矛盾
            let enters_loop = match op {
                "<" | "<=" => *init_val < bound_val || (*init_val == bound_val && op == "<="),
                ">" | ">=" => *init_val > bound_val || (*init_val == bound_val && op == ">="),
                _ => continue,
            };

            if !enters_loop {
                continue; // 初始就不进循环，不算死循环
            }

            let moving_away = match (op, delta) {
                ("<" | "<=", d) if d < 0 => true,   // 条件需要 i 增加，但 i 在减小
                (">" | ">=", d) if d > 0 => true,   // 条件需要 i 减小，但 i 在增大
                _ => false,
            };

            if moving_away {
                return Some(format!(
                    "for 循环更新方向与条件矛盾：循环变量 `{var_name}` 初始值为 {init_val}，条件为 `{var_name} {op} {bound_val}`，但更新表达式使变量朝远离条件的方向变化（delta={delta}），条件永远不会变为 false：死循环"
                ));
            }
        }
    }
    None
}

/// 分析 update 表达式对变量的数值变化量。
/// 返回 `(delta, true)`；若该表达式不是对此变量的更新，返回 None。
fn analyze_update_delta(u: &Expr, var: &str) -> Option<(i64, bool)> {
    match u {
        // i++ → delta=1
        Expr::UnaryExpr(ue) if ue.op == "++" => {
            if let Expr::NameExpr(n) = ue.expr.as_ref() {
                if n.name == var {
                    return Some((1, true));
                }
            }
            None
        }
        // i-- → delta=-1
        Expr::UnaryExpr(ue) if ue.op == "--" => {
            if let Expr::NameExpr(n) = ue.expr.as_ref() {
                if n.name == var {
                    return Some((-1, true));
                }
            }
            None
        }
        // i += N → delta=N
        Expr::AssignExpr(ae) if ae.op == "+=" => {
            if let (Expr::NameExpr(n), Expr::LiteralExpr(l)) =
                (ae.target.as_ref(), ae.value.as_ref())
            {
                if n.name == var {
                    let d = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                    return Some((d, true));
                }
            }
            None
        }
        // i -= N → delta=-N
        Expr::AssignExpr(ae) if ae.op == "-=" => {
            if let (Expr::NameExpr(n), Expr::LiteralExpr(l)) =
                (ae.target.as_ref(), ae.value.as_ref())
            {
                if n.name == var {
                    let d = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                    return Some((-d, true));
                }
            }
            None
        }
        // i = i + N → delta=N
        Expr::AssignExpr(ae) if ae.op == "=" => {
            if let (Expr::NameExpr(n), Expr::BinaryExpr(be)) =
                (ae.target.as_ref(), ae.value.as_ref())
            {
                if n.name == var && be.op == "+" {
                    // i + N 或 N + i
                    if let Expr::LiteralExpr(l) = be.right.as_ref() {
                        if let Expr::NameExpr(n2) = be.left.as_ref() {
                            if n2.name == var {
                                let d = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                                return Some((d, true));
                            }
                        }
                    }
                    if let Expr::LiteralExpr(l) = be.left.as_ref() {
                        if let Expr::NameExpr(n2) = be.right.as_ref() {
                            if n2.name == var {
                                let d = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                                return Some((d, true));
                            }
                        }
                    }
                }
                // i = i - N → delta=-N
                if n.name == var && be.op == "-" {
                    if let (Expr::NameExpr(n2), Expr::LiteralExpr(l)) =
                        (be.left.as_ref(), be.right.as_ref())
                    {
                        if n2.name == var {
                            let d = l.value.trim_end_matches(['L', 'l']).parse::<i64>().ok()?;
                            return Some((-d, true));
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 退出路径判定
// ---------------------------------------------------------------------------

/// 循环体是否存在「退出本循环」的可达路径。
fn loop_has_exit_stmt(stmt: &Stmt, rel: usize, in_lambda: bool) -> bool {
    match stmt {
        Stmt::BreakStmt(b) => {
            if in_lambda {
                false
            } else if b.label.is_some() {
                true
            } else {
                rel == 0
            }
        }
        Stmt::ReturnStmt(_) => !in_lambda,
        Stmt::ThrowStmt(_) => true,
        Stmt::ForStmt(f) => {
            loop_has_exit_stmt(&f.body, rel + 1, in_lambda)
                || f.condition
                    .as_ref()
                    .map_or(false, |c| loop_has_exit_expr(c, rel + 1, in_lambda))
                || f.initialization
                    .as_ref()
                    .map_or(false, |c| loop_has_exit_expr(c, rel + 1, in_lambda))
                || f.update.iter().any(|u| loop_has_exit_expr(u, rel + 1, in_lambda))
        }
        Stmt::ForEachStmt(fe) => {
            // for-each 自身是循环：体内的 break 只退出 for-each（rel+1）；
            // return / throw 仍会传播出方法（与 ForStmt 处理一致）。
            loop_has_exit_stmt(&fe.body, rel + 1, in_lambda)
                || loop_has_exit_expr(&fe.iterable, rel + 1, in_lambda)
        }
        Stmt::WhileStmt(w) => {
            loop_has_exit_stmt(&w.body, rel + 1, in_lambda)
                || loop_has_exit_expr(&w.condition, rel + 1, in_lambda)
        }
        Stmt::DoStmt(d) => {
            loop_has_exit_stmt(&d.body, rel + 1, in_lambda)
                || loop_has_exit_expr(&d.condition, rel + 1, in_lambda)
        }
        Stmt::BlockStmt(b) => b
            .statements
            .iter()
            .any(|s| loop_has_exit_stmt(s, rel, in_lambda)),
        Stmt::IfStmt(is) => {
            loop_has_exit_stmt(&is.then_stmt, rel, in_lambda)
                || is.else_stmt
                    .as_ref()
                    .map_or(false, |e| loop_has_exit_stmt(e, rel, in_lambda))
                || loop_has_exit_expr(&is.condition, rel, in_lambda)
        }
        Stmt::TryStmt(ts) => {
            loop_has_exit_block(&ts.try_body, rel, in_lambda)
                || ts.catch_clauses
                    .iter()
                    .any(|cc| loop_has_exit_block(&cc.body, rel, in_lambda))
                || ts.finally_body
                    .as_ref()
                    .map_or(false, |f| loop_has_exit_block(f, rel, in_lambda))
        }
        Stmt::SwitchStmt(ss) => {
            loop_has_exit_expr(&ss.selector, rel, in_lambda)
                || ss.cases
                    .iter()
                    .any(|c| c.statements.iter().any(|s| loop_has_exit_stmt(s, rel, in_lambda)))
        }
        Stmt::SynchronizedStmt(sy) => {
            loop_has_exit_expr(&sy.expr, rel, in_lambda)
                || loop_has_exit_block(&sy.body, rel, in_lambda)
        }
        Stmt::ExpressionStmt(es) => loop_has_exit_expr(&es.expr, rel, in_lambda),
        Stmt::VariableDeclarationStmt(vs) => vs
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().map_or(false, |i| loop_has_exit_expr(i, rel, in_lambda))),
        Stmt::ContinueStmt(_) => false,
        _ => false,
    }
}

fn loop_has_exit_block(b: &BlockStmt, rel: usize, in_lambda: bool) -> bool {
    b.statements
        .iter()
        .any(|s| loop_has_exit_stmt(s, rel, in_lambda))
}

fn loop_has_exit_expr(expr: &Expr, rel: usize, in_lambda: bool) -> bool {
    match expr {
        Expr::MethodCallExpr(mc) => mc.arguments.iter().any(|a| loop_has_exit_expr(a, rel, in_lambda)),
        Expr::FieldAccessExpr(fa) => loop_has_exit_expr(&fa.target, rel, in_lambda),
        Expr::BinaryExpr(be) => {
            loop_has_exit_expr(&be.left, rel, in_lambda) || loop_has_exit_expr(&be.right, rel, in_lambda)
        }
        Expr::UnaryExpr(ue) => loop_has_exit_expr(&ue.expr, rel, in_lambda),
        Expr::AssignExpr(ae) => {
            loop_has_exit_expr(&ae.target, rel, in_lambda) || loop_has_exit_expr(&ae.value, rel, in_lambda)
        }
        Expr::CastExpr(ce) => loop_has_exit_expr(&ce.expr, rel, in_lambda),
        Expr::ConditionalExpr(ce) => {
            loop_has_exit_expr(&ce.condition, rel, in_lambda)
                || loop_has_exit_expr(&ce.then_expr, rel, in_lambda)
                || loop_has_exit_expr(&ce.else_expr, rel, in_lambda)
        }
        Expr::ArrayAccessExpr(aa) => {
            loop_has_exit_expr(&aa.array, rel, in_lambda) || loop_has_exit_expr(&aa.index, rel, in_lambda)
        }
        Expr::ArrayCreationExpr(ac) => {
            ac.initializer.iter().any(|i| loop_has_exit_expr(i, rel, in_lambda))
        }
        Expr::ObjectCreationExpr(oc) => {
            oc.arguments.iter().any(|a| loop_has_exit_expr(a, rel, in_lambda))
        }
        Expr::InstanceOfExpr(io) => loop_has_exit_expr(&io.expr, rel, in_lambda),
        Expr::LambdaExpr(le) => loop_has_exit_stmt(&le.body, rel, true),
        Expr::VariableDeclarationExpr(vde) => vde
            .declarations
            .iter()
            .any(|d| d.initializer.as_ref().map_or(false, |i| loop_has_exit_expr(i, rel, in_lambda))),
        Expr::EnclosedExpr { inner, .. } => loop_has_exit_expr(inner, rel, in_lambda),
        Expr::NameExpr(_)
        | Expr::LiteralExpr(_)
        | Expr::ThisExpr(_)
        | Expr::SuperExpr(_)
        | Expr::MethodReferenceExpr(_)
        | Expr::UnknownExpr { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> String {
        "Test.java".to_string()
    }

    fn true_expr() -> Expr {
        Expr::LiteralExpr(LiteralExpr {
            value: "true".to_string(),
            literal_type: Some("boolean".to_string()),
            line: 1,
        })
    }

    fn empty_block() -> Stmt {
        Stmt::BlockStmt(BlockStmt {
            statements: vec![],
            line: 2,
            end_line: 3,
        })
    }

    fn while_true(body: Stmt) -> Stmt {
        Stmt::WhileStmt(WhileStmt {
            condition: true_expr(),
            body: Box::new(body),
            line: 1,
        })
    }

    fn while_stmt(cond: Expr, body: Stmt) -> Stmt {
        Stmt::WhileStmt(WhileStmt {
            condition: cond,
            body: Box::new(body),
            line: 1,
        })
    }

    fn do_while_stmt(cond: Expr, body: Stmt) -> Stmt {
        Stmt::DoStmt(DoStmt {
            body: Box::new(body),
            condition: cond,
            line: 1,
        })
    }

    fn name(n: &str) -> Expr {
        Expr::NameExpr(NameExpr {
            name: n.to_string(),
            line: 1,
        })
    }

    fn int_lit(v: &str) -> Expr {
        Expr::LiteralExpr(LiteralExpr {
            value: v.to_string(),
            literal_type: Some("int".to_string()),
            line: 1,
        })
    }

    fn bin(op: &str, l: Expr, r: Expr) -> Expr {
        Expr::BinaryExpr(BinaryExpr {
            left: Box::new(l),
            op: op.to_string(),
            right: Box::new(r),
            line: 1,
        })
    }

    fn method_call(m: &str) -> Expr {
        Expr::MethodCallExpr(MethodCallExpr {
            callee: None,
            method_name: m.to_string(),
            arguments: vec![],
            line: 1,
        })
    }

    /// 局部布尔声明语句（带任意初始化表达式）。
    fn local_bool_stmt(var: &str, init: Expr) -> Stmt {
        Stmt::VariableDeclarationStmt(VarDeclStmt {
            var_type: Some("boolean".to_string()),
            declarations: vec![VarDeclarator {
                name: var.to_string(),
                initializer: Some(init),
            }],
            line: 1,
        })
    }

    fn unary_inc(n: &str) -> Expr {
        Expr::UnaryExpr(UnaryExpr {
            expr: Box::new(name(n)),
            op: "++".to_string(),
            prefix: false,
            line: 1,
        })
    }

    fn unary_dec(n: &str) -> Expr {
        Expr::UnaryExpr(UnaryExpr {
            expr: Box::new(name(n)),
            op: "--".to_string(),
            prefix: false,
            line: 1,
        })
    }

    fn assign_update(n: &str, op: &str, val: Expr) -> Expr {
        Expr::AssignExpr(AssignExpr {
            target: Box::new(name(n)),
            op: op.to_string(),
            value: Box::new(val),
            line: 1,
        })
    }

    fn assign_stmt(n: &str, val: Expr) -> Stmt {
        Stmt::ExpressionStmt(ExprStmt {
            expr: Expr::AssignExpr(AssignExpr {
                target: Box::new(name(n)),
                op: "=".to_string(),
                value: Box::new(val),
                line: 1,
            }),
            line: 1,
        })
    }

    fn for_loop(init: Option<Expr>, cond: Option<Expr>, update: Vec<Expr>, body: Stmt) -> Stmt {
        Stmt::ForStmt(ForStmt {
            initialization: init,
            condition: cond,
            update,
            body: Box::new(body),
            line: 1,
        })
    }

    fn int_var_init(var: &str, val: &str) -> Expr {
        Expr::VariableDeclarationExpr(VarDeclStmt {
            var_type: Some("int".to_string()),
            declarations: vec![VarDeclarator {
                name: var.to_string(),
                initializer: Some(int_lit(val)),
            }],
            line: 1,
        })
    }

    fn body_with(stmts: Vec<Stmt>) -> Stmt {
        Stmt::BlockStmt(BlockStmt {
            statements: stmts,
            line: 2,
            end_line: 5,
        })
    }

    fn unit_with(stmts: Vec<Stmt>) -> CompilationUnit {
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
                    name: "run".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    return_type: Some("void".to_string()),
                    parameters: vec![],
                    body: Some(BlockStmt {
                        statements: stmts,
                        line: 1,
                        end_line: 10,
                    }),
                    line: 1,
                    end_line: 10,
                })],
                line: 1,
                end_line: 10,
            })],
            source_file: file(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    fn unit_with_field(field: FieldDecl, stmts: Vec<Stmt>) -> CompilationUnit {
        CompilationUnit {
            package: None,
            imports: vec![],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Test".to_string(),
                modifiers: vec![],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![
                    MemberDecl::FieldDeclaration(field),
                    MemberDecl::MethodDeclaration(MethodDecl {
                        name: "run".to_string(),
                        modifiers: vec![],
                        annotations: vec![],
                        return_type: Some("void".to_string()),
                        parameters: vec![],
                        body: Some(BlockStmt {
                            statements: stmts,
                            line: 1,
                            end_line: 10,
                        }),
                        line: 1,
                        end_line: 10,
                    }),
                ],
                line: 1,
                end_line: 10,
            })],
            source_file: file(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    fn final_bool_field(name: &str, val: &str) -> FieldDecl {
        FieldDecl {
            name: name.to_string(),
            modifiers: vec!["final".to_string()],
            annotations: vec![],
            field_type: Some("boolean".to_string()),
            initializer: Some(Expr::LiteralExpr(LiteralExpr {
                value: val.to_string(),
                literal_type: Some("boolean".to_string()),
                line: 1,
            })),
            line: 1,
        }
    }

    // ---- 原 definite 死循环用例 ----

    #[test]
    fn detects_bare_while_true() {
        let unit = unit_with(vec![while_true(empty_block())]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "应检测到裸 while(true)");
        assert_eq!(vs[0].rule_id.0, "J009");
        assert_eq!(vs[0].line, 1);
    }

    #[test]
    fn detects_for_empty_condition() {
        // for (;;) 无 condition -> 恒定真
        let f = for_loop(None, None, vec![], empty_block());
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "应检测到 for(;;)");
    }

    #[test]
    fn detects_do_while_true() {
        let d = do_while_stmt(true_expr(), empty_block());
        let unit = unit_with(vec![d]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "应检测到 do-while(true)");
    }

    #[test]
    fn ignores_while_true_with_break() {
        let body = body_with(vec![Stmt::BreakStmt(BreakStmt { label: None, line: 2 })]);
        let unit = unit_with(vec![while_true(body)]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "while(true) 含 break 不会死循环");
    }

    #[test]
    fn ignores_non_constant_condition() {
        // 方法调用条件（非变量、非字面量）不触发任何分支
        let w = while_stmt(method_call("hasNext"), empty_block());
        let unit = unit_with(vec![w]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "非常量且非变量条件不报");
    }

    #[test]
    fn ignores_conditional_break() {
        let body = body_with(vec![Stmt::IfStmt(IfStmt {
            condition: name("x"),
            then_stmt: Box::new(Stmt::BreakStmt(BreakStmt { label: None, line: 3 })),
            else_stmt: None,
            line: 2,
        })]);
        let unit = unit_with(vec![while_true(body)]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "条件性 break 视为可能退出，不报");
    }

    #[test]
    fn detects_nested_infinite() {
        let inner = Stmt::WhileStmt(WhileStmt {
            condition: true_expr(),
            body: Box::new(empty_block()),
            line: 2,
        });
        let outer_body = body_with(vec![inner]);
        let unit = unit_with(vec![while_true(outer_body)]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 2, "内层与外层 while(true) 均被判死循环");
        assert!(vs.iter().any(|v| v.line == 1), "外层循环（line 1）应被报告");
    }

    #[test]
    fn lambda_return_does_not_mask_outer_loop() {
        let lambda = Expr::LambdaExpr(LambdaExpr {
            parameters: vec![],
            body: Box::new(body_with(vec![Stmt::ReturnStmt(ReturnStmt { expr: None, line: 2 })])),
            line: 2,
        });
        let es = Stmt::ExpressionStmt(ExprStmt {
            expr: lambda,
            line: 2,
        });
        let unit = unit_with(vec![while_true(es)]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "lambda 内 return 不退出外层 while(true)");
    }

    // ---- #1：for 计数器不推进 ----

    #[test]
    fn detects_for_without_update_counter_unmutated() {
        // for (int i = 0; i < n; ) { doWork(); }
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), name("n"))),
            vec![],
            body_with(vec![Stmt::ExpressionStmt(ExprStmt {
                expr: method_call("doWork"),
                line: 2,
            })]),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "for 缺 update 且计数器未变应报 #1");
        assert!(vs[0].message.contains("缺少更新表达式"), "应给出 #1 提示");
    }

    #[test]
    fn ignores_for_with_update() {
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), name("n"))),
            vec![unary_inc("i")],
            body_with(vec![Stmt::ExpressionStmt(ExprStmt {
                expr: method_call("doWork"),
                line: 2,
            })]),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "有 update 的正常 for 不报");
    }

    #[test]
    fn ignores_for_counter_mutated_in_body() {
        // for (int i = 0; i < n; ) { i = 5; } —— 计数器在循环体内被修改
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), name("n"))),
            vec![],
            body_with(vec![assign_stmt("i", int_lit("5"))]),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "计数器在循环体内被修改则不报 #1");
    }

    #[test]
    fn detects_for_constant_bound_no_update() {
        // for (int i = 0; i < 10; ) { doWork(); } —— 常量边界，i 永不达 10
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![],
            body_with(vec![Stmt::ExpressionStmt(ExprStmt {
                expr: method_call("doWork"),
                line: 2,
            })]),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "常量边界 for 缺 update 必死循环（definite）");
    }

    // ---- #2：常量传播恒真 ----

    #[test]
    fn detects_const_propagation_field() {
        // final boolean T = true; while (T) {}
        let field = final_bool_field("T", "true");
        let w = while_stmt(name("T"), empty_block());
        let unit = unit_with_field(field, vec![w]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "final 字段常量 T=true 应判死循环（#2）");
        assert!(vs[0].message.contains("常量传播"), "应说明常量传播");
    }

    #[test]
    fn detects_const_propagation_local() {
        // boolean running = true; while (running) {}
        let body = vec![
            local_bool_stmt("running", true_expr()),
            while_stmt(name("running"), empty_block()),
        ];
        let unit = unit_with(body);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "有效终态局部布尔常量应判死循环（#2）");
    }

    #[test]
    fn ignores_const_propagation_local_reassigned() {
        // boolean running = true; running = false; while (running) {}
        let body = vec![
            local_bool_stmt("running", true_expr()),
            assign_stmt("running", true_expr()),
            while_stmt(name("running"), empty_block()),
        ];
        let unit = unit_with(body);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "被重赋值的局部变量不再是常量，不报");
    }

    // ---- #3：条件变量从不更新 ----

    #[test]
    fn detects_while_var_never_updated_external_state() {
        // boolean b = ext(); while (b) { doWork(); }
        let body = vec![
            local_bool_stmt("b", method_call("ext")),
            while_stmt(
                name("b"),
                body_with(vec![Stmt::ExpressionStmt(ExprStmt {
                    expr: method_call("doWork"),
                    line: 2,
                })]),
            ),
        ];
        let unit = unit_with(body);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "while(外部状态变量) 且从不更新应报 #3");
        assert!(vs[0].message.contains("从未被修改"), "应给出 #3 提示");
    }

    #[test]
    fn ignores_while_var_mutated_in_body() {
        // while (b) { b = false; }
        let w = while_stmt(name("b"), body_with(vec![assign_stmt("b", true_expr())]));
        let unit = unit_with(vec![w]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "条件变量在循环体内被修改则不报 #3");
    }

    #[test]
    fn detects_do_while_var_never_updated() {
        // boolean b = ext(); do { doWork(); } while (b);
        let body = vec![
            local_bool_stmt("b", method_call("ext")),
            do_while_stmt(
                name("b"),
                body_with(vec![Stmt::ExpressionStmt(ExprStmt {
                    expr: method_call("doWork"),
                    line: 2,
                })]),
            ),
        ];
        let unit = unit_with(body);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "do-while(变量) 且从不更新应报 #3");
    }

    #[test]
    fn ignores_while_var_with_break_event_loop() {
        // boolean running = true; while (running) { if (x) break; }
        let body = vec![
            local_bool_stmt("running", true_expr()),
            while_stmt(
                name("running"),
                body_with(vec![Stmt::IfStmt(IfStmt {
                    condition: name("x"),
                    then_stmt: Box::new(Stmt::BreakStmt(BreakStmt { label: None, line: 3 })),
                    else_stmt: None,
                    line: 2,
                })]),
            ),
        ];
        let unit = unit_with(body);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "存在 break 的合法事件循环不报");
    }

    // ---- #1b：无效 update ----

    #[test]
    fn detects_for_self_assign_update() {
        // for (int i = 0; i < 10; i = i) { }
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "=", name("i"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i = i 自赋值应报 #1b");
        assert!(vs[0].message.contains("无实际效果"), "应给出 #1b 提示");
    }

    // ---- #1b 回归：无效 update 必须考虑初始值与入口条件 ----

    #[test]
    fn ignores_for_reset_to_non_initial_value() {
        // for (int i = 10; i > 0; i = 0) —— i = 0 使条件立即为 false，循环正常终止
        let f = for_loop(
            Some(int_var_init("i", "10")),
            Some(bin(">", name("i"), int_lit("0"))),
            vec![assign_update("i", "=", int_lit("0"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "i = 0 与初始值 10 不同，循环可正常终止，不应报 #1b");
    }

    #[test]
    fn ignores_for_ineffective_update_never_entered() {
        // for (int i = 10; i < 10; i = 0) —— 入口条件为 false，循环体根本不执行
        let f = for_loop(
            Some(int_var_init("i", "10")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "=", int_lit("0"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "入口条件恒 false 的循环不应报死循环");
    }

    #[test]
    fn ignores_for_effective_update_alongside_ineffective() {
        // for (int i = 0; i < 10; i = 0, i++) —— i++ 仍能推进，循环可终止
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "=", int_lit("0")), unary_inc("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "i = 0 与 i++ 并存时循环仍会推进，不应报 #1b");
    }

    // ---- #1 回归：条件中的非 init 变量被修改时不应误报 ----

    #[test]
    fn ignores_for_cond_var_mutated_in_body() {
        // for (int i = 0; i < n; ) { n -= 1; } —— n 递减使条件终将变为 false
        let dec = Stmt::ExpressionStmt(ExprStmt {
            expr: Expr::AssignExpr(AssignExpr {
                target: Box::new(name("n")),
                op: "-=".to_string(),
                value: Box::new(int_lit("1")),
                line: 2,
            }),
            line: 2,
        });
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), name("n"))),
            vec![],
            body_with(vec![dec]),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "条件变量 n 在循环体内被修改，循环可能终止，不应报 #1");
    }

    // ---- #1c 回归：无关变量的 update 不应中断方向矛盾检测 ----

    #[test]
    fn detects_direction_conflict_with_unrelated_update_first() {
        // for (int i = 0, j = 0; i < 10; j++, i--) —— j++ 在前，i-- 仍需被检出
        let multi = Expr::VariableDeclarationExpr(VarDeclStmt {
            var_type: Some("int".to_string()),
            declarations: vec![
                VarDeclarator {
                    name: "i".to_string(),
                    initializer: Some(int_lit("0")),
                },
                VarDeclarator {
                    name: "j".to_string(),
                    initializer: Some(int_lit("0")),
                },
            ],
            line: 1,
        });
        let f = for_loop(
            Some(multi),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![unary_inc("j"), unary_dec("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "j++ 之后 i-- 与 i < 10 矛盾仍应检出 #1c");
    }

    // ---- for-each 遍历 ----

    fn for_each_stmt(iterable: Expr, body: Stmt) -> Stmt {
        Stmt::ForEachStmt(ForEachStmt {
            variable: Expr::VariableDeclarationExpr(VarDeclStmt {
                var_type: Some("String".to_string()),
                declarations: vec![VarDeclarator {
                    name: "s".to_string(),
                    initializer: None,
                }],
                line: 2,
            }),
            iterable,
            body: Box::new(body),
            line: 2,
        })
    }

    #[test]
    fn for_each_break_does_not_exit_outer_loop() {
        // while (true) { for (String s : list) { break; } } —— break 只退出 for-each
        let fe = for_each_stmt(name("list"), Stmt::BreakStmt(BreakStmt { label: None, line: 3 }));
        let unit = unit_with(vec![while_true(body_with(vec![fe]))]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "for-each 内的 break 不退出外层 while(true)，外层仍应报死循环");
    }

    #[test]
    fn for_each_return_exits_outer_loop() {
        // while (true) { for (String s : list) { return; } } —— return 退出整个方法
        let fe = for_each_stmt(name("list"), Stmt::ReturnStmt(ReturnStmt { expr: None, line: 3 }));
        let unit = unit_with(vec![while_true(body_with(vec![fe]))]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "for-each 内的 return 会退出外层 while(true)，不应报死循环");
    }

    #[test]
    fn for_each_body_loops_are_checked() {
        // for (String s : list) { while (true) {} } —— 体内死循环应被检出
        let inner = Stmt::WhileStmt(WhileStmt {
            condition: true_expr(),
            body: Box::new(empty_block()),
            line: 3,
        });
        let fe = for_each_stmt(name("list"), body_with(vec![inner]));
        let unit = unit_with(vec![fe]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "for-each 体内的 while(true) 应被检出");
        assert_eq!(vs[0].line, 3);
    }

    #[test]
    fn detects_for_zero_plus_update() {
        // for (int i = 0; i < 10; i += 0) { }
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "+=", int_lit("0"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i += 0 无效更新应报 #1b");
    }

    #[test]
    fn detects_for_reset_to_zero_update() {
        // for (int i = 0; i < 10; i = 0) { }
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "=", int_lit("0"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i = 0 重置为初始值应报 #1b");
    }

    #[test]
    fn ignores_for_effective_update() {
        // for (int i = 0; i < 10; i += 2) { } — 正常更新
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "+=", int_lit("2"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "i += 2 正常更新不报");
    }

    // ---- #1c：update 方向矛盾 ----

    #[test]
    fn detects_for_decrement_with_lt_condition() {
        // for (int i = 0; i < 10; i--) { } — i 递减但条件是 i < 10
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![unary_dec("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i-- 与 i < 10 矛盾应报 #1c");
        assert!(vs[0].message.contains("方向与条件矛盾"), "应给出 #1c 提示");
    }

    #[test]
    fn detects_for_increment_with_gt_condition() {
        // for (int i = 10; i > 0; i++) { } — i 递增但条件是 i > 0
        let f = for_loop(
            Some(int_var_init("i", "10")),
            Some(bin(">", name("i"), int_lit("0"))),
            vec![unary_inc("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i++ 与 i > 0 矛盾应报 #1c");
    }

    #[test]
    fn detects_for_subtract_with_lt_condition() {
        // for (int i = 0; i < 10; i -= 1) { } — i 递减但条件是 i < 10
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![assign_update("i", "-=", int_lit("1"))],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 1, "i -= 1 与 i < 10 矛盾应报 #1c");
    }

    #[test]
    fn ignores_for_correct_direction() {
        // for (int i = 0; i < 10; i++) { } — 正常递增
        let f = for_loop(
            Some(int_var_init("i", "0")),
            Some(bin("<", name("i"), int_lit("10"))),
            vec![unary_inc("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "i++ 与 i < 10 方向一致不报");
    }

    #[test]
    fn ignores_for_correct_decrement() {
        // for (int i = 10; i > 0; i--) { } — 正常递减
        let f = for_loop(
            Some(int_var_init("i", "10")),
            Some(bin(">", name("i"), int_lit("0"))),
            vec![unary_dec("i")],
            empty_block(),
        );
        let unit = unit_with(vec![f]);
        let vs = InfiniteLoopRule::new().check_unit(&unit);
        assert_eq!(vs.len(), 0, "i-- 与 i > 0 方向一致不报");
    }
}
