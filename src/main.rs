mod adapters;
mod rules;
mod scanner;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use guard_core::gate::{GateConfig, GateResult, SeverityCounts};
use guard_core::git_diff;
use guard_core::reporter::{report_to, ReportFormat};
use guard_core::rule::{Rule, ViolationCollector};
use java_ast::ast::CompilationUnit;
use java_ast::bridge::{CliParser, JavaParser};
use rule_yaml::YamlRuleAdapter;
use rule_rhai::rule::RhaiRule;
use crate::adapters::RhaiRuleAdapter;

#[derive(Parser)]
#[clap(name = "java-guard", version, about = "Lightweight Java static analysis")]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// 扫描 Java 代码
    Scan {
        /// 扫描路径（文件或目录）
        #[clap(default_value = ".")]
        path: String,

        /// 报告格式：console / json / csv / sarif
        #[clap(short = 'f', long, default_value = "console")]
        format: String,

        /// 输出到文件（默认 stdout）
        #[clap(short = 'o', long)]
        output: Option<String>,

        /// 排除的目录名（逗号分隔，默认 target,build,.git,node_modules）
        #[clap(short = 'x', long)]
        exclude: Option<String>,

        /// YAML 规则目录（默认 rules/）
        #[clap(short = 'r', long)]
        rules_dir: Option<String>,

        /// 增量扫描：git diff 范围（如 HEAD~1 或 main...feature）
        #[clap(long)]
        diff: Option<String>,

        /// Baseline 文件（只报告新增违规）
        #[clap(long)]
        baseline: Option<String>,

        /// CI gate 模式（违规超阈值时退出码 1）
        #[clap(long)]
        gate: bool,

        /// Gate 配置文件（YAML）
        #[clap(long)]
        gate_config: Option<String>,

        /// 启用规则（逗号分隔，覆盖默认）
        #[clap(long)]
        enable: Option<String>,

        /// 禁用规则（逗号分隔）
        #[clap(long)]
        disable: Option<String>,

        /// 最低严重级别
        #[clap(long, default_value = "info")]
        min_severity: String,

        /// java-parser.jar 路径
        #[clap(long, env = "JAVAGUARD_PARSER_JAR")]
        parser_jar: Option<String>,

        /// Java 运行时路径
        #[clap(long, env = "JAVA_CMD")]
        java_cmd: Option<String>,
    },
    /// 列出可用规则
    Rules,
    /// 显示版本信息
    Version,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Scan {
            path, format, output, exclude, rules_dir, diff, baseline, gate, gate_config,
            enable, disable, min_severity, parser_jar, java_cmd,
        } => {
            if let Err(e) = run_scan(
                &path, &format, output.as_deref(), exclude.as_deref(),
                rules_dir.as_deref(), diff.as_deref(), baseline.as_deref(),
                gate, gate_config.as_deref(),
                enable.as_deref(), disable.as_deref(), &min_severity,
                parser_jar.as_deref(), java_cmd.as_deref(),
            ) {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
        }
        Command::Rules => {
            println!("Built-in rules:");
            for r in rules::builtin_rules() {
                println!("  {} [{}] {}", r.id(), r.severity(), r.description());
            }
            let yaml_rules = load_yaml_rules(Path::new("rules"));
            for r in &yaml_rules {
                println!("  {} [{}] {} (YAML)", r.id, r.severity, r.title);
            }
            let rhai_dir = Path::new("rules").join("rhai");
            if rhai_dir.is_dir() {
                if let Ok(rhai_rules) = load_rhai_rules(&rhai_dir) {
                    for r in &rhai_rules {
                        println!("  {} [{}] {} (Rhai)", r.id, r.severity, r.title);
                    }
                }
            }
        }
        Command::Version => {
            println!("java-guard {}", env!("CARGO_PKG_VERSION"));
        }
    }
}

fn load_yaml_rules(dir: &Path) -> Vec<rule_yaml::YamlRule> {
    match rule_yaml::load_rule_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("warn: failed to load rules from {}: {e}", dir.display());
            vec![]
        }
    }
}

fn load_rhai_rules(dir: &Path) -> Result<Vec<RhaiRule>, Box<dyn std::error::Error>> {
    let mut rules = Vec::new();
    if !dir.is_dir() {
        return Ok(rules);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "yml" || ext == "yaml" {
                match rule_rhai::rule::load_rhai_rule_file(&path) {
                    Ok(r) => rules.push(r),
                    Err(e) => eprintln!("warn: skip rhai rule {}: {e}", path.display()),
                }
            }
        }
    }
    Ok(rules)
}

