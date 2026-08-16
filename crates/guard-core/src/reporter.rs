//! 报告输出格式与 Reporter 实现。
//!
//! 支持 Console / JSON / SARIF / CSV 四种格式。

use std::io::{self, Write as IoWrite};

use serde::{Deserialize, Serialize};

use crate::rule::{Severity, Violation};

/// 支持的报告格式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    /// 控制台彩色输出
    Console,
    /// JSON 格式
    Json,
    /// SARIF 2.1.0 格式
    Sarif,
    /// CSV 格式
    Csv,
}

impl Default for ReportFormat {
    fn default() -> Self {
        ReportFormat::Console
    }
}

impl std::str::FromStr for ReportFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "console" => Ok(ReportFormat::Console),
            "json" => Ok(ReportFormat::Json),
            "sarif" => Ok(ReportFormat::Sarif),
            "csv" => Ok(ReportFormat::Csv),
            other => Err(format!("unknown report format: {other}")),
        }
    }
}

/// ANSI 颜色码。
mod color {
    pub const RED: &str = "\x1b[31m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GRAY: &str = "\x1b[90m";
    pub const GREEN: &str = "\x1b[32m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RESET: &str = "\x1b[0m";
}

/// 严重级别 → 控制台标签（如 `[MAJOR]`）与颜色（与 sql-guard 的 ERROR/WARN 风格对齐）。
fn severity_tag(s: Severity) -> (&'static str, &'static str) {
    match s {
        Severity::Critical => ("CRITICAL", color::RED),
        Severity::Major => ("MAJOR", color::RED),
        Severity::Minor => ("MINOR", color::YELLOW),
        Severity::Info => ("INFO", color::CYAN),
    }
}

/// 扫描统计信息。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanStats {
    pub files_scanned: usize,
    pub parse_errors: usize,
    pub total_violations: usize,
    pub by_severity: BySeverity,
    pub by_rule: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BySeverity {
    pub critical: usize,
    pub major: usize,
    pub minor: usize,
    pub info: usize,
}

impl ScanStats {
    pub fn from_violations(violations: &[Violation], files_scanned: usize, parse_errors: usize) -> Self {
        let mut stats = ScanStats {
            files_scanned,
            parse_errors,
            total_violations: violations.len(),
            ..Default::default()
        };
        for v in violations {
            match v.severity {
                Severity::Critical => stats.by_severity.critical += 1,
                Severity::Major => stats.by_severity.major += 1,
                Severity::Minor => stats.by_severity.minor += 1,
                Severity::Info => stats.by_severity.info += 1,
            }
            *stats.by_rule.entry(v.rule_id.to_string()).or_default() += 1;
        }
        stats
    }
}

// ─── Console Reporter ───────────────────────────────────────────

/// 控制台 Reporter：sql-guard 同款版式输出违规报告。
///
/// ```text
/// ════════════════════════════════════════════════════════════
///  JavaGuard Report
/// ════════════════════════════════════════════════════════════
///
/// Files checked: 1
///
/// ── Violations ──
/// ────────────────────────────────────────
///   [MAJOR] src/Foo.java:3 (rule: J010)
///         禁止使用 fastjson ...
///
/// ────────────────────────────────────────
/// Summary: 2 major(s)
/// ```
pub struct ConsoleReporter;

/// 横幅宽度（═ 个数），与 sql-guard 一致。
const BANNER_WIDTH: usize = 60;
/// 分节分隔线宽度（─ 个数），与 sql-guard 一致。
const SEP_WIDTH: usize = 40;

impl ConsoleReporter {
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations, 0, 0, None)
    }

