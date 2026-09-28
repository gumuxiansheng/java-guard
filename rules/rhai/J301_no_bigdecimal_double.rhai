//! rule: J301
//! title: 禁止使用 new BigDecimal(double) 构造
//! severity: major
//! category: bug

// 规范 4.4【强制】禁止使用构造方法 BigDecimal(double) 把 double 转化为 BigDecimal，
// 存在精度损失风险，应改用 BigDecimal(String) 或 BigDecimal.valueOf(double)。

// 通用：递归收集 AST 中所有 kind == want 的节点。
// Rhai 中函数是脚本局部作用域，因此这里不使用顶层 let 常量，全部靠参数传递。
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

// 判断字面量是否是浮点：含小数点，或以 d/D/f/F 结尾。
// 注意 LiteralExpr.value 是 expr.toString()，数字后缀会保留（如 1.5d）。
fn is_float_literal(v) {
    let n = len(v);
    if n == 0 { return false; }
    if v.contains(".") { return true; }
    let last = v.sub_string(n - 1, 1);
    if last == "d" || last == "D" { return true; }
    if last == "f" || last == "F" { return true; }
    return false;
}

let vs = [];
let nodes = collect(ast, "ObjectCreationExpr");
for n in nodes {
    if n["class_name"] == "BigDecimal" {
        let args = n["arguments"];
        if args.type_of() == "array" && len(args) > 0 {
            let a = args[0];
            if a["kind"] == "LiteralExpr" {
                let v = a["value"].to_string();
                if is_float_literal(v) {
                    vs.push(#{
                        line: n["line"],
                        message: "禁止用 new BigDecimal(double) 构造（精度损失风险），请改用 new BigDecimal(String) 或 BigDecimal.valueOf(double)"
                    });
                }
            }
        }
    }
}
vs
