mod adapters;
mod rules;
mod scanner;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use clap::Parser;
use guard_core::gate::{GateConfig, GateResult, SeverityCounts};
use guard_core::git_diff;
use guard_core::reporter::{report_to, ReportFormat};
use guard_core::rule::{Rule, Violation, ViolationCollector};
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

        /// 包含路径白名单（逗号分隔，如 src/main）
        #[clap(short = 'I', long)]
        include: Option<String>,

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

        /// 配置文件路径（默认 java-guard.yml）
        #[clap(long, default_value = "java-guard.yml")]
        config: String,
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
            path, format, output, exclude, include, rules_dir, diff, baseline, gate, gate_config,
            enable, disable, min_severity, parser_jar, java_cmd, config,
        } => {
            if let Err(e) = run_scan(
                &path, &format, output.as_deref(), exclude.as_deref(), include.as_deref(),
                rules_dir.as_deref(), diff.as_deref(), baseline.as_deref(),
                gate, gate_config.as_deref(),
                enable.as_deref(), disable.as_deref(), &min_severity,
                parser_jar.as_deref(), java_cmd.as_deref(), config.as_str(),
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
                    Ok(r) => match r.validate() {
                        Ok(()) => rules.push(r),
                        Err(bad) => eprintln!(
                            "warn: skip rhai rule {}: {}",
                            path.display(),
                            bad.join("; ")
                        ),
                    },
                    Err(e) => eprintln!("warn: skip rhai rule {}: {e}", path.display()),
                }
            }
        }
    }
    Ok(rules)
}

/// 单个文件的解析 + 规则检查结果。
struct FileCheck {
    violations: Vec<Violation>,
    parse_error: bool,
    error_msg: Option<String>,
}