    pub fn report_to<W: IoWrite>(
        w: &mut W,
        violations: &[Violation],
        files_scanned: usize,
        parse_errors: usize,
        _duration_ms: Option<u64>,
    ) -> io::Result<()> {
        // ── Banner ──
        writeln!(w, "{}{}{}", color::GRAY, "═".repeat(BANNER_WIDTH), color::RESET)?;
        writeln!(w, "{} JavaGuard Report{}", color::BOLD, color::RESET)?;
        writeln!(w, "{}{}{}", color::GRAY, "═".repeat(BANNER_WIDTH), color::RESET)?;
        writeln!(w)?;
        writeln!(w, "{}Files{} checked: {files_scanned}", color::BOLD, color::RESET)?;
        writeln!(w)?;

        if violations.is_empty() {
            writeln!(w, "{}✓ No violations found{}", color::GREEN, color::RESET)?;
            writeln!(w)?;
            Self::write_summary(w, violations, parse_errors)
        } else {
            writeln!(w, "{}{}── Violations ──{}", color::RED, color::BOLD, color::RESET)?;
            writeln!(w, "{}{}{}", color::GRAY, "─".repeat(SEP_WIDTH), color::RESET)?;

            // 按严重级别降序 + 文件路径/行号排序（与 sql-guard 一致：最严重的排最前）
            let mut sorted = violations.to_vec();
            sorted.sort_by(|a, b| {
                b.severity
                    .cmp(&a.severity)
                    .then_with(|| a.file.cmp(&b.file))
                    .then_with(|| a.line.cmp(&b.line))
            });

            for v in &sorted {
                let (tag, sev_color) = severity_tag(v.severity);
                // 位置串：单行为 `:8`，跨行为 `:8-9`（与 sql-guard 的 file:line[-end] 对齐）
                let loc = match v.end_line {
                    Some(end) if end > v.line => format!(":{}-{}", v.line, end),
                    _ => format!(":{}", v.line),
                };
                writeln!(
                    w,
                    "  {}{}[{}]{} {}{}{} {}{}(rule: {}){}",
                    color::BOLD,
                    sev_color,
                    tag,
                    color::RESET,
                    color::BOLD,
                    v.file,
                    loc,
                    color::RESET,
                    color::GRAY,
                    v.rule_id,
                    color::RESET,
                )?;
                writeln!(w, "        {}", v.message)?;
            }
            writeln!(w)?;

            Self::write_summary(w, &sorted, parse_errors)
        }
    }

    /// 输出 Summary 行：非零级别计数（sql-guard 风格），全零时显示 All checks passed。
    fn write_summary<W: IoWrite>(
        w: &mut W,
        violations: &[Violation],
        parse_errors: usize,
    ) -> io::Result<()> {
        writeln!(w, "{}{}{}", color::GRAY, "─".repeat(SEP_WIDTH), color::RESET)?;
        write!(w, "{}Summary{}: ", color::BOLD, color::RESET)?;

        let stats = ScanStats::from_violations(violations, 0, parse_errors);
        let mut parts: Vec<String> = Vec::new();
        for (count, label, c) in [
            (stats.by_severity.critical, "critical", color::RED),
            (stats.by_severity.major, "major", color::RED),
            (stats.by_severity.minor, "minor", color::YELLOW),
            (stats.by_severity.info, "info", color::CYAN),
        ] {
            if count > 0 {
                parts.push(format!("{c}{} {label}(s){}", count, color::RESET));
            }
        }
        if stats.parse_errors > 0 {
            parts.push(format!(
                "{}{} parse error(s){}",
                color::YELLOW, stats.parse_errors, color::RESET
            ));
        }

        if parts.is_empty() {
            writeln!(w, "{}All checks passed{}", color::GREEN, color::RESET)
        } else {
            writeln!(w, "{}", parts.join(" "))
        }
    }
}

// ─── JSON Reporter ───────────────────────────────────────────────

/// JSON 报告根结构。
#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub version: String,
    pub scan_info: JsonScanInfo,
    pub violations: Vec<Violation>,
    pub stats: ScanStats,
}

#[derive(Debug, Serialize)]
pub struct JsonScanInfo {
    pub timestamp: String,
    pub files_scanned: usize,
    pub parse_errors: usize,
    pub duration_ms: Option<u64>,
}

/// JSON Reporter。
pub struct JsonReporter;

impl JsonReporter {
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations, 0, 0, None)
    }

    pub fn report_to<W: IoWrite>(
        w: &mut W,
        violations: &[Violation],
        files_scanned: usize,
        parse_errors: usize,
        duration_ms: Option<u64>,
    ) -> io::Result<()> {
        let stats = ScanStats::from_violations(violations, files_scanned, parse_errors);
        let report = JsonReport {
            version: "1.0".to_string(),
            scan_info: JsonScanInfo {
                timestamp: chrono_now(),
                files_scanned,
                parse_errors,
                duration_ms,
            },
            violations: violations.to_vec(),
            stats,
        };
        let json = serde_json::to_string_pretty(&report)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(w, "{json}")?;
        Ok(())
    }
}

/// 生成当前 UTC 时间的 RFC3339 字符串，例如 `2024-01-02T03:04:05Z`。
///
/// 旧实现把 Unix 秒直接当作「当天秒数」，产出 `1970-01-01T00:00:1753987200Z`
/// 这种非法 ISO8601，导致所有 JSON/SARIF 报告的 timestamp 字段都是垃圾值。
/// 这里基于 Howard Hinnant 的公历换算算法，无需引入外部依赖即可输出合法时间。
fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format_rfc3339(d.as_secs(), d.subsec_nanos())
}

