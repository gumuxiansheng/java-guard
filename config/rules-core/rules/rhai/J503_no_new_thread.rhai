//! rule: J503
//! title: 禁止显式创建线程，必须使用线程池
//! severity: major
//! category: concurrency

// 规范 4.6【强制】线程资源必须通过线程池提供，不允许在应用中自行显式创建线程；
// 创建线程或线程池时请指定有意义的线程名称，方便出错时回溯。

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
let nodes = collect(ast, "ObjectCreationExpr");
for n in nodes {
    let cn = n["class_name"].to_string();
    if cn == "Thread" || cn.ends_with(".Thread") {
        vs.push(#{
            line: n["line"],
            message: "禁止显式 new Thread()，线程资源必须通过线程池提供并指定有意义的线程名称"
        });
    }
}
vs
