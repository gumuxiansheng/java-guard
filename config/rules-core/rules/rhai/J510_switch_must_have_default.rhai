//! rule: J510
//! title: switch 语句必须包含 default 分支
//! severity: major
//! category: bug

// 规范 4.7【强制】在一个 switch 块内，都必须包含一个 default 语句并放在最后，
// 即使它什么代码也没有。
// AST 约定：SwitchCase.label == null 即表示 default 分支。

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
let nodes = collect(ast, "SwitchStmt");
for n in nodes {
    let cases = n["cases"];
    let has_default = false;
    if cases.type_of() == "array" {
        for c in cases {
            // label 为 JSON null（type_of 是 "()"）时即 default
            if c["label"].type_of() != "map" {
                has_default = true;
            }
        }
    }
    if !has_default {
        vs.push(#{
            line: n["line"],
            message: "switch 语句必须包含 default 分支并放在最后，用于处理未覆盖到的取值"
        });
    }
}
vs
