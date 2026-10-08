//! rule: J013
//! title: Spring Controller HTTP 接口禁止使用 Map 传参
//! severity: major
//! category: convention

// 规则：@Controller / @RestController 类中的 HTTP 接口方法
// （@GetMapping / @PostMapping / @PutMapping / @DeleteMapping / @PatchMapping / @RequestMapping）
// 不允许以 Map 作为入参类型；应使用明确的请求参数对象（DTO）来定义请求内容。
//
// 覆盖的 Map 类型（含泛型与全限定名形式）：
//   Map / Map<String, Object> / java.util.Map
//   HashMap / LinkedHashMap / TreeMap / ConcurrentHashMap
// （不拦截 Spring 的 MultiValueMap 等专用类型，避免误报。）

// 去掉泛型参数：Map<String, Object> -> Map；java.util.Map<String,Object> -> java.util.Map
fn strip_generics(t) {
    let n = len(t);
    let i = 0;
    while i < n {
        let c = t.sub_string(i, 1);
        if c == "<" {
            return t.sub_string(0, i);
        }
        i = i + 1;
    }
    return t;
}

// 判断参数类型是否为「松散 Map」：
// 覆盖裸类型、泛型形式，以及 java.util 全限定形式（带或不带泛型）。
// 不拦截 Spring 的 MultiValueMap 等专用类型，避免误报。
fn is_map_type(t) {
    let base = strip_generics(t);
    return base == "Map"
        || base == "HashMap"
        || base == "LinkedHashMap"
        || base == "TreeMap"
        || base == "ConcurrentHashMap"
        || base == "java.util.Map"
        || base == "java.util.HashMap"
        || base == "java.util.LinkedHashMap"
        || base == "java.util.TreeMap"
        || base == "java.util.ConcurrentHashMap";
}

fn ann_ends_with(anns, suffix) {
    if anns.type_of() != "array" { return false; }
    for a in anns {
        let n = a["name"];
        if n.type_of() == "string" && n.ends_with(suffix) { return true; }
    }
    return false;
}

fn is_controller(anns) {
    return ann_ends_with(anns, "Controller");
}

fn is_http_endpoint(anns) {
    return ann_ends_with(anns, "GetMapping")
        || ann_ends_with(anns, "PostMapping")
        || ann_ends_with(anns, "PutMapping")
        || ann_ends_with(anns, "DeleteMapping")
        || ann_ends_with(anns, "PatchMapping")
        || ann_ends_with(anns, "RequestMapping");
}

// 递归检查一个类型；返回该类型（及其嵌套 controller 类型）内的所有违规。
fn visit_type(t) {
    let vs = [];
    let members = t["members"];
    if members.type_of() != "array" { return vs; }
    let ctrl = is_controller(t["annotations"]);
    for m in members {
        let kind = m["kind"];
        if kind == "ClassDeclaration" || kind == "InterfaceDeclaration" {
            // 嵌套类型自身也是 Controller 时才继续检查
            if is_controller(m["annotations"]) {
                vs = vs + visit_type(m);
            }
        } else if kind == "MethodDeclaration" {
            if ctrl && is_http_endpoint(m["annotations"]) {
                let params = m["parameters"];
                if params.type_of() == "array" {
                    for p in params {
                        let pt = p["param_type"];
                        if pt.type_of() == "string" && is_map_type(pt) {
                            vs.push(#{
                                line: m["line"],
                                message: "HTTP 接口 '" + m["name"] + "' 不允许使用 Map 传参（参数类型 " + pt + "），应定义明确的请求参数对象（DTO）"
                            });
                        }
                    }
                }
            }
        }
    }
    return vs;
}

let all = [];
let types = ast["types"];
if types.type_of() == "array" {
    for t in types {
        if is_controller(t["annotations"]) {
            all = all + visit_type(t);
        }
    }
}
all
