//! rule: J601
//! title: finally 块中禁止使用 return
//! severity: major
//! category: bug

// 规范 5【强制】不能在 finally 块中使用 return——finally 块中的 return 返回后
// 方法结束执行，不会再执行 try 块中的 return 语句，且会吞掉正在抛出的异常。
//
// 实现说明：对 finally_body 递归查找 ReturnStmt，
// 因此 `finally { if (x) { return 1; } }` 这类嵌套写法同样能检出。

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
let trys = collect(ast, "TryStmt");
for t in trys {
    let fin = t["finally_body"];
    if fin.type_of() == "map" {
        let rets = collect(fin, "ReturnStmt");
        for r in rets {
            vs.push(#{
                line: r["line"],
                message: "finally 块中禁止使用 return——它会覆盖 try/catch 的返回值并吞掉异常"
            });
        }
    }
}
vs
