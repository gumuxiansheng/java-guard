//! 规则核心类型：Severity、Violation、RuleId、Rule trait、ViolationCollector。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 规则 ID，全局唯一（如 "J001"）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuleId(pub String);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for RuleId {
    fn from(s: &str) -> Self {
        RuleId(s.to_string())
    }
}

impl From<String> for RuleId {
    fn from(s: String) -> Self {
        RuleId(s)
    }
}

/// 违规严重级别，从低到高排序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Minor => "minor",
            Severity::Major => "major",
            Severity::Critical => "critical",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Severity {
    type Err = SeverityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "info" => Ok(Severity::Info),
            "minor" => Ok(Severity::Minor),
            "major" => Ok(Severity::Major),
            "critical" => Ok(Severity::Critical),
            other => Err(SeverityParseError(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("unknown severity: {0}")]
pub struct SeverityParseError(String);

/// 增量扫描（git diff）时违规的报告策略。
///
/// 决定违规在行级过滤时如何与变更行范围比较：
/// - `anchor`（默认）：只有**锚点行** `violation.line` 落在变更行范围内才报告。
///   适用于大多数行级规则（如禁止 System.out、空 catch）——违规的「成因」就在锚点行。
/// - `intersect`：违规区间 `[line, end_line]` 与变更行范围相交即报告。
///   适用于结构类规则（如方法超长、死循环）——违规的成因可能位于节点内任意位置，
///   仅看锚点行会漏报「变更发生在节点内部」的新增违规。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SpanPolicy {
    #[default]
    Anchor,
    Intersect,
}

/// 一条违规记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    /// 规则 ID
    pub rule_id: RuleId,
    /// 严重级别
    pub severity: Severity,
    /// 文件路径（相对于项目根目录）
    pub file: String,
    /// 起始行号（1-indexed）
    pub line: usize,
    /// 结束行号（1-indexed，含）
    pub end_line: Option<usize>,
    /// 违规描述
    pub message: String,
}

impl Violation {
    pub fn new(
        rule_id: impl Into<RuleId>,
        severity: Severity,
        file: impl Into<String>,
        line: usize,
        message: impl Into<String>,
    ) -> Self {
        Violation {
            rule_id: rule_id.into(),
            severity,
            file: file.into(),
            line,
            end_line: None,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::Major);
        assert!(Severity::Major > Severity::Minor);
        assert!(Severity::Minor > Severity::Info);
    }

    #[test]
    fn severity_from_str() {
        assert_eq!(Severity::from_str("major").unwrap(), Severity::Major);
        assert_eq!(Severity::from_str("MINOR").unwrap(), Severity::Minor);
        assert!(Severity::from_str("unknown").is_err());
    }

    #[test]
    fn violation_serde() {
        let v = Violation::new("J001", Severity::Minor, "Foo.java", 10, "test");
        let json = serde_json::to_string(&v).unwrap();
        let back: Violation = serde_json::from_str(&json).unwrap();
        assert_eq!(v.rule_id, back.rule_id);
        assert_eq!(v.line, back.line);
    }
}

/// 规则 trait：每条规则实现此接口。
///
/// 使用泛型参数 `U` 表示 AST 根节点类型，
/// 保持 guard-core 语言无关。
pub trait Rule<U>: Send + Sync {
    /// 规则 ID（如 "J001"）。
    fn id(&self) -> &RuleId;

    /// 人类可读描述。
    fn description(&self) -> &str;

    /// 严重级别。
    fn severity(&self) -> Severity;

    /// 是否启用。
    fn enabled(&self) -> bool {
        true
    }

    /// 检查编译单元，返回违规列表。
    fn check_unit(&self, unit: &U) -> Vec<Violation>;

    /// 增量扫描时的报告策略（默认：仅按锚点行判定）。
    ///
    /// `anchor`：锚点行 `violation.line` 落在变更行范围才报告；
    /// `intersect`：违规区间与变更行范围相交即报告（结构类规则覆盖）。
    fn span_policy(&self) -> SpanPolicy {
        SpanPolicy::Anchor
    }
}

/// 违规收集器：聚合多条规则的违规结果。
#[derive(Debug, Default)]
pub struct ViolationCollector {
    violations: Vec<Violation>,
}

impl ViolationCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, v: Violation) {
        self.violations.push(v);
    }

    pub fn add_all(&mut self, mut vs: Vec<Violation>) {
        self.violations.append(&mut vs);
    }

    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    pub fn into_inner(self) -> Vec<Violation> {
        self.violations
    }

    pub fn count(&self) -> usize {
        self.violations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    /// 按文件路径+行号排序。
    pub fn sort(&mut self) {
        self.violations.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.rule_id.cmp(&b.rule_id))
        });
    }
}

#[cfg(test)]
mod collector_tests {
    use super::*;

    #[test]
    fn collector_add_and_sort() {
        let mut c = ViolationCollector::new();
        c.add(Violation::new("J002", Severity::Major, "B.java", 5, "b"));
        c.add(Violation::new("J001", Severity::Minor, "A.java", 10, "a"));
        c.add(Violation::new("J001", Severity::Minor, "A.java", 3, "a2"));
        c.sort();
        let v = c.violations();
        assert_eq!(v[0].file, "A.java");
        assert_eq!(v[0].line, 3);
        assert_eq!(v[1].file, "A.java");
        assert_eq!(v[1].line, 10);
        assert_eq!(v[2].file, "B.java");
    }
}
