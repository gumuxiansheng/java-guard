//! CI gate 逻辑：根据违规数量和阈值决定退出码。
//!
//! 退出码：
//! - 0：无违规或违规均在阈值内
//! - 1：存在违规超过阈值（CI gate 失败）
//! - 2：扫描过程出错

use serde::{Deserialize, Serialize};

use crate::rule::Severity;

/// Gate 阈值配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateConfig {
    /// critical 级别最大允许数（默认 0）
    pub max_critical: usize,
    /// major 级别最大允许数（默认 0）
    pub max_major: usize,
    /// minor 级别最大允许数（默认不限制）
    pub max_minor: usize,
    /// info 级别最大允许数（默认不限制）
    pub max_info: usize,
}

impl Default for GateConfig {
    fn default() -> Self {
        GateConfig {
            max_critical: 0,
            max_major: 0,
            max_minor: usize::MAX,
            max_info: usize::MAX,
        }
    }
}

impl GateConfig {
    /// 从 YAML 配置文件加载。
    ///
    /// ```yaml
    /// gate:
    ///   max_critical: 0
    ///   max_major: 0
    ///   max_minor: 10
    ///   max_info: 100
    /// ```
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        #[derive(Deserialize)]
        struct Wrapper {
            gate: GateConfig,
        }
        let w: Wrapper = serde_yaml::from_str(yaml)?;
        Ok(w.gate)
    }

    /// 检查违规是否超过阈值。
    pub fn check(&self, counts: &SeverityCounts) -> GateResult {
        let mut failures = Vec::new();

        if counts.critical > self.max_critical {
            failures.push(format!(
                "critical: {} > {}",
                counts.critical, self.max_critical
            ));
        }
        if counts.major > self.max_major {
            failures.push(format!(
                "major: {} > {}",
                counts.major, self.max_major
            ));
        }
        if counts.minor > self.max_minor {
            failures.push(format!(
                "minor: {} > {}",
                counts.minor, self.max_minor
            ));
        }
        if counts.info > self.max_info {
            failures.push(format!(
                "info: {} > {}",
                counts.info, self.max_info
            ));
        }

        if failures.is_empty() {
            GateResult::Pass
        } else {
            GateResult::Fail(failures)
        }
    }
}

/// 按严重级别统计违规数。
#[derive(Debug, Clone, Default)]
pub struct SeverityCounts {
    pub critical: usize,
    pub major: usize,
    pub minor: usize,
    pub info: usize,
}

impl SeverityCounts {
    pub fn from_violations(violations: &[crate::rule::Violation]) -> Self {
        let mut counts = SeverityCounts::default();
        for v in violations {
            match v.severity {
                Severity::Critical => counts.critical += 1,
                Severity::Major => counts.major += 1,
                Severity::Minor => counts.minor += 1,
                Severity::Info => counts.info += 1,
            }
        }
        counts
    }
}

/// Gate 检查结果。
#[derive(Debug, Clone)]
pub enum GateResult {
    Pass,
    Fail(Vec<String>),
}

impl GateResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, GateResult::Pass)
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            GateResult::Pass => 0,
            GateResult::Fail(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Violation;

    fn make_violations(critical: usize, major: usize, minor: usize, info: usize) -> Vec<Violation> {
        let mut vs = Vec::new();
        for _ in 0..critical {
            vs.push(Violation::new("T", Severity::Critical, "f.java", 1, "c"));
        }
        for _ in 0..major {
            vs.push(Violation::new("T", Severity::Major, "f.java", 1, "m"));
        }
        for _ in 0..minor {
            vs.push(Violation::new("T", Severity::Minor, "f.java", 1, "mi"));
        }
        for _ in 0..info {
            vs.push(Violation::new("T", Severity::Info, "f.java", 1, "i"));
        }
        vs
    }

    #[test]
    fn gate_pass_when_no_violations() {
        let config = GateConfig::default();
        let counts = SeverityCounts::default();
        assert!(config.check(&counts).is_pass());
    }

    #[test]
    fn gate_fail_when_major_exceeds() {
        let config = GateConfig::default();
        let vs = make_violations(0, 1, 0, 0);
        let counts = SeverityCounts::from_violations(&vs);
        let result = config.check(&counts);
        assert!(!result.is_pass());
        assert_eq!(result.exit_code(), 1);
    }

    #[test]
    fn gate_pass_when_within_threshold() {
        let config = GateConfig {
            max_critical: 0,
            max_major: 5,
            max_minor: usize::MAX,
            max_info: usize::MAX,
        };
        let vs = make_violations(0, 3, 10, 5);
        let counts = SeverityCounts::from_violations(&vs);
        assert!(config.check(&counts).is_pass());
    }

    #[test]
    fn gate_fail_multiple_levels() {
        let config = GateConfig {
            max_critical: 0,
            max_major: 0,
            max_minor: 5,
            max_info: usize::MAX,
        };
        let vs = make_violations(1, 2, 10, 3);
        let counts = SeverityCounts::from_violations(&vs);
        let result = config.check(&counts);
        assert!(matches!(result, GateResult::Fail(_)));
        if let GateResult::Fail(reasons) = result {
            assert!(reasons.iter().any(|r| r.contains("critical")));
            assert!(reasons.iter().any(|r| r.contains("major")));
            assert!(reasons.iter().any(|r| r.contains("minor")));
        }
    }
}