/// 解析单个文件并对所有启用的规则执行检查。
///
/// 设计为线程安全，可在并行线程池中调用：`CliParser` 与 `Rule` 均为 `Sync`，
/// 且 `CliParser::parse` 使用「进程 id + 调用序号」生成唯一临时文件，不会相互冲突。
fn check_one_file(
    file: &Path,
    parser: &CliParser,
    rule_list: &[Arc<dyn Rule<CompilationUnit>>],
    line_filter: &guard_core::git_diff::LineFilter,
    root: &Path,
) -> FileCheck {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            return FileCheck {
                violations: Vec::new(),
                parse_error: true,
                error_msg: Some(format!("skip (read error): {} — {e}", file.display())),
            };
        }
    };

    let rel_path = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");

    match parser.parse(&source, &rel_path) {
        Ok(mut unit) => {
            if unit.source_file.is_empty() {
                unit.source_file = rel_path.clone();
            }
            let mut violations = Vec::new();
            for rule in rule_list {
                if !rule.enabled() {
                    continue;
                }
                let vs = rule.check_unit(&unit);
                let filtered: Vec<_> = if line_filter.is_incremental() {
                    vs.into_iter()
                        .filter(|v| line_filter.allows_range(&rel_path, v.line, v.end_line))
                        .collect()
                } else {
                    vs
                };
                violations.extend(filtered);
            }
            FileCheck {
                violations,
                parse_error: false,
                error_msg: None,
            }
        }
        Err(e) => FileCheck {
            violations: Vec::new(),
            parse_error: true,
            error_msg: Some(format!("parse error: {rel_path} — {e}")),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn run_scan(
    path: &str,
    format: &str,
    output: Option<&str>,
    exclude: Option<&str>,
    include: Option<&str>,
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
    config_path: &str,
) -> anyhow::Result<()> {
    let start = Instant::now();

    // 加载配置文件（如果存在）
    let project_config = load_project_config(config_path)?;
    let report_format = ReportFormat::from_str(format)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // 合并配置文件和 CLI 参数（CLI 优先）
    let enable_str = enable.unwrap_or("");
    let disable_str = disable.unwrap_or("");
    let enable_ids: Vec<String> = if enable_str.is_empty() {
        project_config.rules.enable.clone()
    } else {
        enable_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    let disable_ids: Vec<String> = if disable_str.is_empty() {
        project_config.rules.disable.clone()
    } else {
        disable_str.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    let min_sev: guard_core::rule::Severity = if min_severity.is_empty() {
        project_config.rules.min_severity.as_deref().unwrap_or("info").parse()
            .map_err(|e| anyhow::anyhow!("invalid min_severity: {e}"))?
    } else {
        min_severity.parse()
            .map_err(|e| anyhow::anyhow!("invalid min_severity: {e}"))?
    };

    // 默认排除目录
    let default_excludes = ["target", "build", ".git", "node_modules"];
    let mut excludes: Vec<String> = match exclude {
        Some(e) => e.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        None => default_excludes.iter().map(|s| s.to_string()).collect(),
    };
    // 合并配置文件的 exclude
    excludes.extend(project_config.scan.exclude.iter().cloned());
    let excludes_ref: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();

    // 路径白名单：CLI + 配置文件
    let includes: Vec<String> = match include {
        Some(i) => i.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        None => project_config.scan.include.clone(),
    };
    let includes_ref: Vec<&str> = includes.iter().map(|s| s.as_str()).collect();

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
    rule_list.retain(|r| !disable_ids.iter().any(|d| r.id().0 == *d));
    if !enable_ids.is_empty() {
        rule_list.retain(|r| enable_ids.iter().any(|e| r.id().0 == *e));
    }

    // 规则过滤：min_severity
    rule_list.retain(|r| r.severity() >= min_sev);

    let enabled_count = rule_list.iter().filter(|r| r.enabled()).count();

    // 扫描文件
    let root = Path::new(path);
    let scan_result = scanner::scan_java_files(root, &excludes_ref);

    // 路径白名单过滤
    let scan_files = if includes_ref.is_empty() {
        scan_result.files.clone()
    } else {
        scan_result.files.iter().filter(|f| {
            let f_str = f.to_string_lossy().replace('\\', "/");
            includes_ref.iter().any(|inc| {
                let inc = inc.replace('\\', "/");
                f_str.contains(&inc)
            })
        }).cloned().collect()
    };
    let scan_result = scanner::ScanResult { files: scan_files, root: scan_result.root };

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

    // 解析 + 检查（并行，受 CPU 核数限制；临时文件名已含调用序号，无冲突风险）
    let mut collector = ViolationCollector::new();
    let parsed = std::sync::atomic::AtomicUsize::new(0);
    let parse_errors = std::sync::atomic::AtomicUsize::new(0);
    let parser = Arc::new(parser);

    let results: Vec<FileCheck> = {
        let files = &line_filter.0;
        if files.is_empty() {
            Vec::new()
        } else {
            let n_workers = thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(files.len())
                .min(8)
                .max(1);
            let collected: Mutex<Vec<FileCheck>> = Mutex::new(Vec::with_capacity(files.len()));
            thread::scope(|s| {
                for w in 0..n_workers {
                    let collected = &collected;
                    let parser = &parser;
                    let rule_list = &rule_list;
                    let line_filter = &line_filter.1;
                    let root = &scan_result.root;
                    let parsed = &parsed;
                    let parse_errors = &parse_errors;
                    s.spawn(move || {
                        for idx in (w..files.len()).step_by(n_workers) {
                            let check =
                                check_one_file(&files[idx], parser, rule_list, line_filter, root);
                            if check.parse_error {
                                parse_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                parsed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            if let Some(err) = &check.error_msg {
                                eprintln!("  {err}");
                            }
                            collected.lock().unwrap().push(check);
                        }
                    });
                }
            });
            collected.into_inner().unwrap()
        }
    };

    for check in results {
        collector.add_all(check.violations);
    }

    let parsed = parsed.into_inner();
    let parse_errors = parse_errors.into_inner();

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
        } else if let Some(ref cfg) = project_config.gate {
            cfg.clone()
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

/// 项目级配置文件 java-guard.yml 的模型。
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ProjectConfig {
    /// 规则配置
    rules: RulesConfig,
    /// 扫描配置
    scan: ScanConfig,
    /// gate 配置
    gate: Option<guard_core::gate::GateConfig>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct RulesConfig {
    /// 启用的规则 ID 列表（为空则全部启用）
    enable: Vec<String>,
    /// 禁用的规则 ID 列表
    disable: Vec<String>,
    /// 最低严重级别
    min_severity: Option<String>,
    /// 规则参数
    params: std::collections::BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ScanConfig {
    /// 路径白名单
    include: Vec<String>,
    /// 路径黑名单
    exclude: Vec<String>,
}

/// 加载项目配置文件。文件不存在时返回默认值（不报错）。
fn load_project_config(path: &str) -> anyhow::Result<ProjectConfig> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(ProjectConfig::default());
    }
    let content = std::fs::read_to_string(p)
        .map_err(|e| anyhow::anyhow!("read config {path}: {e}"))?;
    let cfg: ProjectConfig = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("parse config {path}: {e}"))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parse_scan_defaults() {
        let cli = Cli::parse_from(vec!["java-guard", "scan", "."]);
        match cli.command {
            Command::Scan { format, path, .. } => {
                assert_eq!(path, ".");
                assert_eq!(format, "console");
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn cli_parse_scan_json_format_and_gate() {
        let cli = Cli::parse_from(vec!["java-guard", "scan", "src", "-f", "json", "--gate"]);
        match cli.command {
            Command::Scan { format, gate, .. } => {
                assert_eq!(format, "json");
                assert!(gate);
            }
            _ => panic!("expected Scan"),
        }
    }

    #[test]
    fn cli_parse_rules_and_version() {
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "rules"]).command,
            Command::Rules
        ));
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "version"]).command,
            Command::Version
        ));
    }

    #[test]
    fn load_project_config_missing_returns_default() {
        let cfg = load_project_config("__nonexistent_config_12345.yml").unwrap();
        assert!(cfg.rules.enable.is_empty());
        assert!(cfg.rules.disable.is_empty());
        assert!(cfg.scan.include.is_empty());
        assert!(cfg.scan.exclude.is_empty());
        assert!(cfg.gate.is_none());
    }

    #[test]
    fn load_project_config_parses_rules() {
        let dir = std::env::temp_dir().join("javaguard_cfg_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("java-guard.yml");
        let yaml = concat!(
            "rules:\n",
            "  enable: [J001, J003]\n",
            "  disable: [J008]\n",
            "  min_severity: major\n",
            "scan:\n",
            "  include: [src/main]\n",
            "  exclude: [build]\n",
        );
        std::fs::write(&path, yaml).unwrap();
        let cfg = load_project_config(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.rules.enable, vec!["J001".to_string(), "J003".to_string()]);
        assert_eq!(cfg.rules.disable, vec!["J008".to_string()]);
        assert_eq!(cfg.rules.min_severity.as_deref(), Some("major"));
        assert_eq!(cfg.scan.include, vec!["src/main".to_string()]);
        assert_eq!(cfg.scan.exclude, vec!["build".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_parser_jar_explicit_existing() {
        let dir = std::env::temp_dir().join("javaguard_jar_test");
        let _ = std::fs::create_dir_all(&dir);
        let jar = dir.join("java-parser.jar");
        std::fs::write(&jar, b"fake").unwrap();
        let found = find_parser_jar(Some(jar.to_str().unwrap())).unwrap();
        assert_eq!(found, jar);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_parser_jar_explicit_missing_errors() {
        let res = find_parser_jar(Some("/no/such/java-parser.jar"));
        assert!(res.is_err());
    }

    #[test]
    fn load_baseline_parses_known_violations() {
        let dir = std::env::temp_dir().join("javaguard_baseline_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("baseline.json");
        std::fs::write(&path, r#"[{"file":"A.java","line":10,"rule_id":"J001"}]"#).unwrap();
        let set = load_baseline(path.to_str().unwrap()).unwrap();
        assert!(set.contains(&("A.java".to_string(), 10, "J001".to_string())));
        assert!(!set.contains(&("A.java".to_string(), 11, "J001".to_string())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_baseline_invalid_json_errors() {
        let dir = std::env::temp_dir().join("javaguard_baseline_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_baseline(path.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
