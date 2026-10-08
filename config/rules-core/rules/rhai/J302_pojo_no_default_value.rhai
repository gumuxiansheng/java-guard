//! rule: J302
//! title: POJO 属性不得设置默认值
//! severity: major
//! category: convention

// 规范 4.4【强制】定义 DO/DTO/VO/BO 等 POJO 类时，不要设定任何属性默认值。
// 反例：POJO 的 gmtCreate 默认值为 new Date()，数据提取时未置入具体值，
// 更新其它字段时又附带更新了此字段，导致创建时间被修改。

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

    if is_pojo(nm) && members.type_of() == "array" {
        for m in members {
            if m["kind"] == "FieldDeclaration" {
                // initializer 为 JSON null 时 type_of() 是 "()"，非 "map" 即无默认值
                if m["initializer"].type_of() == "map" {
                    vs.push(#{
                        line: m["line"],
                        message: "POJO（" + nm + "）的属性 '" + m["name"].to_string() + "' 不得设置默认值，应由使用方在使用时显式赋值"
                    });
                }
            }
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