/// 将 Unix 时间（秒 + 纳秒）格式化为 UTC RFC3339（以 `Z` 结尾）。
///
/// 采用逐年简单推算（在合理时间范围内足够且易于验证），避免引入 chrono 依赖，
/// 也不依赖易出错的闭式日期算法。
fn format_rfc3339(secs: u64, nanos: u32) -> String {
    const SECS_PER_DAY: u64 = 86_400;
    let days = (secs / SECS_PER_DAY) as i64;
    let mut rem = (secs % SECS_PER_DAY) as i64;
    let hour = rem / 3600;
    rem %= 3600;
    let minute = rem / 60;
    let second = rem % 60;

    let (year, month, day) = yday_to_date(days);

    if nanos == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{nanos:09}Z"
        )
    }
}

/// days since 1970-01-01 -> (year, month, day)，逐年推算（简单且可验证）。
fn yday_to_date(mut days: i64) -> (i64, i32, i32) {
    let mut year = 1970;
    loop {
        let ydays: i64 = if is_leap_year(year) { 366 } else { 365 };
        if days < ydays {
            break;
        }
        days -= ydays;
        year += 1;
    }
    let (month, day) = yday_to_month_day(year, days as u32);
    (year, month, day)
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn yday_to_month_day(year: i64, yday: u32) -> (i32, i32) {
    let month_days: [u32; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut rem = yday;
    for (i, &md) in month_days.iter().enumerate() {
        if rem < md {
            return ((i + 1) as i32, (rem + 1) as i32);
        }
        rem -= md;
    }
    (12, 31)
}

// ─── SARIF Reporter ──────────────────────────────────────────────

/// SARIF 2.1.0 Reporter。
pub struct SarifReporter;

impl SarifReporter {
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations, 0, 0, None)
    }

    pub fn report_to<W: IoWrite>(
        w: &mut W,
        violations: &[Violation],
        _files_scanned: usize,
        _parse_errors: usize,
        _duration_ms: Option<u64>,
    ) -> io::Result<()> {
        let sarif = Self::build_sarif(violations);
        let json = serde_json::to_string_pretty(&sarif)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(w, "{json}")?;
        Ok(())
    }

    fn build_sarif(violations: &[Violation]) -> serde_json::Value {
        use serde_json::{json, Value};

        // 收集规则
        let mut rules_map: std::collections::BTreeMap<String, &Violation> =
            std::collections::BTreeMap::new();
        for v in violations {
            rules_map.entry(v.rule_id.to_string()).or_insert(v);
        }

        let rules: Vec<Value> = rules_map
            .iter()
            .map(|(id, v)| {
                json!({
                    "id": id,
                    "name": id,
                    "shortDescription": {
                        "text": id
                    },
                    "defaultConfiguration": {
                        "level": severity_to_sarif_level(v.severity)
                    }
                })
            })
            .collect();

        let results: Vec<Value> = violations
            .iter()
            .map(|v| {
                json!({
                    "ruleId": v.rule_id.to_string(),
                    "level": severity_to_sarif_level(v.severity),
                    "message": {
                        "text": v.message
                    },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": {
                                "uri": v.file
                            },
                            "region": {
                                "startLine": v.line,
                                "endLine": v.end_line.unwrap_or(v.line)
                            }
                        }
                    }]
                })
            })
            .collect();

        json!({
            "version": "2.1.0",
            "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/Schemata/sarif-schema-2.1.0.json",
            "runs": [{
                "tool": {
                    "driver": {
                        "name": "java-guard",
                        "version": env!("CARGO_PKG_VERSION"),
                        "informationUri": "https://github.com/javaguard/java-guard",
                        "rules": rules
                    }
                },
                "results": results
            }]
        })
    }
}

fn severity_to_sarif_level(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "error",
        Severity::Major => "error",
        Severity::Minor => "warning",
        Severity::Info => "note",
    }
}

// ─── CSV Reporter ─────────────────────────────────────────────────

/// CSV Reporter。
pub struct CsvReporter;

