//! rule: J011
//! title: StringUtils 应使用 org.apache.commons.lang3.StringUtils
//! severity: minor
//! category: convention

// 推荐 org.apache.commons.lang3.StringUtils；
// 不允许「其它包」的同名/近名工具类（如 commons-lang(2.x)、spring、fastjson 等）。
// 注意：allow 的是整个 org.apache.commons.lang3 包（该类唯一推荐实现即 StringUtils），
// 因此 lang3 包下的导入不告警，仅对其它包命中。
let violations = [];
let allowed_prefix = "org.apache.commons.lang3.";
let imports = ast["imports"];
for imp in imports {
    let pkg = imp["package"];
    // 仅检查类名为 StringUtils 或 StringUtil 的导入（含其余包路径）
    let is_target = pkg.ends_with(".StringUtils") || pkg.ends_with(".StringUtil");
    if is_target && !pkg.starts_with(allowed_prefix) {
        violations.push(#{
            line: imp["line"],
            message: "StringUtils 应使用 org.apache.commons.lang3.StringUtils，不建议使用其它包（" + pkg + "）"
        });
    }
}
violations
