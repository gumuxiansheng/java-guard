//! YAML 规则加载器：从 YAML 文件加载规则定义。

use std::path::Path;

use crate::rule::YamlRule;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("failed to read rule file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse YAML in {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// 从单个 YAML 文件加载一条规则。
pub fn load_rule_file(path: &Path) -> Result<YamlRule, LoadError> {
    let content = std::fs::read_to_string(path).map_err(|e| LoadError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let rule: YamlRule = serde_yaml::from_str(&content).map_err(|e| LoadError::Yaml {
        path: path.display().to_string(),
        source: e,
    })?;
    Ok(rule)
}

/// 从目录加载所有 `.yml` / `.yaml` 规则文件。
pub fn load_rule_dir(dir: &Path) -> Result<Vec<YamlRule>, LoadError> {
    let mut rules = Vec::new();
    if !dir.is_dir() {
        return Ok(rules);
    }
    let entries = std::fs::read_dir(dir).map_err(|e| LoadError::Io {
        path: dir.display().to_string(),
        source: e,
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "yml" || ext == "yaml" {
                match load_rule_file(&path) {
                    Ok(r) => {
                        // 校验 match_fields 键是否合法，未知键会被 matcher 静默忽略，
                        // 导致规则「隐形失效」，故在加载期直接跳过并告警。
                        match r.validate() {
                            Ok(()) => rules.push(r),
                            Err(bad) => eprintln!(
                                "warn: skip rule {} ({}): unknown match_fields keys: {:?}",
                                r.id,
                                path.display(),
                                bad
                            ),
                        }
                    }
                    Err(e) => eprintln!("warn: skip rule file {path}: {e}", path = path.display()),
                }
            }
        }
    }
    Ok(rules)
}

/// 从内联 YAML 字符串加载规则（用于内置规则）。
pub fn load_rule_str(yaml: &str) -> Result<YamlRule, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::PatternKind;

    #[test]
    fn load_inline_rule() {
        let yaml = r#"
id: J001
title: "禁止 System.out.println"
severity: minor
category: code-smell
pattern:
  type: MethodCall
  match_fields:
    callee: "System.out"
    method: "println"
message: "不要使用 System.out.println"
"#;
        let rule = load_rule_str(yaml).unwrap();
        assert_eq!(rule.id, "J001");
        assert_eq!(rule.pattern.kind, PatternKind::MethodCall);
    }

    #[test]
    fn load_inline_rule_invalid() {
        let yaml = "id: 123\nbroken: [";
        assert!(load_rule_str(yaml).is_err());
    }
}
