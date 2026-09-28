//! rule: J304
//! title: 重写 equals 必须同时重写 hashCode
//! severity: major
//! category: bug

// 规范 4.5【强制】只要重写 equals，就必须重写 hashCode。
// 否则作为 HashMap 的键 / HashSet 的元素时会出现「equals 相等但 hashCode 不同」的不一致行为。

fn nested_kind(k) {
    return k == "ClassDeclaration" || k == "InterfaceDeclaration"
        || k == "EnumDeclaration" || k == "AnnotationDeclaration";
}

fn walk_type(t) {
    let vs = [];
    let nm = t["name"].to_string();
    let members = t["members"];

    let has_equals = false;
    let has_hash = false;
    if members.type_of() == "array" {
        for m in members {
            if m["kind"] == "MethodDeclaration" {
                if m["name"] == "equals" { has_equals = true; }
                if m["name"] == "hashCode" { has_hash = true; }
            }
        }
    }
    if has_equals && !has_hash {
        vs.push(#{
            line: t["line"],
            message: "类 " + nm + " 重写了 equals() 但未重写 hashCode()，作为 HashMap/HashSet 的键会出现行为不一致"
        });
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
