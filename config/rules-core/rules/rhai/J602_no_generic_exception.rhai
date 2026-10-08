//! rule: J602
//! title: 禁止抛出的通用异常类型
//! severity: major
//! category: convention

// 规范 5【强制】定义时区分 unchecked / checked 异常，避免直接抛出 RuntimeException，
// 更不允许抛出 Exception 或者 Throwable，应使用有业务含义的自定义异常。
//
// 只检查 new 出来的异常对象（ObjectCreationExpr），
// 因此 `catch (Exception e)` 这种捕获写法不会命中。

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

// 简单名或全限定名尾段匹配：java.lang.RuntimeException 与 RuntimeException 都算命中
fn is_generic_exception(cn) {
    if cn == "RuntimeException" || cn.ends_with(".RuntimeException") { return true; }
    if cn == "Exception" || cn.ends_with(".Exception") { return true; }
    if cn == "Throwable" || cn.ends_with(".Throwable") { return true; }
    return false;
}

let vs = [];
let nodes = collect(ast, "ObjectCreationExpr");
for n in nodes {
    let cn = n["class_name"].to_string();
    if is_generic_exception(cn) {
        vs.push(#{
            line: n["line"],
            message: "禁止直接创建/抛出 " + cn + "，应使用有业务含义的自定义异常"
        });
    }
}
vs
