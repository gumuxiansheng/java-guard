mod rules;
mod scanner;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use guard_core::reporter::{report, ReportFormat};
use guard_core::rule::{Rule, ViolationCollector};
use java_ast::bridge::{CliParser, JavaParser};

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
            path,
            format,
            output,
            exclude,
            parser_jar,
            java_cmd,
        } => {
            if let Err(e) = run_scan(&path, &format, output.as_deref(), exclude.as_deref(), parser_jar.as_deref(), java_cmd.as_deref()) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        Command::Rules => {
            println!("Built-in rules:");
            for r in rules::builtin_rules() {
                println!("  {} [{}] {}", r.id(), r.severity(), r.description());
            }
        }
        Command::Version => {
            println!("java-guard {}", env!("CARGO_PKG_VERSION"));
        }
    }
}

fn run_scan(
    path: &str,
    format: &str,
    output: Option<&str>,
    exclude: Option<&str>,
    parser_jar: Option<&str>,
    java_cmd: Option<&str>,
) -> anyhow::Result<()> {
    let report_format = ReportFormat::from_str(format)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // 默认排除目录
    let default_excludes = ["target", "build", ".git", "node_modules"];
    let excludes: Vec<&str> = match exclude {
        Some(e) => e.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect(),
        None => default_excludes.to_vec(),
    };

    // 查找 java-parser.jar
    let jar_path = find_parser_jar(parser_jar)?;
    let mut parser_builder = CliParser::new(&jar_path);
    if let Some(cmd) = java_cmd {
        parser_builder = parser_builder.with_java_cmd(cmd);
    }
    let parser = parser_builder;

    // 收集规则
    let rule_list: Vec<Arc<dyn Rule<java_ast::ast::CompilationUnit>>> = rules::builtin_rules();
    let enabled_count = rule_list.iter().filter(|r| r.enabled()).count();

    // 扫描文件
    let root = Path::new(path);
    let scan_result = scanner::scan_java_files(root, &excludes);

    eprintln!("Scanning {} .java files ({} rules enabled)...", scan_result.files.len(), enabled_count);

    // 解析 + 检查
    let mut collector = ViolationCollector::new();
    let mut parsed = 0usize;
    let mut parse_errors = 0usize;

    for file in &scan_result.files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  skip (read error): {} — {e}", file.display());
                parse_errors += 1;
                continue;
            }
        };

        let rel_path = file.strip_prefix(&scan_result.root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");

        match parser.parse(&source, &rel_path) {
            Ok(unit) => {
                for rule in &rule_list {
                    if !rule.enabled() {
                        continue;
                    }
                    let vs = rule.check_unit(&unit);
                    collector.add_all(vs);
                }
                parsed += 1;
            }
            Err(e) => {
                eprintln!("  parse error: {rel_path} — {e}");
                parse_errors += 1;
            }
        }
    }

    eprintln!("Parsed {parsed} files, {parse_errors} errors, {} violations", collector.count());

    // 排序
    collector.sort();

    // 输出报告
    let violations = collector.violations();
    match output {
        Some(out_path) => {
            let mut file = std::fs::File::create(out_path)?;
            report(&report_format, violations)?;
            // 对于文件输出，重新写到文件
            use std::io::Write;
            let content = match report_format {
                ReportFormat::Json => {
                    serde_json::to_string_pretty(violations)?
                }
                ReportFormat::Csv => {
                    let mut s = String::from("rule_id,severity,file,line,end_line,message\n");
                    for v in violations {
                        use std::fmt::Write;
                        writeln!(s, "{},{},{},{},{},\"{}\"",
                            v.rule_id, v.severity, v.file, v.line,
                            v.end_line.map(|e| e.to_string()).unwrap_or_default(),
                            v.message.replace('"', "\"\"")).ok();
                    }
                    s
                }
                _ => {
                    // console / sarif 默认写到文件内容
                    format!("{} violations", violations.len())
                }
            };
            file.write_all(content.as_bytes())?;
            eprintln!("Report written to {out_path}");
        }
        None => {
            report(&report_format, violations)?;
        }
    }

    // 退出码：有 violation 则返回 1（M7 CI gate 细化）
    if !violations.is_empty() {
        std::process::exit(1);
    }

    Ok(())
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

    // 尝试相对路径
    let candidates = [
        PathBuf::from("java-parser/target/java-parser.jar"),
        PathBuf::from("../java-parser/target/java-parser.jar"),
        // CARGO_MANIFEST_DIR 在编译时已知
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
