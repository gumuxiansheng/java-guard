//! rule: J402
//! title: 集合转数组必须使用 toArray(T[] array)
//! severity: major
//! category: bug

// 规范 4.5【强制】使用集合转数组的方法，必须使用 toArray(T[] array)，
// 传入类型完全一样的数组。无参 toArray() 返回 Object[]，强转会抛 ClassCastException。

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
let nodes = collect(ast, "MethodCallExpr");
for n in nodes {
    if n["method_name"] == "toArray" {
        let args = n["arguments"];
        if args.type_of() != "array" || len(args) == 0 {
            vs.push(#{
                line: n["line"],
                message: "集合转数组必须使用 toArray(T[] array)，无参 toArray() 返回 Object[]，强转会抛 ClassCastException"
            });
        }
    }
}
vs