#[allow(clippy::too_many_arguments)]
fn run_scan(
    path: &str,
    format: &str,
    output: Option<&str>,
    exclude: Option<&str>,
    rules_dir: Option<&str>,
    diff: Option<&str>,
    baseline: Option<&str>,
    gate: bool,
    gate_config: Option<&str>,
    enable: Option<&str>,
    disable: Option<&str>,
    min_severity: &str,
    parser_jar: Option<&str>,
    java_cmd: Option<&str>,
) -> anyhow::Result<()> {
    let start = Instant::now();
    let report_format = ReportFormat::from_str(format)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // 解析最低严重级别
    let min_sev: guard_core::rule::Severity = min_severity
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid min_severity: {e}"))?;

    // 默认排除目录
    let default_excludes = ["target", "build", ".git", "node_modules"];
    let excludes: Vec<String> = match exclude {
        Some(e) => e.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        None => default_excludes.iter().map(|s| s.to_string()).collect(),
    };
    let excludes_ref: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();

    // 查找 java-parser.jar
    let jar_path = find_parser_jar(parser_jar)?;
    let parser_builder = CliParser::new(&jar_path);
    let mut parser = parser_builder;
    if let Some(cmd) = java_cmd {
        parser = parser.with_java_cmd(cmd);
    }

    // 收集规则
    let mut rule_list: Vec<Arc<dyn Rule<CompilationUnit>>> = rules::builtin_rules();

    let yaml_dir = rules_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules"));

    let yaml_rules = load_yaml_rules(&yaml_dir);
    for yr in yaml_rules {
        rule_list.push(Arc::new(YamlRuleAdapter::new(yr)));
    }

    let rhai_dir = yaml_dir.join("rhai");
    if rhai_dir.is_dir() {
        match load_rhai_rules(&rhai_dir) {
            Ok(rules) => {
                for rr in rules {
                    rule_list.push(Arc::new(RhaiRuleAdapter::new(rr)));
                }
            }
            Err(e) => {
                eprintln!("warn: failed to load rhai rules: {e}");
            }
        }
    }

    // 规则过滤：enable / disable
    if let Some(disable_str) = disable {
        let disabled: Vec<&str> = disable_str.split(',').map(|s| s.trim()).collect();
        rule_list.retain(|r| !disabled.iter().any(|d| r.id().0 == *d));
    }
    if let Some(enable_str) = enable {
        let enabled: Vec<&str> = enable_str.split(',').map(|s| s.trim()).collect();
        rule_list.retain(|r| enabled.iter().any(|e| r.id().0 == *e));
    }

    // 规则过滤：min_severity
    rule_list.retain(|r| r.severity() >= min_sev);

    let enabled_count = rule_list.iter().filter(|r| r.enabled()).count();

    // 扫描文件
    let root = Path::new(path);
    let scan_result = scanner::scan_java_files(root, &excludes_ref);

    // M5: 增量扫描 — git diff 过滤
    let line_filter = if let Some(diff_spec) = diff {
        match git_diff::get_diff(root, diff_spec) {
            Ok(diffs) => {
                let diff_files: std::collections::HashSet<String> =
                    diffs.iter().map(|d| d.path.replace('\\', "/")).collect();
                let filtered: Vec<PathBuf> = scan_result
                    .files
                    .iter()
                    .filter(|f| {
                        let rel = f
                            .strip_prefix(&scan_result.root)
                            .unwrap_or(f)
                            .to_string_lossy()
                            .replace('\\', "/");
                        diff_files.contains(&rel)
                    })
                    .cloned()
                    .collect();
                eprintln!(
                    "Incremental scan: {} of {} files changed (diff: {diff_spec})",
                    filtered.len(),
                    scan_result.files.len()
                );
                let lf = git_diff::LineFilter::from_diffs(&diffs);
                // 返回过滤后的文件列表和行过滤器
                (filtered, lf)
            }
            Err(e) => {
                eprintln!("warn: git diff failed: {e}, falling back to full scan");
                (scan_result.files.clone(), git_diff::LineFilter::all())
            }
        }
    } else {
        (scan_result.files.clone(), git_diff::LineFilter::all())
    };

    eprintln!(
        "Scanning {} .java files ({} rules enabled)...",
        line_filter.0.len(),
        enabled_count
    );

    // 解析 + 检查
    let mut collector = ViolationCollector::new();
    let mut parsed = 0usize;
    let mut parse_errors = 0usize;

    for file in &line_filter.0 {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  skip (read error): {} — {e}", file.display());
                parse_errors += 1;
                continue;
            }
        };

        let rel_path = file
            .strip_prefix(&scan_result.root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");

        match parser.parse(&source, &rel_path) {
            Ok(mut unit) => {
                if unit.source_file.is_empty() {
                    unit.source_file = rel_path.clone();
                }
                for rule in &rule_list {
                    if !rule.enabled() {
                        continue;
                    }
                    let vs = rule.check_unit(&unit);
                    // M5: 行级过滤
                    let filtered: Vec<_> = if line_filter.1.is_incremental() {
                        vs.into_iter()
                            .filter(|v| line_filter.1.allows(&rel_path, v.line))
                            .collect()
                    } else {
                        vs
                    };
                    collector.add_all(filtered);
                }
                parsed += 1;
            }
            Err(e) => {
                eprintln!("  parse error: {rel_path} — {e}");
                parse_errors += 1;
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    eprintln!(
        "Parsed {parsed} files, {parse_errors} errors, {} violations",
        collector.count()
    );

    // M5: Baseline 过滤
    let violations: Vec<_> = if let Some(baseline_path) = baseline {
        match load_baseline(baseline_path) {
            Ok(baseline_set) => {
                let before = collector.count();
                let filtered: Vec<_> = collector
                    .violations()
                    .iter()
                    .filter(|v| !baseline_set.contains(&(v.file.clone(), v.line, v.rule_id.to_string())))
                    .cloned()
                    .collect();
                eprintln!(
                    "Baseline: {} of {} violations are new",
                    filtered.len(),
                    before
                );
                filtered
            }
            Err(e) => {
                eprintln!("warn: failed to load baseline: {e}");
                collector.violations().to_vec()
            }
        }
    } else {
        collector.violations().to_vec()
    };

    // 排序
    let mut violations = violations;
    violations.sort_by(|a, b| {
        a.file.cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.rule_id.cmp(&b.rule_id))
    });

    // 输出报告
    match output {
        Some(out_path) => {
            let mut file = std::fs::File::create(out_path)?;
            report_to(
                &report_format,
                &mut file,
                &violations,
                parsed,
                parse_errors,
                Some(duration_ms),
            )?;
            eprintln!("Report written to {out_path}");
        }
        None => {
            report_to(
                &report_format,
                &mut std::io::stdout(),
                &violations,
                parsed,
                parse_errors,
                Some(duration_ms),
            )?;
        }
    }

    // M7: CI Gate 检查
    if gate {
        let gate_cfg = if let Some(cfg_path) = gate_config {
            let yaml = std::fs::read_to_string(cfg_path)?;
            GateConfig::from_yaml(&yaml)?
        } else {
            GateConfig::default()
        };
        let counts = SeverityCounts::from_violations(&violations);
        match gate_cfg.check(&counts) {
            GateResult::Pass => {
                eprintln!("Gate: PASS");
                std::process::exit(0);
            }
            GateResult::Fail(reasons) => {
                eprintln!("Gate: FAIL");
                for r in &reasons {
                    eprintln!("  - {r}");
                }
                std::process::exit(1);
            }
        }
    }

    // 默认退出码：有 violation 则返回 1
    if !violations.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

/// 加载 baseline 文件（JSON 格式，包含已知的违规列表）。
fn load_baseline(path: &str) -> anyhow::Result<std::collections::HashSet<(String, usize, String)>> {
    let content = std::fs::read_to_string(path)?;
    let baseline: Vec<serde_json::Value> = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("parse baseline JSON: {e}"))?;

    let mut set = std::collections::HashSet::new();
    for v in &baseline {
        let file = v.get("file").and_then(|f| f.as_str()).unwrap_or("");
        let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
        let rule_id = v.get("rule_id").and_then(|r| r.as_str()).unwrap_or("");
        set.insert((file.to_string(), line, rule_id.to_string()));
    }

    Ok(set)
}

/// 查找 java-parser.jar。
fn find_parser_jar(explicit: Option<&str>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(anyhow::anyhow!("parser jar not found: {p}"));
    }

    let candidates = [
        PathBuf::from("java-parser/target/java-parser.jar"),
        PathBuf::from("../java-parser/target/java-parser.jar"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("java-parser/target/java-parser.jar"),
    ];

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }

    Err(anyhow::anyhow!(
        "java-parser.jar not found. Set --parser-jar or JAVAGUARD_PARSER_JAR env."
    ))
}
