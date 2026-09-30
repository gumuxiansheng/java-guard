//! 规则包（Rule Pack）元数据与版本契约。
//!
//! 规则与引擎分开发版时，最大的风险是**规则静默失败**：旧引擎没有规则用到的
//! API 时，规则可能整条不生效却毫无报错，用户误以为「检查通过」。本模块定义
//! 版本声明与校验，把这种情况变成**可读的硬错误**。
//!
//! - `RULE_API_VERSION`：引擎暴露给规则脚本（YAML / Rhai）的全部 API 面的版本号。
//! - `[pack]` 段：可写在规则文件（javaguard.rules.toml）或独立的规则包清单中，
//!   声明包名、版本、所需 API 版本与兼容的引擎版本范围。

use serde::Deserialize;

/// 当前引擎支持的**规则 API 版本**（单调递增的整体单号）。
///
/// 语义：引擎暴露给规则的全部 API 面——YAML 规则的 pattern 类型与 match_fields 键、
/// Rhai 规则注册的类型/函数、`context`/`config` 变量字段。以下变更都必须递增本值：
/// - 增删改 YAML pattern 类型或其可用字段（新增也算，旧引擎没有该字段）；
/// - 增删改 Rhai 注册类型 / 注册函数 / 内置 helpers；
/// - 增删改规则可访问的上下文变量字段。
///
/// 规则包在 `[pack].api_version` 声明其所需版本；声明值大于本常量时引擎拒绝加载，
/// 避免「新规则跑在旧引擎上静默失效」。纯内部实现变更（解析器实现、性能优化）
/// 不影响规则可见面，**不**递增本值。
pub const RULE_API_VERSION: u32 = 1;

/// 规则包元数据（`[pack]` 段）。
///
/// 全部字段可选：缺省即「无声明」，保持对现有规则文件的完全向后兼容。
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RulePackMeta {
    /// 包名（如 "javaguard-core-rules"）。
    pub name: Option<String>,
    /// 规则 id 命名空间（可选）。设置后本包规则对外 id = `<namespace>:<id>`，
    /// 使多个包可各自拥有同名短 id 而互不冲突。字符集限制为 `[a-z0-9_-]+`。
    pub namespace: Option<String>,
    /// 包版本（SemVer）。
    pub version: Option<String>,
    /// 本包使用的规则 API 版本。缺省视为 `1`（兼容未声明版本的旧规则文件）。
    pub api_version: Option<u32>,
    /// 兼容的引擎版本范围，如 `">=0.1.7 <0.2"`。缺省表示不声明。
    pub engine: Option<String>,
    /// 包描述（可选，纯元数据）。
    pub description: Option<String>,
    /// 许可证标识（可选，纯元数据）。
    pub license: Option<String>,
}

impl RulePackMeta {
    /// 声明所需的 API 版本（缺省 = 1，兼容旧规则文件）。
    pub fn declared_api_version(&self) -> u32 {
        self.api_version.unwrap_or(1)
    }

