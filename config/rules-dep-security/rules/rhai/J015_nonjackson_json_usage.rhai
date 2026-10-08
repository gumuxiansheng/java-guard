//! rule: J015
//! title: 禁止在代码中使用非 jackson（com.fasterxml.jackson）的 JSON 框架
//! severity: major
//! category: convention

// 部门规范第 15 条：增量代码禁止使用除中台默认 jackson 之外的 JSON 框架。
// 与 J014（检查 import 语句）互补：本规则检查「代码中对非 jackson JSON 框架类的实际使用点」。
// 增量扫描（--diff）下，import 行往往不在变更范围内，导致 J014 漏报；
// 本规则在「使用行」上报，使用行即为变更行，因此增量模式下依然有效。
//
// 覆盖的非 jackson JSON 框架（fastjson 由 J012 负责，这里不重复，避免双重报告）：
//   com.google.gson                (Gson)
//   org.json                       (JSON-java，含 org.json.simple / json-simple)
//   net.sf.json                    (json-lib)
//   org.codehaus.jackson           (Jackson 1.x，已废弃，区别于默认的 2.x)
// 允许的默认框架 com.fasterxml.jackson（2.x）不会被命中。
//
// 注意：本引擎中脚本函数无法读取顶层 let 全局变量（与 J012 同理），
// 因此包名清单以内联函数 is_bad_prefix / bad_str 承载，不依赖全局数组。

// 判断某包名/类型名是否以被禁 JSON 框架包名开头（用于 import 收集）。
fn is_bad_prefix(p) {
    return p.starts_with("com.google.gson")
        || p.starts_with("org.json")
        || p.starts_with("net.sf.json")
        || p.starts_with("org.codehaus.jackson");
}

// 判断字符串是否包含任一被禁 JSON 框架包名（用于全限定名写法）。
fn bad_str(s) {
    if s.type_of() != "string" { return false; }
    return s.contains("com.google.gson")
        || s.contains("org.json")
        || s.contains("net.sf.json")
        || s.contains("org.codehaus.jackson");
}

// 收集从被禁 JSON 框架 import 进来的简单类名（如 Gson / JSONObject）。
let imported = [];
let imports = ast["imports"];
if imports != () {
    for imp in imports {
        let pkg = imp["package"];
        let name = pkg;
        if name.ends_with(".*") { name = name.sub_string(0, len(name) - 2); }
        if is_bad_prefix(name) {
            let parts = name.split('.');
            let simple = parts[len(parts) - 1];
            if simple != "" && simple != "gson" && simple != "json" && simple != "jackson" {
                imported.push(simple);
            }
        }
    }
}

