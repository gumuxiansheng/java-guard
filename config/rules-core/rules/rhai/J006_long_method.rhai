//! rule: J006
//! title: 方法不超过 50 行
//! severity: minor
//! category: code-smell
//! params: max_lines=50

let violations = [];
let max_lines = config["max_lines"];
if max_lines == () {
    max_lines = 50;
}
let types = ast["types"];
for t in types {
    for member in t["members"] {
        if member["kind"] == "MethodDeclaration" {
            let lines = member["end_line"] - member["line"];
            if lines > max_lines {
                violations.push(#{
                    line: member["line"],
                    message: "方法长度 " + lines + " 行，超过 " + max_lines + " 行限制"
                });
            }
        }
    }
}
violations
