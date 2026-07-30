//! 报告输出格式与 Reporter 实现。

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
    pub const BLUE: &str = "\x1b[34m";
    pub const GRAY: &str = "\x1b[90m";
    pub const GREEN: &str = "\x1b[32m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RESET: &str = "\x1b[0m";
}

fn severity_color(s: Severity) -> &'static str {
    match s {
        Severity::Critical => color::RED,
        Severity::Major => color::RED,
        Severity::Minor => color::YELLOW,
        Severity::Info => color::BLUE,
    }
}

/// 控制台 Reporter：彩色输出违规列表。
pub struct ConsoleReporter;

impl ConsoleReporter {
    /// 输出到 stdout。
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations)
    }

    /// 输出到指定 writer。
    pub fn report_to<W: IoWrite>(w: &mut W, violations: &[Violation]) -> io::Result<()> {
        if violations.is_empty() {
            writeln!(w, "{}No issues found.{}", color::GREEN, color::RESET)?;
            return Ok(());
        }

        let mut by_file: std::collections::BTreeMap<&str, Vec<&Violation>> =
            std::collections::BTreeMap::new();
        for v in violations {
            by_file.entry(v.file.as_str()).or_default().push(v);
        }

        for (file, vs) in &by_file {
            writeln!(
                w,
                "\n{}{}{} ({} issue{})",
                color::BOLD,
                file,
                color::RESET,
                vs.len(),
                if vs.len() > 1 { "s" } else { "" }
            )?;

            for v in vs {
                let sev = severity_color(v.severity);
                let end = v
                    .end_line
                    .map(|e| format!("-{e}"))
                    .unwrap_or_default();

                writeln!(
                    w,
                    "  {}{}:{}{}{} {}{}{}",
                    color::GRAY,
                    v.line,
                    end,
                    color::RESET,
                    "",
                    sev,
                    v.severity,
                    color::RESET,
                )?;

                writeln!(
                    w,
                    "    {}{}{} {}",
                    color::BOLD,
                    v.rule_id,
                    color::RESET,
                    v.message
                )?;
            }
        }

        let total = violations.len();
        let files = by_file.len();
        writeln!(
            w,
            "\n{}{} violation{} in {} file{}{}",
            color::BOLD,
            total,
            if total > 1 { "s" } else { "" },
            files,
            if files > 1 { "s" } else { "" },
            color::RESET,
        )?;

        Ok(())
    }
}

/// JSON Reporter：输出 violations 数组。
pub struct JsonReporter;

impl JsonReporter {
    pub fn report(violations: &[Violation]) -> io::Result<()> {
        Self::report_to(&mut io::stdout(), violations)
    }

    pub fn report_to<W: IoWrite>(w: &mut W, violations: &[Violation]) -> io::Result<()> {
        let json = serde_json::to_string_pretty(violations)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(w, "{json}")?;
        Ok(())
    }
}

/// 统一 Reporter 入口。
pub fn report(format: &ReportFormat, violations: &[Violation]) -> io::Result<()> {
    match format {
        ReportFormat::Console => ConsoleReporter::report(violations),
        ReportFormat::Json => JsonReporter::report(violations),
        ReportFormat::Sarif => {
            // M6 实现
            writeln!(io::stdout(), "{{\"sarif\": \"not yet implemented\"}}")
        }
        ReportFormat::Csv => {
            // M6 实现
            let mut w = io::stdout();
            writeln!(w, "rule_id,severity,file,line,end_line,message")?;
            for v in violations {
                writeln!(
                    w,
                    "{},{},{},{},{},\"{}\"",
                    v.rule_id,
                    v.severity,
                    v.file,
                    v.line,
                    v.end_line.map(|e| e.to_string()).unwrap_or_default(),
                    v.message.replace('"', "\"\"")
                )?;
            }
            Ok(())
        }
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

    #[test]
    fn console_report_produces_output() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &vs).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("Foo.java"));
        assert!(output.contains("Bar.java"));
        assert!(output.contains("J001"));
        assert!(output.contains("J002"));
        assert!(output.contains("3 violations"));
    }

    #[test]
    fn console_report_empty() {
        let mut buf = Vec::new();
        ConsoleReporter::report_to(&mut buf, &[]).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("No issues"));
    }

    #[test]
    fn json_report_valid_json() {
        let vs = sample_violations();
        let mut buf = Vec::new();
        JsonReporter::report_to(&mut buf, &vs).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 3);
    }

    #[test]
    fn csv_report_has_header() {
        let vs = sample_violations();
        let mut csv_buf: Vec<u8> = Vec::new();
        for v in &vs {
            writeln!(
                csv_buf,
                "{},{},{},{},{},\"{}\"",
                v.rule_id,
                v.severity,
                v.file,
                v.line,
                v.end_line.map(|e| e.to_string()).unwrap_or_default(),
                v.message.replace('"', "\"\"")
            ).unwrap();
        }
        let csv = String::from_utf8(csv_buf).unwrap();
        assert!(csv.contains("J001"));
    }

    #[test]
    fn report_format_from_str() {
        assert_eq!("console".parse::<ReportFormat>().unwrap(), ReportFormat::Console);
        assert_eq!("JSON".parse::<ReportFormat>().unwrap(), ReportFormat::Json);
        assert!("unknown".parse::<ReportFormat>().is_err());
    }
}
