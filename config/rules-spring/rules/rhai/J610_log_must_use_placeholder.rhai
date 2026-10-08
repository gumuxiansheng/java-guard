//! rule: J610
//! title: 日志输出必须使用占位符而非字符串拼接
//! severity: minor
//! category: performance

// 规范 4.10【强制】在日志输出时，字符串变量之间的拼接使用占位符的方式。
// 因为 String 拼接会用 StringBuilder.append()，有一定性能损耗；
// 使用占位符仅是替换动作，可有效提升性能。
// 正例：logger.debug("Processing trade with id: {} and symbol: {}", id, symbol);

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

// 日志方法名白名单
fn is_log_method(m) {
    return m == "trace" || m == "debug" || m == "info" || m == "warn" || m == "error";
}

// 调用者限定为日志对象，避免把业务方法 debug()/info() 误判成日志调用。
// 覆盖 log / LOG / Logger 等常见写法（logger 含 log 子串）。
fn is_logger(callee) {
    return callee.contains("log") || callee.contains("Log") || callee.contains("LOG");
}

let vs = [];
let nodes = collect(ast, "MethodCallExpr");
for n in nodes {
    let m = n["method_name"].to_string();
    if is_log_method(m) {
        let callee = n["callee"].to_string();
        if is_logger(callee) {
            let args = n["arguments"];
            if args.type_of() == "array" && len(args) > 0 {
                let a0 = args[0];
                if a0["kind"] == "BinaryExpr" && a0["op"] == "+" {
                    vs.push(#{
                        line: n["line"],
                        message: "日志输出应使用占位符 {} 而非字符串拼接，避免无谓的 StringBuilder 开销"
                    });
                }
            }
        }
    }
}
vs
