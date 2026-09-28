//! rule: J511
//! title: if / 循环语句必须使用大括号
//! severity: major
//! category: convention

// 规范 4.7【强制】在 if / else / for / while / do 语句中必须使用大括号，
// 即使只有一行代码，避免使用 `if (condition) statements;` 这种单行形式。
// 注意：else if（else_stmt 本身就是 IfStmt）属于正常写法，不视为违规。

fn collect(n, want) {
    let acc = [];
    if n.type_of() != "map" { return acc; }
    if n["kind"] == want { acc.push(n); }
    for key in n.keys() {
        let v = n[key];
        if v.type_of() == "array" {
            for item in v {
                if item.type_of() == "map" {
                    acc = acc + collect(item, want);
                }
            }
        } else if v.type_of() == "map" {
            acc = acc + collect(v, want);
        }
    }
    return acc;
}

let vs = [];

let ifs = collect(ast, "IfStmt");
for n in ifs {
    let then = n["then_stmt"];
    if then.type_of() == "map" && then["kind"] != "BlockStmt" {
        vs.push(#{ line: n["line"], message: "if 语句必须使用大括号，即使只有一行代码" });
    }
    let els = n["else_stmt"];
    if els.type_of() == "map" && els["kind"] != "BlockStmt" && els["kind"] != "IfStmt" {
        vs.push(#{ line: n["line"], message: "else 分支必须使用大括号，即使只有一行代码" });
    }
}

let loops = collect(ast, "ForStmt") + collect(ast, "ForEachStmt")
    + collect(ast, "WhileStmt") + collect(ast, "DoStmt");
for n in loops {
    let b = n["body"];
    if b.type_of() == "map" && b["kind"] != "BlockStmt" {
        vs.push(#{ line: n["line"], message: "循环体必须使用大括号，即使只有一行代码" });
    }
}

vs
