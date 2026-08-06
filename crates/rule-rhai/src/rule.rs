//! Rhai 脚本规则定义与加载。

use guard_core::rule::SpanPolicy;
use serde::Deserialize;
use thiserror::Error;

/// 一条 Rhai 脚本规则的元数据。
#[derive(Debug, Clone, Deserialize)]
pub struct RhaiRule {
    /// 规则 ID
    pub id: String,
    /// 规则标题
    pub title: String,
    /// 严重级别
    pub severity: String,
    /// 分类
    #[serde(default)]
    pub category: String,
    /// Rhai 脚本内容
    pub script: String,
    /// 是否默认启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 规则参数（注入到脚本的 config 变量）
    #[serde(default)]
    pub params: serde_yaml::Value,
    /// 增量扫描时的报告策略：anchor（默认）/ intersect
    #[serde(default)]
    pub span_policy: SpanPolicy,
}

fn default_true() -> bool {
    true
}

/// 解析头部 `enabled` 值，兼容 YAML 布尔写法（大小写不敏感）。无法识别时返回 `None`。
fn parse_bool_header(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Error)]
pub enum RhaiRuleError {
    #[error("failed to parse rule YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("script compilation failed: {0}")]
    Compile(String),
}

/// 从 YAML 字符串加载 Rhai 规则。
pub fn load_rhai_rule_str(yaml: &str) -> Result<RhaiRule, RhaiRuleError> {
    let rule: RhaiRule = serde_yaml::from_str(yaml)?;
    Ok(rule)
}

/// 从文件加载 Rhai 规则。支持 .yml/.yaml（YAML 元数据+内嵌脚本）和 .rhai（纯脚本+头部元数据注释）。
pub fn load_rhai_rule_file(path: &std::path::Path) -> Result<RhaiRule, RhaiRuleError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| RhaiRuleError::Compile(format!("read {}: {e}", path.display())))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rhai" => parse_rhai_script(&content),
        _ => load_rhai_rule_str(&content),
    }
}