impl CsvReporter {
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations, 0, 0, None)
    }

    pub fn report_to<W: IoWrite>(
        w: &mut W,
        violations: &[Violation],
        _files_scanned: usize,
        _parse_errors: usize,
        _duration_ms: Option<u64>,
    ) -> io::Result<()> {
        writeln!(w, "rule_id,severity,file,line,end_line,message")?;
        for v in violations {
            let end = v.end_line.map(|e| e.to_string()).unwrap_or_default();
            let msg = csv_escape(&v.message);
            writeln!(w, "{},{},{},{},{},{}", v.rule_id, v.severity, v.file, v.line, end, msg)?;
        }
        Ok(())
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ─── 统一入口 ─────────────────────────────────────────────────────

/// 统一 Reporter 入口（控制台输出）。
pub fn report(format: &ReportFormat, violations: &[Violation]) -> io::Result<()> {
    match format {
        ReportFormat::Console => ConsoleReporter::report(violations),
        ReportFormat::Json => JsonReporter::report(violations),
        ReportFormat::Sarif => SarifReporter::report(violations),
        ReportFormat::Csv => CsvReporter::report(violations),
    }
}

/// 统一 Reporter 入口（写入指定 writer，带统计信息）。
pub fn report_to<W: IoWrite>(
    format: &ReportFormat,
    w: &mut W,
    violations: &[Violation],
    files_scanned: usize,
    parse_errors: usize,
    duration_ms: Option<u64>,
) -> io::Result<()> {
    match format {
        ReportFormat::Console => ConsoleReporter::report_to(w, violations, files_scanned, parse_errors, duration_ms),
        ReportFormat::Json => JsonReporter::report_to(w, violations, files_scanned, parse_errors, duration_ms),
        ReportFormat::Sarif => SarifReporter::report_to(w, violations, files_scanned, parse_errors, duration_ms),
        ReportFormat::Csv => CsvReporter::report_to(w, violations, files_scanned, parse_errors, duration_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Violation;

    fn sample_violations() -> Vec<Violation> {
        vec![
            Violation::new("J001", Severity::Minor, "Foo.java", 10, "missing serial version UID"),
            Violation::new("J002", Severity::Major, "Bar.java", 5, "empty catch block"),
            Violation::new("J001", Severity::Minor, "Foo.java", 3, "duplicate import"),
        ]
    }

    /// 去除 ANSI 转义序列，便于对纯文本内容做断言。
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // 跳过 `[...m` 形式的颜色码
                if chars.next() == Some('[') {
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn console_report_produces_output() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &vs, 3, 0, None).unwrap();
        let output = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(output.contains("JavaGuard Report"));
        assert!(output.contains("Files checked: 3"));
        assert!(output.contains("Foo.java"));
        assert!(output.contains("Bar.java"));
        assert!(output.contains("J001"));
        assert!(output.contains("J002"));
        // sql-guard 风格的违规行与 Summary
        assert!(output.contains("[MAJOR]"));
        assert!(output.contains("[MINOR]"));
        assert!(output.contains("(rule: J001)"));
        assert!(output.contains("Summary:"));
        assert!(output.contains("1 major(s)"));
        assert!(output.contains("2 minor(s)"));
    }

    #[test]
    fn console_report_empty() {
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &[], 5, 0, None).unwrap();
        let output = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(output.contains("Files checked: 5"));
        assert!(output.contains("No violations found"));
        assert!(output.contains("All checks passed"));
    }

    #[test]
    fn console_report_parse_errors_in_summary() {
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &[], 2, 1, None).unwrap();
        let output = strip_ansi(&String::from_utf8(buf).unwrap());
        assert!(output.contains("1 parse error(s)"));
        assert!(!output.contains("All checks passed"));
    }

    #[test]
    fn json_report_valid_json() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        JsonReporter::report_to(&mut buf, &vs, 3, 0, Some(100)).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["violations"].as_array().unwrap().len(), 3);
        assert_eq!(parsed["scan_info"]["files_scanned"], 3);
        assert_eq!(parsed["scan_info"]["duration_ms"], 100);
        assert_eq!(parsed["stats"]["total_violations"], 3);
    }

    #[test]
    fn sarif_report_valid() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        SarifReporter::report_to(&mut buf, &vs, 0, 0, None).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["version"], "2.1.0");
        assert_eq!(parsed["runs"][0]["tool"]["driver"]["name"], "java-guard");
        assert_eq!(parsed["runs"][0]["results"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn csv_report_has_header() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        CsvReporter::report_to(&mut buf, &vs, 0, 0, None).unwrap();
        let csv = String::from_utf8(buf).unwrap();
        assert!(csv.starts_with("rule_id,severity,file,line,end_line,message"));
        assert!(csv.contains("J001"));
        assert!(csv.contains("J002"));
    }

    #[test]
    fn csv_escape_comma() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("hello, world"), "\"hello, world\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn report_format_from_str() {
        assert_eq!("console".parse::<ReportFormat>().unwrap(), ReportFormat::Console);
        assert_eq!("JSON".parse::<ReportFormat>().unwrap(), ReportFormat::Json);
        assert_eq!("sarif".parse::<ReportFormat>().unwrap(), ReportFormat::Sarif);
        assert_eq!("csv".parse::<ReportFormat>().unwrap(), ReportFormat::Csv);
        assert!("unknown".parse::<ReportFormat>().is_err());
    }

    #[test]
    fn scan_stats_correct() {
        let vs = sample_violations();
        let stats = ScanStats::from_violations(&vs, 3, 1);
        assert_eq!(stats.total_violations, 3);
        assert_eq!(stats.by_severity.minor, 2);
        assert_eq!(stats.by_severity.major, 1);
        assert_eq!(stats.by_rule.get("J001"), Some(&2));
        assert_eq!(stats.by_rule.get("J002"), Some(&1));
        assert_eq!(stats.files_scanned, 3);
        assert_eq!(stats.parse_errors, 1);
    }

    #[test]
    fn rfc3339_epoch_is_valid() {
        assert_eq!(format_rfc3339(0, 0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn rfc3339_with_nanos() {
        assert_eq!(
            format_rfc3339(0, 500_000_000),
            "1970-01-01T00:00:00.500000000Z"
        );
    }

    #[test]
    fn rfc3339_known_timestamp() {
        // 2024-01-02T03:04:05Z == 1_704_164_645 秒（UTC）
        assert_eq!(format_rfc3339(1_704_164_645, 0), "2024-01-02T03:04:05Z");
    }

    #[test]
    fn yday_to_date_epoch() {
        assert_eq!(yday_to_date(0), (1970, 1, 1));
        assert_eq!(yday_to_date(1), (1970, 1, 2));
        assert_eq!(yday_to_date(31), (1970, 2, 1));
        assert_eq!(yday_to_date(365), (1971, 1, 1));
        assert_eq!(yday_to_date(7_258), (1989, 11, 15));
        assert_eq!(yday_to_date(19_723), (2024, 1, 1));
    }

    #[test]
    fn json_timestamp_is_valid_rfc3339() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        JsonReporter::report_to(&mut buf, &vs, 3, 0, Some(100)).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let ts = parsed["scan_info"]["timestamp"].as_str().unwrap();
        // 合法 RFC3339：以 Z 结尾，且不是旧的非法格式
        assert!(ts.ends_with('Z'), "timestamp {ts} 应以 Z 结尾");
        assert!(
            !ts.starts_with("1970-01-01T00:00:"),
            "timestamp {ts} 不应是旧的非法格式"
        );
    }

    #[test]
    fn sarif_severity_levels_mapping() {
        let vs = vec![
            Violation::new("J1", Severity::Critical, "A.java", 1, "c"),
            Violation::new("J2", Severity::Major, "A.java", 2, "m"),
            Violation::new("J3", Severity::Minor, "A.java", 3, "i"),
            Violation::new("J4", Severity::Info, "A.java", 4, "n"),
        ];
        let sarif = SarifReporter::build_sarif(&vs);
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results[0]["level"], "error");
        assert_eq!(results[1]["level"], "error");
        assert_eq!(results[2]["level"], "warning");
        assert_eq!(results[3]["level"], "note");
    }

    #[test]
    fn sarif_includes_distinct_rules() {
        let vs = vec![
            Violation::new("J001", Severity::Minor, "A.java", 1, "a"),
            Violation::new("J001", Severity::Minor, "A.java", 2, "b"),
            Violation::new("J002", Severity::Major, "B.java", 1, "c"),
        ];
        let sarif = SarifReporter::build_sarif(&vs);
        let rules = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn sarif_region_endline() {
        let vs = vec![Violation {
            rule_id: "J001".into(),
            severity: Severity::Minor,
            file: "A.java".to_string(),
            line: 8,
            end_line: Some(9),
            message: "x".to_string(),
        }];
        let sarif = SarifReporter::build_sarif(&vs);
        let region = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 8);
        assert_eq!(region["endLine"], 9);
    }

    #[test]
    fn json_report_includes_end_line() {
        let mut v = Violation::new("J001", Severity::Minor, "A.java", 8, "x");
        v.end_line = Some(9);
        let vs = vec![v];
        let mut buf = Vec::new();
        JsonReporter::report_to(&mut buf, &vs, 1, 0, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed["violations"][0]["end_line"], 9);
    }

    #[test]
    fn console_report_shows_line_range() {
        let mut v = Violation::new("J001", Severity::Minor, "A.java", 8, "x");
        v.end_line = Some(9);
        let vs = vec![v];
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &vs, 0, 0, None).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("8-9"), "expected line range 8-9 in: {out}");
    }
}