    /// 包名（未声明时的占位），用于报错文案。
    fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("<unnamed pack>")
    }

    /// 硬校验：任一失败都必须拒绝加载（返回可读错误消息）。
    ///
    /// 覆盖三项：`api_version` 过高、`namespace` 字符集非法、`engine` 范围不可解析。
    /// 引擎版本是否落在范围内由 [`Self::engine_compat_error`] 单独判定
    /// （需要调用方提供当前引擎版本）。
    pub fn validate(&self) -> Result<(), String> {
        if let Some(err) = self.api_version_error() {
            return Err(err);
        }
        if let Some(err) = self.namespace_error() {
            return Err(err);
        }
        if let Some(range) = self.engine.as_deref() {
            if let Err(err) = parse_engine_range(range) {
                return Err(format!(
                    "rule pack '{}' has an invalid engine range '{}': {}",
                    self.display_name(),
                    range,
                    err
                ));
            }
        }
        Ok(())
    }

    /// API 版本不兼容时的错误消息（兼容则返回 `None`）。
    pub fn api_version_error(&self) -> Option<String> {
        let declared = self.declared_api_version();
        if declared <= RULE_API_VERSION {
            return None;
        }
        Some(format!(
            "rule pack '{}' requires rule API version {}, but this engine supports up to {}.\n\
             hint: upgrade java-guard, or use a rule pack written for rule API version {} or lower.",
            self.display_name(),
            declared,
            RULE_API_VERSION,
            RULE_API_VERSION
        ))
    }

    /// `namespace` 字符集校验：非空且仅含 `[a-z0-9_-]`（合法则返回 `None`）。
    pub fn namespace_error(&self) -> Option<String> {
        let ns = self.namespace.as_deref()?;
        if ns.is_empty() {
            return Some(format!(
                "rule pack '{}' has an empty namespace",
                self.display_name()
            ));
        }
        if let Some(bad) = ns
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
        {
            return Some(format!(
                "rule pack '{}' has an invalid namespace '{}': character '{}' is not allowed \
                 (allowed: a-z, 0-9, '_' and '-').",
                self.display_name(),
                ns,
                bad
            ));
        }
        None
    }

    /// 当前引擎版本不满足 `[pack].engine` 范围时的错误消息（满足/未声明则返回 `None`）。
    pub fn engine_compat_error(&self, engine_version: &str) -> Option<String> {
        let range = self.engine.as_deref()?;
        let comparators = parse_engine_range(range).ok()?;
        let current = match parse_version(engine_version) {
            Ok(v) => v,
            Err(_) => return None, // 引擎自身版本不可解析时不误判（开发构建等场景）
        };
        if comparators.iter().all(|c| c.matches(&current)) {
            return None;
        }
        Some(format!(
            "rule pack '{}' requires engine version '{}', but this java-guard is {}.\n\
             hint: upgrade/downgrade java-guard, or use a rule pack compatible with this engine.",
            self.display_name(),
            range,
            engine_version
        ))
    }
}

// ===== 极简 SemVer 范围解析（仅支持 AND 组合的比较符，不支持 `||`） =====

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);

fn parse_version(s: &str) -> Result<Version, String> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let parse_part = |p: Option<&str>, what: &str| -> Result<u64, String> {
        let p = p.ok_or_else(|| format!("missing {what} in version '{s}'"))?;
        // 允许 "0.2" 这类省略写法时缺省为 0；含预发布后缀（如 0.2.7-rc1）只取数字前缀
        let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return Err(format!("invalid {what} '{p}' in version '{s}'"));
        }
        digits
            .parse()
            .map_err(|_| format!("invalid {what} '{p}' in version '{s}'"))
    };
    let major = parse_part(parts.next(), "major")?;
    let minor = match parts.next() {
        Some(p) => parse_part(Some(p), "minor")?,
        None => 0,
    };
    let patch = match parts.next() {
        Some(p) => parse_part(Some(p), "patch")?,
        None => 0,
    };
    Ok(Version(major, minor, patch))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

#[derive(Debug, Clone, Copy)]
struct Comparator {
    op: Op,
    version: Version,
}

impl Comparator {
    fn matches(&self, v: &Version) -> bool {
        match self.op {
            Op::Gt => v > &self.version,
            Op::Ge => v >= &self.version,
            Op::Lt => v < &self.version,
            Op::Le => v <= &self.version,
            Op::Eq => v == &self.version,
        }
    }
}