/// 从纯 .rhai 脚本文件解析规则。
///
/// 文件头部用 `//!` 注释声明元数据，格式：
/// ```text
/// //! rule: J006
/// //! title: 方法不超过 50 行
/// //! severity: minor
/// //! category: code-smell
/// //! enabled: true
/// //! params: max_lines=50,threshold=10
/// ```
/// 注释块之后为 Rhai 脚本正文。`rule` 必填；`severity` 缺省为 `minor`，其余可选。
/// `enabled` 支持 true/false/yes/no/on/off/1/0（大小写不敏感）。
pub fn parse_rhai_script(content: &str) -> Result<RhaiRule, RhaiRuleError> {
    // 去除 UTF-8 BOM（部分 Windows 编辑器默认写入）
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let mut id = String::new();
    let mut title = String::new();
    let mut severity = "minor".to_string();
    let mut category = String::new();
    let mut enabled = true;
    let mut params_str = String::new();
    let mut script_lines = Vec::new();
    let mut in_header = true;
    let mut saw_meta = false;

    for line in content.lines() {
        if in_header {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("//!") {
                saw_meta = true;
                let rest = rest.trim();
                if let Some((key, val)) = rest.split_once(':') {
                    let key = key.trim();
                    let val = val.trim();
                    match key {
                        "rule" | "id" => id = val.to_string(),
                        "title" | "name" => title = val.to_string(),
                        "severity" => severity = val.to_string(),
                        "category" => category = val.to_string(),
                        "enabled" => {
                            enabled = parse_bool_header(val).ok_or_else(|| {
                                RhaiRuleError::Compile(format!(
                                    "invalid `enabled` value `{val}` in .rhai file header"
                                ))
                            })?;
                        }
                        "params" => params_str = val.to_string(),
                        _ => {}
                    }
                }
                continue;
            } else if trimmed.is_empty() && !saw_meta {
                // 首个元数据行之前的空行
                continue;
            } else {
                // 第一个非元数据行（空行、普通注释或正文）结束头部，
                // 保证正文开头的 `//` 注释块原样保留
                in_header = false;
                script_lines.push(line.to_string());
            }
        } else {
            script_lines.push(line.to_string());
        }
    }

    if id.is_empty() {
        return Err(RhaiRuleError::Compile(
            "missing `//! rule:` metadata in .rhai file header".to_string(),
        ));
    }
    if title.is_empty() {
        title = id.clone();
    }

    // 解析 params: key=value,key2=value2
    let params = if params_str.is_empty() {
        serde_yaml::Value::Null
    } else {
        let mut map = serde_yaml::Mapping::new();
        for pair in params_str.split(',') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                // 尝试解析为数字
                if let Ok(n) = v.parse::<i64>() {
                    map.insert(serde_yaml::Value::String(k.to_string()), serde_yaml::Value::Number(n.into()));
                } else if let Ok(f) = v.parse::<f64>() {
                    map.insert(serde_yaml::Value::String(k.to_string()), serde_yaml::Value::Number(serde_yaml::Number::from(f)));
                } else if v == "true" || v == "false" {
                    map.insert(serde_yaml::Value::String(k.to_string()), serde_yaml::Value::Bool(v == "true"));
                } else {
                    map.insert(serde_yaml::Value::String(k.to_string()), serde_yaml::Value::String(v.to_string()));
                }
            }
        }
        serde_yaml::Value::Mapping(map)
    };

    let script = script_lines.join("\n");

    Ok(RhaiRule {
        id,
        title,
        severity,
        category,
        script,
        enabled,
        params,
        span_policy: guard_core::rule::SpanPolicy::Anchor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rhai_rule() {
        let yaml = r#"
id: J006
title: "方法不超过 50 行"
severity: minor
category: code-smell
script: |
  let unit = ast;
  let violations = [];
  for type in unit["types"] {
    for member in type["members"] {
      if member["kind"] == "MethodDeclaration" {
        let lines = member["end_line"] - member["line"];
        if lines > 50 {
          violations.push({
            line: member["line"],
            message: "方法长度 " + lines + " 行，超过 50 行限制"
          });
        }
      }
    }
  }
  violations
"#;
        let rule = load_rhai_rule_str(yaml).unwrap();
        assert_eq!(rule.id, "J006");
        assert!(rule.script.contains("violations"));
    }

    #[test]
    fn parse_pure_rhai_script() {
        let script = r#"
//! rule: J006
//! title: 方法不超过 50 行
//! severity: minor
//! category: code-smell
//! params: max_lines=50

let violations = [];
let max_lines = config["max_lines"];
if max_lines == () { max_lines = 50; }
violations
"#;
        let rule = parse_rhai_script(script).unwrap();
        assert_eq!(rule.id, "J006");
        assert_eq!(rule.title, "方法不超过 50 行");
        assert_eq!(rule.severity, "minor");
        assert_eq!(rule.category, "code-smell");
        assert!(rule.enabled);
        assert!(rule.script.contains("let violations = []"));
        assert!(rule.script.contains("config[\"max_lines\"]"));
        // params 解析
        match &rule.params {
            serde_yaml::Value::Mapping(m) => {
                assert_eq!(m.get(&serde_yaml::Value::String("max_lines".into())),
                    Some(&serde_yaml::Value::Number(50.into())));
            }
            _ => panic!("expected mapping"),
        }
    }

    #[test]
    fn parse_rhai_script_missing_id_errors() {
        let script = "//! title: no id\nlet x = 1;";
        assert!(parse_rhai_script(script).is_err());
    }

    #[test]
    fn parse_rhai_script_default_severity() {
        let script = "//! rule: J999\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        assert_eq!(rule.id, "J999");
        assert_eq!(rule.severity, "minor"); // 默认
        assert_eq!(rule.title, "J999"); // 默认 title = id
    }

    #[test]
    fn parse_rhai_script_params_types() {
        let script = "//! rule: J100\n//! params: max_lines=50,enabled=true,name=foo,pi=3.14\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        match &rule.params {
            serde_yaml::Value::Mapping(m) => {
                assert_eq!(m.get(&serde_yaml::Value::String("max_lines".into())),
                    Some(&serde_yaml::Value::Number(50.into())));
                assert_eq!(m.get(&serde_yaml::Value::String("enabled".into())),
                    Some(&serde_yaml::Value::Bool(true)));
                assert_eq!(m.get(&serde_yaml::Value::String("name".into())),
                    Some(&serde_yaml::Value::String("foo".into())));
                // pi=3.14 -> f64
                if let Some(serde_yaml::Value::Number(n)) = m.get(&serde_yaml::Value::String("pi".into())) {
                    assert!((n.as_f64().unwrap() - 3.14).abs() < 0.001);
                } else { panic!("expected number"); }
            }
            _ => panic!("expected mapping"),
        }
    }

    #[test]
    fn parse_rhai_script_enabled_case_insensitive() {
        let script = "//! rule: J101\n//! enabled: True\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        assert!(rule.enabled);

        let script = "//! rule: J102\n//! enabled: OFF\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        assert!(!rule.enabled);

        let script = "//! rule: J103\n//! enabled: YES\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        assert!(rule.enabled);
    }

    #[test]
    fn parse_rhai_script_invalid_enabled_errors() {
        let script = "//! rule: J104\n//! enabled: maybe\nlet x = 1;";
        assert!(parse_rhai_script(script).is_err());
    }

    #[test]
    fn parse_rhai_script_keeps_body_comments() {
        let script = "\
//! rule: J105
//! title: 保留正文注释

// 正文开头的注释块
// 应当原样保留
let x = 1;
x
";
        let rule = parse_rhai_script(script).unwrap();
        assert_eq!(rule.id, "J105");
        assert!(rule.script.contains("// 正文开头的注释块"), "script: {}", rule.script);
        assert!(rule.script.contains("// 应当原样保留"), "script: {}", rule.script);
    }

    #[test]
    fn parse_rhai_script_strips_utf8_bom() {
        let script = "\u{FEFF}//! rule: J106\n//! severity: major\nlet x = 1;";
        let rule = parse_rhai_script(script).unwrap();
        assert_eq!(rule.id, "J106");
        assert_eq!(rule.severity, "major");
        assert!(rule.script.contains("let x = 1;"));
    }

    #[test]
    fn load_rhai_rule_file_supports_both_formats() {
        let dir = std::env::temp_dir().join("javaguard_rhai_format_test");
        let _ = std::fs::create_dir_all(&dir);

        // .yml 格式
        let yml_path = dir.join("J006.yml");
        std::fs::write(&yml_path, r#"
id: J006
title: "YAML format"
severity: minor
script: |
  let x = 1;
  x
"#).unwrap();
        let rule_yml = load_rhai_rule_file(&yml_path).unwrap();
        assert_eq!(rule_yml.id, "J006");
        assert_eq!(rule_yml.title, "YAML format");

        // .rhai 格式
        let rhai_path = dir.join("J007.rhai");
        std::fs::write(&rhai_path, r#"
//! rule: J007
//! title: Rhai format
//! severity: major

let x = 1;
x
"#).unwrap();
        let rule_rhai = load_rhai_rule_file(&rhai_path).unwrap();
        assert_eq!(rule_rhai.id, "J007");
        assert_eq!(rule_rhai.title, "Rhai format");
        assert_eq!(rule_rhai.severity, "major");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
