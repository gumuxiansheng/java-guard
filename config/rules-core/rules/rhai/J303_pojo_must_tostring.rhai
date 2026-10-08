//! rule: J303
//! title: POJO 必须实现 toString 方法
//! severity: major
//! category: convention

// 规范 4.4【强制】POJO 类必须写 toString 方法；抛出异常时可直接调用
// toString() 打印属性值，便于排查问题。继承另一个 POJO 时记得加 super.toString()。

fn is_pojo(nm) {
    return nm.ends_with("DO") || nm.ends_with("DTO")
        || nm.ends_with("BO") || nm.ends_with("VO");
}

fn nested_kind(k) {
    return k == "ClassDeclaration" || k == "InterfaceDeclaration"
        || k == "EnumDeclaration" || k == "AnnotationDeclaration";
}

fn walk_type(t) {
    let vs = [];
    let nm = t["name"].to_string();
    let members = t["members"];

    if is_pojo(nm) {
        let has_to_string = false;
        if members.type_of() == "array" {
            for m in members {
                if m["kind"] == "MethodDeclaration" && m["name"] == "toString" {
                    has_to_string = true;
                }
            }
        }
        if !has_to_string {
            vs.push(#{
                line: t["line"],
                message: "POJO（" + nm + "）必须实现 toString()，便于异常排查时打印属性值"
            });
        }
    }

    if members.type_of() == "array" {
        for m in members {
            let k = m["kind"].to_string();
            if nested_kind(k) {
                vs = vs + walk_type(m);
            }
        }
    }
    return vs;
}

let all = [];
let types = ast["types"];
if types.type_of() == "array" {
    for t in types { all = all + walk_type(t); }
}
all