fn walk_expr(e, imported) {
    let vs = [];
    if e.type_of() != "map" { return vs; }
    let kind = e["kind"];
    if kind == "MethodCallExpr" {
        let callee = e["callee"];
        if callee != () {
            if bad_str(callee) {
                vs.push(#{ line: e["line"], msg: "禁止使用非 jackson（com.fasterxml.jackson）的 JSON 框架（" + callee + "），请改用 jackson；如必须使用请走部门审核确认" });
            } else if callee in imported {
                vs.push(#{ line: e["line"], msg: "禁止使用非 jackson 的 JSON 框架（" + callee + "），请改用 jackson（com.fasterxml.jackson）" });
            }
        }
        let args = e["arguments"];
        if args != () { for a in args { vs = vs + walk_expr(a, imported); } }
        return vs;
    }
    if kind == "ObjectCreationExpr" {
        let cn = e["class_name"];
        if bad_str(cn) {
            vs.push(#{ line: e["line"], msg: "禁止使用非 jackson（com.fasterxml.jackson）的 JSON 框架（" + cn + "），请改用 jackson；如必须使用请走部门审核确认" });
        } else if cn in imported {
            vs.push(#{ line: e["line"], msg: "禁止使用非 jackson 的 JSON 框架（" + cn + "），请改用 jackson（com.fasterxml.jackson）" });
        }
        let args = e["arguments"];
        if args != () { for a in args { vs = vs + walk_expr(a, imported); } }
        return vs;
    }
    if kind == "CastExpr" {
        let ct = e["cast_type"];
        if bad_str(ct) {
            vs.push(#{ line: e["line"], msg: "禁止使用非 jackson（com.fasterxml.jackson）的 JSON 框架（" + ct + "），请改用 jackson；如必须使用请走部门审核确认" });
        }
        vs = vs + walk_expr(e["expr"], imported);
        return vs;
    }
    if kind == "InstanceOfExpr" {
        let ct = e["check_type"];
        if bad_str(ct) {
            vs.push(#{ line: e["line"], msg: "禁止使用非 jackson（com.fasterxml.jackson）的 JSON 框架（" + ct + "），请改用 jackson；如必须使用请走部门审核确认" });
        }
        vs = vs + walk_expr(e["expr"], imported);
        return vs;
    }
    if kind == "FieldAccessExpr" { return vs + walk_expr(e["target"], imported); }
    if kind == "EnclosedExpr" { return vs + walk_expr(e["inner"], imported); }
    if kind == "BinaryExpr" { return vs + walk_expr(e["left"], imported) + walk_expr(e["right"], imported); }
    if kind == "UnaryExpr" { return vs + walk_expr(e["expr"], imported); }
    if kind == "AssignExpr" { return vs + walk_expr(e["target"], imported) + walk_expr(e["value"], imported); }
    if kind == "VariableDeclarationExpr" {
        let decls = e["declarations"];
        if decls != () { for d in decls { if d["initializer"] != () { vs = vs + walk_expr(d["initializer"], imported); } } }
        return vs;
    }
    if kind == "MethodReferenceExpr" {
        let t = e["target"];
        if t.type_of() == "string" && bad_str(t) {
            vs.push(#{ line: e["line"], msg: "禁止使用非 jackson（com.fasterxml.jackson）的 JSON 框架（" + t + "），请改用 jackson；如必须使用请走部门审核确认" });
        }
        return vs;
    }
    return vs;
}

fn walk_stmt(s, imported) {
    let vs = [];
    if s.type_of() != "map" { return vs; }
    let kind = s["kind"];
    if kind == "BlockStmt" {
        let stmts = s["statements"];
        if stmts != () { for x in stmts { vs = vs + walk_stmt(x, imported); } }
        return vs;
    }
    if kind == "IfStmt" {
        if s["condition"] != () { vs = vs + walk_expr(s["condition"], imported); }
        if s["then_stmt"] != () { vs = vs + walk_stmt(s["then_stmt"], imported); }
        if s["else_stmt"] != () { vs = vs + walk_stmt(s["else_stmt"], imported); }
        return vs;
    }
    if kind == "ExpressionStmt" {
        if s["expr"] != () { vs = vs + walk_expr(s["expr"], imported); }
        return vs;
    }
    if kind == "ReturnStmt" {
        if s["expr"] != () { vs = vs + walk_expr(s["expr"], imported); }
        return vs;
    }
    if kind == "VariableDeclarationStmt" {
        let decls = s["declarations"];
        if decls != () { for d in decls { if d["initializer"] != () { vs = vs + walk_expr(d["initializer"], imported); } } }
        return vs;
    }
    if kind == "ForStmt" {
        if s["initialization"] != () { vs = vs + walk_expr(s["initialization"], imported); }
        if s["condition"] != () { vs = vs + walk_expr(s["condition"], imported); }
        if s["update"] != () { for u in s["update"] { vs = vs + walk_expr(u, imported); } }
        if s["body"] != () { vs = vs + walk_stmt(s["body"], imported); }
        return vs;
    }
    if kind == "ForEachStmt" {
        vs = vs + walk_expr(s["variable"], imported);
        vs = vs + walk_expr(s["iterable"], imported);
        if s["body"] != () { vs = vs + walk_stmt(s["body"], imported); }
        return vs;
    }
    if kind == "WhileStmt" {
        if s["condition"] != () { vs = vs + walk_expr(s["condition"], imported); }
        if s["body"] != () { vs = vs + walk_stmt(s["body"], imported); }
        return vs;
    }
    if kind == "DoStmt" {
        if s["body"] != () { vs = vs + walk_stmt(s["body"], imported); }
        if s["condition"] != () { vs = vs + walk_expr(s["condition"], imported); }
        return vs;
    }
    if kind == "TryStmt" {
        if s["try_body"] != () { vs = vs + walk_stmt(s["try_body"], imported); }
        if s["finally_body"] != () { vs = vs + walk_stmt(s["finally_body"], imported); }
        let catches = s["catch_clauses"];
        if catches != () { for c in catches { if c["body"] != () { vs = vs + walk_stmt(c["body"], imported); } } }
        return vs;
    }
    if kind == "ThrowStmt" {
        if s["expr"] != () { vs = vs + walk_expr(s["expr"], imported); }
        return vs;
    }
    if kind == "SynchronizedStmt" {
        if s["expr"] != () { vs = vs + walk_expr(s["expr"], imported); }
        if s["body"] != () { vs = vs + walk_stmt(s["body"], imported); }
        return vs;
    }
    if kind == "SwitchStmt" {
        if s["selector"] != () { vs = vs + walk_expr(s["selector"], imported); }
        let cases = s["cases"];
        if cases != () { for c in cases { let cs = c["statements"]; if cs != () { for x in cs { vs = vs + walk_stmt(x, imported); } } } }
        return vs;
    }
    return vs;
}

fn walk_member(m, imported) {
    let vs = [];
    let kind = m["kind"];
    if kind == "MethodDeclaration" || kind == "ConstructorDeclaration" {
        if m["body"] != () { vs = vs + walk_stmt(m["body"], imported); }
        return vs;
    }
    if kind == "ClassDeclaration" || kind == "InterfaceDeclaration" ||
       kind == "EnumDeclaration" || kind == "AnnotationDeclaration" {
        let members = m["members"];
        if members != () { for x in members { vs = vs + walk_member(x, imported); } }
        return vs;
    }
    if kind == "FieldDeclaration" {
        if m["initializer"] != () { vs = vs + walk_expr(m["initializer"], imported); }
        return vs;
    }
    if kind == "InitializerDeclaration" {
        if m["body"] != () { vs = vs + walk_stmt(m["body"], imported); }
        return vs;
    }
    return vs;
}

let violations = [];
let types = ast["types"];
if types != () {
    for t in types { violations = violations + walk_member(t, imported); }
}

// 将 {line, msg} 转为引擎要求的 {line, message}
let out = [];
for v in violations { out.push(#{ line: v["line"], message: v["msg"] }); }
out