/// 解析引擎版本范围（如 `">=0.1.7 <0.2"`）为比较符列表（AND 语义）。
fn parse_engine_range(range: &str) -> Result<Vec<Comparator>, String> {
    if range.contains("||") {
        return Err(
            "OR ('||') is not supported; use space- or comma-separated AND comparators, \
             e.g. \">=0.1.7 <0.2\""
                .to_string(),
        );
    }
    let mut out = Vec::new();
    for token in range.split(|c: char| c.is_whitespace() || c == ',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
            (Op::Ge, r)
        } else if let Some(r) = token.strip_prefix("<=") {
            (Op::Le, r)
        } else if let Some(r) = token.strip_prefix('>') {
            (Op::Gt, r)
        } else if let Some(r) = token.strip_prefix('<') {
            (Op::Lt, r)
        } else if let Some(r) = token.strip_prefix('=') {
            (Op::Eq, r)
        } else {
            (Op::Eq, token)
        };
        out.push(Comparator {
            op,
            version: parse_version(rest.trim())?,
        });
    }
    if out.is_empty() {
        return Err("empty engine range".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_declaration_is_compatible() {
        let meta = RulePackMeta::default();
        assert!(meta.validate().is_ok());
        assert!(meta.api_version_error().is_none());
        assert!(meta.engine_compat_error("0.1.7").is_none());
    }

    #[test]
    fn api_version_too_high_is_hard_error() {
        let meta = RulePackMeta {
            name: Some("future-pack".into()),
            api_version: Some(RULE_API_VERSION + 1),
            ..Default::default()
        };
        let err = meta.validate().unwrap_err();
        assert!(err.contains("future-pack"), "err: {err}");
        assert!(err.contains("rule API version"), "err: {err}");
    }

    #[test]
    fn api_version_equal_is_ok() {
        let meta = RulePackMeta {
            api_version: Some(RULE_API_VERSION),
            ..Default::default()
        };
        assert!(meta.validate().is_ok());
    }

    #[test]
    fn engine_range_parse_and_match() {
        let meta = RulePackMeta {
            name: Some("core".into()),
            engine: Some(">=0.1.7 <0.2".into()),
            ..Default::default()
        };
        assert!(meta.validate().is_ok());
        assert!(meta.engine_compat_error("0.1.7").is_none());
        assert!(meta.engine_compat_error("0.1.9").is_none());
        let err = meta.engine_compat_error("0.2.0").unwrap();
        assert!(err.contains("core"), "err: {err}");
        let err = meta.engine_compat_error("0.1.6").unwrap();
        assert!(err.contains(">=0.1.7 <0.2"), "err: {err}");
    }

    #[test]
    fn engine_range_invalid_is_hard_error() {
        let meta = RulePackMeta {
            engine: Some(">=0.1.7 || <0.1".into()),
            ..Default::default()
        };
        assert!(meta.validate().is_err());

        let meta = RulePackMeta {
            engine: Some(">=abc".into()),
            ..Default::default()
        };
        assert!(meta.validate().is_err());
    }

    #[test]
    fn version_parsing_variants() {
        assert_eq!(parse_version("0.1.7").unwrap(), Version(0, 1, 7));
        assert_eq!(parse_version("v1.2").unwrap(), Version(1, 2, 0));
        assert_eq!(parse_version("0.2.7-rc1").unwrap(), Version(0, 2, 7));
        assert!(parse_version("").is_err());
        assert!(parse_version("a.b.c").is_err());
    }

    #[test]
    fn engine_range_comma_separated() {
        let cs = parse_engine_range(">=0.1.7, <0.2").unwrap();
        assert_eq!(cs.len(), 2);
        assert!(cs.iter().all(|c| c.matches(&Version(0, 1, 8))));
    }

    #[test]
    fn namespace_charset_is_validated() {
        let ok = RulePackMeta {
            namespace: Some("spring-boot_rules2".into()),
            ..Default::default()
        };
        assert!(ok.validate().is_ok());

        let bad = RulePackMeta {
            name: Some("p".into()),
            namespace: Some("Spring Boot".into()),
            ..Default::default()
        };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("invalid namespace"), "err: {err}");

        let empty = RulePackMeta {
            namespace: Some(String::new()),
            ..Default::default()
        };
        assert!(empty.validate().is_err());
    }
}
