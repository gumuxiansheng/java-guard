mod adapters;
mod rulelock;
mod rulepack;
mod rules;
mod scanner;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::adapters::RhaiRuleAdapter;
use clap::Parser;
use guard_core::gate::{GateConfig, GateResult, SeverityCounts};
use guard_core::git_diff;
use guard_core::reporter::{report_to, ReportFormat};
use guard_core::rule::{Rule, RuleId, Violation, ViolationCollector};
use java_ast::ast::CompilationUnit;
use java_ast::bridge::{CliParser, JavaParser, LazyParser};
use java_ast::AstCache;
use java_ast::ParseError;
use rule_rhai::rule::RhaiRule;
use rule_yaml::YamlRuleAdapter;

#[derive(Parser)]
#[clap(
    name = "java-guard",
    version,
    about = "Lightweight Java static analysis — lightweight, fast, zero-config",
    long_about = "JavaGuard — 轻量级 Java 静态分析工具\n\
\n\
A lightweight static analysis tool for Java code quality and bug detection.\n\
Built-in rules cover empty catch blocks (J008), infinite loops (J009), naming\n\
conventions, wildcard imports, System.out usage, and more. Custom rules can\n\
be written in YAML (declarative) or Rhai (scripted).\n\
\n\
Features:\n\
  • 8+ built-in rules (Rust / YAML / Rhai)\n\
  • Multi-encoding support (auto-detect BOM/UTF-8/GBK/Shift-JIS)\n\
  • Incremental scan via git diff + baseline filtering\n\
  • CI gate mode with severity thresholds\n\
  • Console / JSON / SARIF / CSV report formats\n\
\n\
Quick start:\n\
  java-guard scan .                   # Scan current directory\n\
  java-guard scan src/main -f json    # JSON report for src/main\n\
  java-guard scan . --gate            # CI gate mode (exit 1 on violations)\n\
\n\
Documentation: https://github.com/javaguard/java-guard\n"
)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
// Scan 子命令携带十余个 CLI 字段，与其他单元变体体积悬殊属预期（clippy::large_enum_variant）。
#[allow(clippy::large_enum_variant)]
enum Command {
    /// 扫描 Java 代码，检测代码质量问题与潜在 bug
    ///
    /// 递归扫描指定路径下所有 .java 文件，使用内置规则和自定义规则
    /// 进行静态分析，输出违规报告。支持增量扫描、CI gate、多种报告格式。
    #[clap(
        verbatim_doc_comment,
        after_help = "Examples:\n  java-guard scan .                      # Scan current directory\n  java-guard scan src/main -f json -o report.json\n  java-guard scan . --diff HEAD~1         # Only scan changed files\n  java-guard scan . --gate --gate-config gate.yml\n  java-guard scan . --encoding gbk       # Specify source encoding\n  java-guard scan . --enable J008,J009 --disable J003\n  java-guard scan . --rules-file javaguard.rules.toml\n"
    )]
    Scan {
        /// 扫描路径（文件或目录，默认当前目录）
        #[clap(default_value = ".")]
        path: String,

        /// 报告格式：console（终端彩色）/ json / csv / sarif（SARIF 2.1.0）
        #[clap(short = 'f', long, default_value = "console")]
        format: String,

        /// 输出到文件（不指定则输出到 stdout）
        #[clap(short = 'o', long)]
        output: Option<String>,

        /// 排除目录名（逗号分隔，默认 target,build,.git,node_modules）
        #[clap(short = 'x', long)]
        exclude: Option<String>,

        /// 路径白名单（逗号分隔，只扫描匹配路径，如 src/main,src/test）
        #[clap(short = 'I', long)]
        include: Option<String>,

        /// 规则配置文件路径（TOML，含规则元数据和脚本路径；覆盖配置文件中的 rules_file）
        #[clap(short = 'r', long)]
        rules_file: Option<String>,

        /// 增量扫描：只检查 git diff 变更的文件（如 HEAD~1 或 main...feature）
        #[clap(long)]
        diff: Option<String>,

        /// 语义对比模式：解析旧版本文件（git show）并做违规集合差（需配合 --diff）
        #[clap(long)]
        semantic_diff: bool,

        /// 把当前扫描结果导出为 baseline JSON 文件（供后续 --baseline 使用）
        #[clap(long)]
        baseline_out: Option<String>,

        /// Baseline JSON 文件：只报告 baseline 之外的新增违规
        #[clap(long)]
        baseline: Option<String>,

        /// Baseline 匹配容差：同一 (文件, 规则) 的行号差不超过此值的违规视为已知违规（抗行号漂移）
        #[clap(long, default_value = "5")]
        baseline_tolerance: usize,

        /// CI gate 模式：违规超过阈值时退出码 1（配合 --gate-config）
        #[clap(long)]
        gate: bool,

        /// Gate 配置文件（YAML，定义 max_critical/max_major/max_minor 阈值）
        #[clap(long)]
        gate_config: Option<String>,

        /// 只启用指定规则（逗号分隔 ID，如 J008,J009，覆盖默认全启用）
        #[clap(long)]
        enable: Option<String>,

        /// 禁用指定规则（逗号分隔 ID，如 J003）
        #[clap(long)]
        disable: Option<String>,

        /// 最低严重级别：info / minor / major / critical（低于此级别的违规不报告）
        #[clap(long, default_value = "info")]
        min_severity: String,

        /// java-parser.jar 路径（不指定则自动查找）
        #[clap(long, env = "JAVAGUARD_PARSER_JAR")]
        parser_jar: Option<String>,

        /// Java 运行时路径（默认 java，可指向 jdk-17/bin/java）
        #[clap(long, env = "JAVA_CMD")]
        java_cmd: Option<String>,

        /// 项目配置文件路径（TOML，含 rules/scan/gate 配置）
        #[clap(long, default_value = "java-guard.toml")]
        config: String,

        /// 源文件编码：auto（自动探测 BOM→UTF-8→GBK→Shift-JIS）/ utf-8 / gbk / shift-jis 等
        #[clap(long, default_value = "auto")]
        encoding: String,

        /// 禁用 AST 解析缓存（默认启用，缓存目录 .java-guard-cache/）
        #[clap(long)]
        no_cache: bool,

        /// 输出各阶段耗时分解到 stderr（性能定位用：rules-load/traverse/diff/check/report）
        #[clap(long)]
        timings: bool,

        /// 严格按 javaguard.lock 校验规则包：锁缺失、版本不符、checksum 不符均报错（CI 推荐）
        #[clap(long, conflicts_with = "no_lock")]
        locked: bool,

        /// 跳过 javaguard.lock 校验（临时绕过；不推荐用于 CI）
        #[clap(long)]
        no_lock: bool,
    },
    /// 列出规则，或管理规则包锁文件（lock / verify）
    Rules {
        #[clap(subcommand)]
        action: Option<RulesAction>,
    },
    /// 显示版本信息和构建详情
    Version,
}

/// `java-guard rules` 的子命令；不指定时列出规则。
#[derive(clap::Subcommand)]
enum RulesAction {
    /// 按当前解析结果生成 / 更新 javaguard.lock
    Lock,
    /// 校验 javaguard.lock 与当前规则包是否一致（不执行检查）
    Verify {
        /// 严格模式：额外比对 checksum，并要求锁文件存在且不含多余条目
        #[clap(long)]
        locked: bool,
    },
    /// 注册一个本地规则包到 java-guard.toml 的 [rule_packs].packs
    ///
    /// 先完整走一遍解析链路校验（清单 / 版本契约 / 脚本逃逸），通过后写入配置
    /// 并自动刷新 javaguard.lock。
    Add {
        /// 包名（须与包清单 [pack].name 一致）
        name: String,
        /// 包根目录（含 rules-pack.toml；相对 java-guard.toml 所在目录解析）
        #[clap(long)]
        path: String,
        /// 要求的版本（可选；写入后与包清单 [pack].version 强一致校验）
        #[clap(long)]
        version: Option<String>,
    },
    /// 把当前生效的规则包复制到 <config>/vendor/rules/（入库存档，供离线复现）
    ///
    /// 并确保 java-guard.toml 的 [rule_packs].search_paths 首位包含 "vendor/rules"。
    Vendor,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Scan {
            path,
            format,
            output,
            exclude,
            include,
            rules_file,
            diff,
            semantic_diff,
            baseline_out,
            baseline,
            baseline_tolerance,
            gate,
            gate_config,
            enable,
            disable,
            min_severity,
            parser_jar,
            java_cmd,
            config,
            encoding,
            no_cache,
            timings,
            locked,
            no_lock,
        } => {
            if let Err(e) = run_scan(
                &path,
                &format,
                output.as_deref(),
                exclude.as_deref(),
                include.as_deref(),
                rules_file.as_deref(),
                diff.as_deref(),
                semantic_diff,
                baseline_out.as_deref(),
                baseline.as_deref(),
                baseline_tolerance,
                gate,
                gate_config.as_deref(),
                enable.as_deref(),
                disable.as_deref(),
                &min_severity,
                parser_jar.as_deref(),
                java_cmd.as_deref(),
                config.as_str(),
                encoding.as_str(),
                no_cache,
                timings,
                locked,
                no_lock,
            ) {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
        }
        Command::Rules { action } => {
            let result = match action {
                None => rules_list(),
                Some(RulesAction::Lock) => rules_lock(),
                Some(RulesAction::Verify { locked }) => rules_verify(locked),
                Some(RulesAction::Add { name, path, version }) => {
                    rules_add(&name, &path, version.as_deref())
                }
                Some(RulesAction::Vendor) => rules_vendor(),
            };
            if let Err(e) = result {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
        }
        Command::Version => {
            println!("java-guard {}", env!("CARGO_PKG_VERSION"));
        }
    }
}

/// 加载 java-guard.toml 并解析规则包，返回 `(合并结果, 配置文件目录)`。
///
/// `rules` 子命令与 `scan` 共用同一条解析链路（规则包 + 本地规则 + overrides）。
fn load_merged_rules() -> anyhow::Result<(rulepack::MergedRules, PathBuf)> {
    let project_config = load_project_config("java-guard.toml")?;
    let config_dir = Path::new("java-guard.toml")
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let rules_file_path = project_config
        .rules
        .rules_file
        .as_deref()
        .unwrap_or("javaguard.rules.toml");
    let merged = rulepack::resolve_and_merge(
        &project_config.rule_packs,
        Path::new(rules_file_path),
        &config_dir,
    )?;
    Ok((merged, config_dir))
}

/// `java-guard rules`：列出生效规则（规则包 + 本地规则合并后的结果）。
fn rules_list() -> anyhow::Result<()> {
    let (merged, _config_dir) = load_merged_rules()?;
    for w in &merged.warnings {
        eprintln!("warn: failed to load rules file: {w}");
    }
    for n in &merged.notes {
        eprintln!("note: {n}");
    }
    if merged.rules.is_empty() {
        // 回退：扫描 rules/ 目录（兼容未使用规则文件的旧项目）
        println!("Rules (from rules/ directory):");
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
        return Ok(());
    }
    if !merged.packs.is_empty() {
        println!("Rule packs:");
        for p in &merged.packs {
            let ns = p.namespace.as_deref().unwrap_or("-");
            println!(
                "  {}@{} (api_version={}, namespace={}, {} rules) — {}",
                p.name,
                p.version,
                p.api_version,
                ns,
                p.rule_count,
                p.root.display()
            );
        }
    }
    println!("Rules:");
    println!(
        "{:<20} {:<28} {:<10} {:<8} {:<18} Description",
        "ID", "Name", "Group", "Severity", "Pack"
    );
    println!("{}", "-".repeat(112));
    for (i, entry) in merged.rules.iter().enumerate() {
        let group = entry.group.as_deref().unwrap_or("-");
        let desc = entry.description.as_deref().unwrap_or("");
        let enabled_mark = if entry.enabled { "" } else { " [disabled]" };
        // 来源包：`pack 'rules-core'` -> `rules-core`；项目本地规则显示为 `(local)`
        let pack = merged
            .sources
            .get(i)
            .map(|s| {
                if s == "project rules" {
                    "(local)".to_string()
                } else {
                    s.trim_start_matches("pack '")
                        .trim_end_matches('\'')
                        .to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<20} {:<28} {:<10} {:<8} {:<18} {}{}",
            entry.id, entry.name, group, entry.severity, pack, desc, enabled_mark
        );
    }
    Ok(())
}

/// `java-guard rules lock`：按当前解析结果生成 / 更新 javaguard.lock。
fn rules_lock() -> anyhow::Result<()> {
    let (merged, config_dir) = load_merged_rules()?;
    if merged.packs.is_empty() {
        println!("No rule packs configured; nothing to lock.");
        return Ok(());
    }
    let lock = rulelock::build_lock(&merged.packs, &config_dir)?;
    let path = rulelock::write_lock(&lock, &config_dir)?;
    println!("Wrote {}", path.display());
    for p in &lock.packs {
        println!("  {:<24} {:<10} {}", p.name, p.version, p.source);
    }
    Ok(())
}

/// `java-guard rules verify`：校验锁文件与当前规则包是否一致。
fn rules_verify(locked: bool) -> anyhow::Result<()> {
    let (merged, config_dir) = load_merged_rules()?;
    let mode = if locked {
        rulelock::LockMode::Strict
    } else {
        rulelock::LockMode::Auto
    };
    for note in rulelock::verify_lock(&merged.packs, &config_dir, mode)? {
        println!("note: {note}");
    }
    for note in &merged.notes {
        println!("note: {note}");
    }
    println!(
        "OK: {} rule pack(s), {} rule(s) effective",
        merged.packs.len(),
        merged.rules.len()
    );
    Ok(())
}

/// `java-guard rules add <name> --path <dir> [--version <v>]`：
/// 校验规则包可加载后，把引用写入 java-guard.toml 的 [rule_packs].packs 并刷新锁文件。
fn rules_add(name: &str, path: &str, version: Option<&str>) -> anyhow::Result<()> {
    let config_path = PathBuf::from("java-guard.toml");
    let project_config = load_project_config("java-guard.toml")?;
    let config_dir = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let rules_file_path = project_config
        .rules
        .rules_file
        .as_deref()
        .unwrap_or("javaguard.rules.toml");

    if project_config
        .rule_packs
        .packs
        .iter()
        .any(|p| p.name == name)
    {
        anyhow::bail!(
            "rule pack '{name}' is already registered in {}; remove the [[rule_packs.packs]] \
             entry first if you want to re-add it",
            config_path.display()
        );
    }

    // 校验：把新包追加进临时配置后完整走一遍解析链路
    // （清单合法性 / api_version / engine 版本契约 / 版本一致 / 脚本逃逸全部过闸）。
    let mut probe = project_config.rule_packs.clone();
    probe.packs.push(rulepack::PackRef {
        name: name.to_string(),
        version: version.map(str::to_string),
        path: Some(PathBuf::from(path)),
    });
    let merged = rulepack::resolve_and_merge(&probe, Path::new(rules_file_path), &config_dir)?;
    let added = merged
        .packs
        .last()
        .expect("the pack pushed above must resolve");

    append_pack_to_config(&config_path, name, version, path)?;
    println!(
        "Added rule pack '{}' v{} ({})",
        added.name,
        added.version,
        added.root.display()
    );
    rules_lock()?;
    println!("hint: its rules default to the pack's own enabled state; turn them on per project via [[rule_packs.overrides]], e.g.");
    println!("  [[rule_packs.overrides]]");
    println!("  id = \"<rule-id>\"");
    println!("  enabled = true");
    Ok(())
}

/// `java-guard rules vendor`：把当前生效的规则包复制到 <config>/vendor/rules/<name>/
/// 并确保 [rule_packs].search_paths 首位包含 "vendor/rules"（离线 / 入库复现）。
fn rules_vendor() -> anyhow::Result<()> {
    let (merged, config_dir) = load_merged_rules()?;
    if merged.packs.is_empty() {
        println!("No rule packs configured; nothing to vendor.");
        return Ok(());
    }
    let vendored = config_dir.join("vendor").join("rules");
    std::fs::create_dir_all(&vendored)?;
    for p in &merged.packs {
        let dst = vendored.join(&p.name);
        copy_dir_recursive(&p.root, &dst).map_err(|e| {
            anyhow::anyhow!(
                "failed to vendor pack '{}' -> '{}': {e}",
                p.name,
                dst.display()
            )
        })?;
        println!(
            "vendored {:<22} v{:<8} -> {}",
            p.name,
            p.version,
            dst.display()
        );
    }
    ensure_search_path_first(&PathBuf::from("java-guard.toml"), "vendor/rules")?;
    println!(
        "note: \"vendor/rules\" is now first in [rule_packs].search_paths of java-guard.toml; \
         packs registered with an explicit `path` keep resolving there — drop the `path` line \
         to resolve from vendor/rules instead."
    );
    Ok(())
}

/// 递归复制目录（vendor 用；目标已存在时覆盖同名文件）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 把一条 `[[rule_packs.packs]]` 引用文本化追加到配置文件末尾（保留用户原有格式）。
///
/// 配置文件不存在时创建最小骨架。路径统一写正斜杠（TOML 字符串免转义，Windows 亦识别）。
fn append_pack_to_config(
    config_path: &Path,
    name: &str,
    version: Option<&str>,
    path: &str,
) -> anyhow::Result<()> {
    let mut content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::from("# java-guard.toml（由 `java-guard rules add` 生成）\n")
    };
    if config_registers_pack(&content, name) {
        anyhow::bail!(
            "{} already registers rule pack '{name}'",
            config_path.display()
        );
    }
    content.push_str("\n[[rule_packs.packs]]\n");
    content.push_str(&format!("name = \"{name}\"\n"));
    if let Some(v) = version {
        content.push_str(&format!("version = \"{v}\"\n"));
    }
    content.push_str(&format!("path = \"{}\"\n", path.replace('\\', "/")));
    std::fs::write(config_path, content)?;
    Ok(())
}

/// 行级检测：配置文本的 `[[rule_packs.packs]]` 段里是否已注册同名包。
fn config_registers_pack(content: &str, name: &str) -> bool {
    let want = format!("name = \"{name}\"");
    let mut in_packs_table = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("[[") {
            in_packs_table = t == "[[rule_packs.packs]]";
        } else if t.starts_with('[') {
            in_packs_table = false;
        } else if in_packs_table && (t == want || t.replace('\'', "\"") == want) {
            return true;
        }
    }
    false
}

/// 确保 java-guard.toml 的 `[rule_packs].search_paths` 数组首位是给定路径
/// （已存在则移到首位；无该键则在 `[rule_packs]` 段内插入；无段则追加段）。
fn ensure_search_path_first(config_path: &Path, want: &str) -> anyhow::Result<()> {
    let mut lines: Vec<String> = if config_path.exists() {
        std::fs::read_to_string(config_path)?
            .lines()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };

    let quoted = format!("\"{want}\"");
    match lines.iter().position(|l| l.trim() == "[rule_packs]") {
        Some(h) => {
            // 段范围到下一个表头（顶层表或数组表均终止）
            let end = lines[h + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map(|i| i + h + 1)
                .unwrap_or(lines.len());
            match (h..end).find(|&i| lines[i].trim_start().starts_with("search_paths")) {
                Some(i) => lines[i] = insert_search_path_first(&lines[i], want),
                None => lines.insert(h + 1, format!("search_paths = [{quoted}]")),
            }
        }
        None => {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("[rule_packs]".into());
            lines.push(format!("search_paths = [{quoted}]"));
        }
    }
    // 末行换行 + 防重复换行
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(config_path, out)?;
    Ok(())
}

/// 把单行 `search_paths = [...]` 数组变换为以 `want` 为首位的等价行。
fn insert_search_path_first(line: &str, want: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let eq = line.find('=').expect("search_paths line must contain '='");
    let arr = line[eq + 1..].trim();
    let inner = arr.trim_start_matches('[').trim_end_matches(']').trim();
    let mut items: Vec<String> = if inner.is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    let quoted = format!("\"{want}\"");
    items.retain(|s| *s != quoted);
    items.insert(0, quoted);
    format!("{indent}search_paths = [{}]", items.join(", "))
}

/// 查找回退用的 rules/ 目录：exe 同级（部署布局 A）→ exe 上级（deploy/bin 布局 B）→ cwd（源码布局 C）。
/// 返回第一个存在的候选目录；都不存在时返回 None。
fn find_fallback_rules_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // 布局 A：exe 与 rules/ 平级
            candidates.push(parent.join("rules"));
            // 布局 B：exe 在 bin/ 下，rules/ 在其上级（deploy 标准布局）
            if let Some(grand) = parent.parent() {
                candidates.push(grand.join("rules"));
            }
        }
    }
    // 布局 C：源码/开发场景（cwd/rules）
    candidates.push(PathBuf::from("rules"));
    candidates.into_iter().find(|d| d.is_dir())
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
            if ext == "yml" || ext == "yaml" || ext == "rhai" {
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

/// 每文件阶段耗时累计（跨 worker 求和，`--timings` 时输出）。
///
/// read/parse/rules 三项之和 ≥ check 阶段墙钟时间（并行 worker 各自累加）。
#[derive(Default)]
struct Timings {
    read_ms: std::sync::atomic::AtomicU64,
    parse_ms: std::sync::atomic::AtomicU64,
    rules_ms: std::sync::atomic::AtomicU64,
}

impl Timings {
    fn add(counter: &std::sync::atomic::AtomicU64, started: Instant) {
        counter.fetch_add(
            started.elapsed().as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// 编码探测：把原始字节按指定编码解码为 UTF-8 字符串（永不失败，回退链完备）。
///
/// 支持的编码：
/// - `auto`：自动探测（BOM → UTF-8 尝试 → GBK fallback）
/// - `utf-8` / `utf8`：UTF-8
/// - `gbk` / `gb2312` / `gb18030`：中文编码
/// - `shift-jis` / `shift_jis` / `sjis`：日文编码
/// - `latin1` / `iso-8859-1`：西欧编码
/// - 其他 encoding_rs 支持的编码名称
fn decode_source_bytes(bytes: &[u8], encoding: &str) -> String {
    let enc = encoding.to_ascii_lowercase();

    if enc == "auto" {
        // 1. BOM 探测
        if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
            return String::from_utf8_lossy(&bytes[3..]).into_owned();
        }
        // 2. 尝试 UTF-8
        if let Ok(s) = std::str::from_utf8(bytes) {
            return s.to_string();
        }
        // 3. fallback 到 GBK（中文项目最常见）
        let (cow, _, had_errors) = encoding_rs::GBK.decode(bytes);
        if had_errors {
            // GBK 也有问题，尝试 Shift-JIS，最后 Latin1（Latin1 不会失败）
            let (cow2, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
            if cow2.is_empty() || cow2.chars().any(|c| c == '\u{FFFD}') {
                let (cow3, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
                return cow3.into_owned();
            }
            return cow2.into_owned();
        }
        return cow.into_owned();
    }

    // 指定编码
    let decoder = match enc.as_str() {
        "utf-8" | "utf8" => encoding_rs::UTF_8,
        "gbk" | "gb2312" => encoding_rs::GBK,
        "gb18030" => encoding_rs::GB18030,
        "shift-jis" | "shift_jis" | "sjis" => encoding_rs::SHIFT_JIS,
        "latin1" | "iso-8859-1" | "iso8859-1" => encoding_rs::WINDOWS_1252,
        "big5" => encoding_rs::BIG5,
        "euc-kr" | "euc_kr" | "korean" => encoding_rs::EUC_KR,
        _ => {
            // 尝试用 encoding_rs 的名称查找
            let (cow, _, _) = encoding_rs::Encoding::for_label(enc.as_bytes())
                .unwrap_or(encoding_rs::UTF_8)
                .decode(bytes);
            return cow.into_owned();
        }
    };
    let (cow, _, _) = decoder.decode(bytes);
    cow.into_owned()
}

/// 读取文件字节并按指定编码解码（失败仅发生在读文件阶段）。
fn read_source_file(path: &Path, encoding: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(decode_source_bytes(&bytes, encoding))
}

/// 对所有传入的规则执行检查，返回原始违规列表（不过滤）。
///
/// 规则的启用/禁用与 enable/disable 覆盖已在调用方（`run_scan` 的列表层过滤）完成，
/// 此处只负责执行，不再二次判断 `enabled()`——否则 `--enable` 对 `enabled=false`
/// 的规则会失效（被选中进入列表却在闸门处被跳过）。
fn run_rules(
    unit: &CompilationUnit,
    rule_list: &[Arc<dyn Rule<CompilationUnit>>],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rule in rule_list {
        violations.extend(rule.check_unit(unit));
    }
    violations
}

/// 解析源码（带 AST 缓存）：命中缓存直接反序列化，跳过 JVM；未命中解析后回填。
/// 解析（带 AST 缓存），并把**源码文本**回填到编译单元。
///
/// 回填必须在这里做，而不是在调用点：本函数是「源码字节 → CompilationUnit」
/// 的唯一收敛点，`check_one_file`（当前版本）与 `collect_old_violations`
/// （`--semantic-diff` 的旧版本）都经由此处，集中回填可保证两边语义一致。
fn parse_with_cache(
    parser: &dyn JavaParser,
    cache: &AstCache,
    source: &str,
    filename: &str,
) -> Result<CompilationUnit, ParseError> {
    if let Some(json) = cache.get(source) {
        if let Ok(mut unit) = serde_json::from_str::<CompilationUnit>(&json) {
            unit.source_file = filename.to_string();
            unit.raw_json = json;
            unit.attach_source(source);
            return Ok(unit);
        }
        // 反序列化失败 = 缓存损坏 → 落到下面走真实解析
    }
    let mut unit = parser.parse(source, filename)?;
    cache.put(source, &unit.raw_json);
    unit.attach_source(source);
    Ok(unit)
}

/// 解析旧版本源码并收集违规（语义对比模式用）。
///
/// - 旧版本文件不存在（新增文件）→ 空列表
/// - 旧版本解析失败 → 空列表 + stderr 警告（不影响新版本检查）
fn collect_old_violations(
    rel_path: &str,
    old_ref: &str,
    parser: &dyn JavaParser,
    cache: &AstCache,
    rule_list: &[Arc<dyn Rule<CompilationUnit>>],
    root: &Path,
    encoding: &str,
) -> Vec<Violation> {
    let bytes = match git_diff::read_old_source(root, Some(old_ref), rel_path) {
        Ok(Some(b)) => b,
        Ok(None) => return Vec::new(), // 新增文件：旧版本不存在
        Err(e) => {
            eprintln!("  warn: {e}");
            return Vec::new();
        }
    };
    let source = decode_source_bytes(&bytes, encoding);
    match parse_with_cache(parser, cache, &source, rel_path) {
        Ok(mut unit) => {
            if unit.source_file.is_empty() {
                unit.source_file = rel_path.to_string();
            }
            run_rules(&unit, rule_list)
        }
        Err(e) => {
            eprintln!("  warn: old version parse error: {rel_path} — {e}");
            Vec::new()
        }
    }
}

/// 语义对比：新版本违规 − 旧版本违规。
///
/// 旧违规的行号先经 `mapper`（基于 diff hunk 的新旧行号区间）精确翻译为
/// 新文件行号，再做集合差；被删除的行（翻译为 None）不参与匹配。
fn semantic_difference(
    new_violations: Vec<Violation>,
    old_violations: Vec<Violation>,
    mapper: &git_diff::LineMapper,
) -> Vec<Violation> {
    let mut known: std::collections::HashSet<(String, String, usize)> =
        std::collections::HashSet::new();
    for v in old_violations {
        if let Some(new_line) = mapper.translate(&v.file, v.line) {
            known.insert((v.file, v.rule_id.0, new_line));
        }
    }
    new_violations
        .into_iter()
        .filter(|v| !known.contains(&(v.file.clone(), v.rule_id.0.clone(), v.line)))
        .collect()
}

/// 解析单个文件并对所有启用的规则执行检查。
///
/// 设计为线程安全，可在并行线程池中调用：`CliParser` 与 `Rule` 均为 `Sync`，
/// 且 `CliParser::parse` 使用「进程 id + 调用序号」生成唯一临时文件，不会相互冲突。
///
/// `semantic_old_ref` 为 `Some` 时进入语义对比模式：解析旧版本（git show）并做
/// 违规集合差；否则按 `line_filter` 做行级过滤（增量模式）。
#[allow(clippy::too_many_arguments)]
fn check_one_file(
    file: &Path,
    parser: &dyn JavaParser,
    cache: &AstCache,
    rule_list: &[Arc<dyn Rule<CompilationUnit>>],
    line_filter: &guard_core::git_diff::LineFilter,
    mapper: Option<&git_diff::LineMapper>,
    semantic_old_ref: Option<&str>,
    root: &Path,
    encoding: &str,
    timings: &Timings,
) -> FileCheck {
    let t_read = Instant::now();
    let source = match read_source_file(file, encoding) {
        Ok(s) => s,
        Err(e) => {
            return FileCheck {
                violations: Vec::new(),
                parse_error: true,
                error_msg: Some(format!("skip (read error): {} — {e}", file.display())),
            };
        }
    };
    Timings::add(&timings.read_ms, t_read);

    let rel_path = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");

    let t_parse = Instant::now();
    match parse_with_cache(parser, cache, &source, &rel_path) {
        Ok(mut unit) => {
            Timings::add(&timings.parse_ms, t_parse);
            if unit.source_file.is_empty() {
                unit.source_file = rel_path.clone();
            }
            let t_rules = Instant::now();
            let mut violations = run_rules(&unit, rule_list);
            if let Some(old_ref) = semantic_old_ref {
                let old_violations = collect_old_violations(
                    &rel_path, old_ref, parser, cache, rule_list, root, encoding,
                );
                violations = match mapper {
                    Some(m) => semantic_difference(violations, old_violations, m),
                    None => violations,
                };
            } else if line_filter.is_incremental() {
                let lf = line_filter;
                violations.retain(|v| {
                    lf.allows_policy(&rel_path, v.line, v.end_line, rule_policy(v, rule_list))
                });
            }
            Timings::add(&timings.rules_ms, t_rules);
            FileCheck {
                violations,
                parse_error: false,
                error_msg: None,
            }
        }
        Err(e) => {
            Timings::add(&timings.parse_ms, t_parse);
            FileCheck {
                violations: Vec::new(),
                parse_error: true,
                error_msg: Some(format!("parse error: {rel_path} — {e}")),
            }
        }
    }
}

/// 找到某违规所属规则的 span_policy（按 id 匹配；找不到时用默认 Anchor）。
fn rule_policy(
    v: &Violation,
    rule_list: &[Arc<dyn Rule<CompilationUnit>>],
) -> guard_core::rule::SpanPolicy {
    rule_list
        .iter()
        .find(|r| r.id().0 == v.rule_id.0)
        .map(|r| r.span_policy())
        .unwrap_or(guard_core::rule::SpanPolicy::Anchor)
}

#[allow(clippy::too_many_arguments)]
fn run_scan(
    path: &str,
    format: &str,
    output: Option<&str>,
    exclude: Option<&str>,
    include: Option<&str>,
    rules_file: Option<&str>,
    diff: Option<&str>,
    semantic_diff: bool,
    baseline_out: Option<&str>,
    baseline: Option<&str>,
    baseline_tolerance: usize,
    gate: bool,
    gate_config: Option<&str>,
    enable: Option<&str>,
    disable: Option<&str>,
    min_severity: &str,
    parser_jar: Option<&str>,
    java_cmd: Option<&str>,
    config_path: &str,
    encoding: &str,
    no_cache: bool,
    timings: bool,
    locked: bool,
    no_lock: bool,
) -> anyhow::Result<()> {
    let start = Instant::now();

    if semantic_diff && diff.is_none() {
        return Err(anyhow::anyhow!(
            "--semantic-diff requires --diff (e.g. --diff HEAD~1)"
        ));
    }

    // 加载配置文件（如果存在）
    let project_config = load_project_config(config_path)?;
    let report_format = ReportFormat::from_str(format).map_err(|e| anyhow::anyhow!("{e}"))?;

    // 合并 encoding：CLI 参数优先于配置文件
    let effective_encoding = if encoding != "auto" {
        encoding
    } else if let Some(ref enc) = project_config.scan.encoding {
        enc.as_str()
    } else {
        "auto"
    };

    // 合并配置文件和 CLI 参数（CLI 优先）
    let enable_str = enable.unwrap_or("");
    let disable_str = disable.unwrap_or("");
    let enable_ids: Vec<String> = if enable_str.is_empty() {
        project_config.rules.enable.clone()
    } else {
        enable_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let disable_ids: Vec<String> = if disable_str.is_empty() {
        project_config.rules.disable.clone()
    } else {
        disable_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let min_sev: guard_core::rule::Severity = if min_severity.is_empty() {
        project_config
            .rules
            .min_severity
            .as_deref()
            .unwrap_or("info")
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid min_severity: {e}"))?
    } else {
        min_severity
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid min_severity: {e}"))?
    };

    // 默认排除目录
    let default_excludes = ["target", "build", ".git", "node_modules"];
    let mut excludes: Vec<String> = match exclude {
        Some(e) => e
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => default_excludes.iter().map(|s| s.to_string()).collect(),
    };
    // 合并配置文件的 exclude
    excludes.extend(project_config.scan.exclude.iter().cloned());
    let excludes_ref: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();

    // 路径白名单：CLI + 配置文件
    let includes: Vec<String> = match include {
        Some(i) => i
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => project_config.scan.include.clone(),
    };
    let includes_ref: Vec<&str> = includes.iter().map(|s| s.as_str()).collect();

    // 查找 java-parser.jar
    let jar_path = find_parser_jar(parser_jar)?;
    let mut cli_parser = CliParser::new(&jar_path);
    if let Some(cmd) = java_cmd {
        cli_parser = cli_parser.with_java_cmd(cmd);
    }

    // AST 解析缓存（默认启用；--no-cache 关闭）
    let cache = AstCache::new(!no_cache, &parser_fingerprint(&jar_path)?);

    // 收集规则
    let t_rules_load = Instant::now();
    let builtin = rules::builtin_rules();
    let mut rule_list: Vec<Arc<dyn Rule<CompilationUnit>>> = Vec::new();

    // 确定规则文件路径（解析基准分离，避免双重拼接）：
    // - CLI --rules-file：相对当前工作目录（cwd）解析（CLI 参数惯例）
    // - 配置文件 rules_file：相对配置文件所在目录解析
    // - 默认 javaguard.rules.toml：相对 cwd
    let config_dir = Path::new(config_path).parent().unwrap_or(Path::new("."));
    let resolved_rules_file = if let Some(rf) = rules_file {
        PathBuf::from(rf)
    } else if let Some(rf) = &project_config.rules.rules_file {
        let pb = PathBuf::from(rf);
        if pb.is_absolute() {
            pb
        } else {
            config_dir.join(pb)
        }
    } else {
        PathBuf::from("javaguard.rules.toml")
    };

    // 规则文件不存在时显式告警（避免静默回退导致 YAML/Rhai 规则全部丢失）
    if !resolved_rules_file.exists() {
        eprintln!(
            "warn: rules file not found: {}（将回退到内置规则，YAML/Rhai 规则不会被加载）",
            resolved_rules_file.display()
        );
    }

    // 优先从 TOML 规则文件 + `[rule_packs]` 规则包加载（合并优先级：
    // 包层 < 项目本地规则 < [[rule_packs.overrides]]）。
    // 包层错误（找不到包 / 清单非法 / 版本契约不满足）为硬错误，直接失败；
    // 本地规则文件出错只告警，走下方回退分支。
    let merged = match rulepack::resolve_and_merge(
        &project_config.rule_packs,
        &resolved_rules_file,
        config_dir,
    ) {
        Ok(m) => m,
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };
    for w in &merged.warnings {
        eprintln!("warn: failed to load rules file: {w}");
    }
    for n in &merged.notes {
        eprintln!("note: {n}");
    }
    if !merged.packs.is_empty() {
        let summary: Vec<String> = merged
            .packs
            .iter()
            .map(|p| format!("{}@{}({} rules)", p.name, p.version, p.rule_count))
            .collect();
        eprintln!("note: rule packs: {}", summary.join(", "));
    }

    // 锁文件校验（保证「同配置 → 同规则集 → 同结果」，CI 可复现）。
    // 默认 Auto：只比对 name/version；--locked 升为 Strict（额外比对 checksum）；
    // --no-lock 跳过。包层校验失败为硬错误。
    let lock_mode = if no_lock {
        rulelock::LockMode::Off
    } else if locked {
        rulelock::LockMode::Strict
    } else {
        rulelock::LockMode::Auto
    };
    for note in rulelock::verify_lock(&merged.packs, config_dir, lock_mode)? {
        eprintln!("note: {note}");
    }

    let config_dir_for_rules = resolved_rules_file.parent().unwrap_or(Path::new("."));
    let loaded_from_toml = !merged.rules.is_empty();
    if loaded_from_toml {
        for entry in &merged.rules {
            if let Some(r) = load_rule_from_entry(entry, config_dir_for_rules, &builtin) {
                rule_list.push(r);
            }
        }
    }

    // 回退：目录扫描（向后兼容旧版 rules/ 目录结构）
    // 注意：不能依赖 env!("CARGO_MANIFEST_DIR")（编译期源码路径，发布后用户机器上不存在），
    // 而是从 exe 所在目录（部署布局）或 cwd（源码布局）定位 rules/。
    if !loaded_from_toml {
        for r in &builtin {
            rule_list.push(r.clone());
        }
        if let Some(yaml_dir) = find_fallback_rules_dir() {
            let yaml_rules = load_yaml_rules(&yaml_dir);
            for yr in yaml_rules {
                rule_list.push(Arc::new(YamlRuleAdapter::new(yr)));
            }
            let rhai_dir = yaml_dir.join("rhai");
            if rhai_dir.is_dir() {
                if let Ok(rhai_rules) = load_rhai_rules(&rhai_dir) {
                    for rr in rhai_rules {
                        rule_list.push(Arc::new(RhaiRuleAdapter::new(rr)));
                    }
                }
            }
        }
    }

    // 规则过滤（列表层即最终生效集，run_rules 不再二次判断 enabled）：
    // 1) disable 总是生效（移除指定 ID）；
    // 2) enable 覆盖优先级最高——非空时仅保留列出的规则，无视 enabled 字段
    //    （否则 enabled=false 的规则即便被 --enable 选中，仍会在 run_rules 的 enabled() 闸门被跳过）；
    // 3) 未指定 enable 时，默认仅保留 enabled=true 的规则。
    // id 匹配：规范 id（`ns:J001`）精确匹配，或裸 id（`J001`）跨包聚合匹配。
    if !disable_ids.is_empty() {
        rule_list.retain(|r| {
            !disable_ids
                .iter()
                .any(|d| rulepack::id_matches(d, &r.id().0))
        });
    }
    if !enable_ids.is_empty() {
        rule_list.retain(|r| enable_ids.iter().any(|e| rulepack::id_matches(e, &r.id().0)));
    } else {
        rule_list.retain(|r| r.enabled());
    }

    // 规则过滤：min_severity
    rule_list.retain(|r| r.severity() >= min_sev);

    let enabled_count = rule_list.len();
    let rules_load_ms = t_rules_load.elapsed().as_millis() as u64;

    // 扫描文件
    let t_traverse = Instant::now();
    let root = Path::new(path);
    let scan_result = scanner::scan_java_files(root, &excludes_ref);

    // 路径白名单过滤
    let scan_files = if includes_ref.is_empty() {
        scan_result.files.clone()
    } else {
        scan_result
            .files
            .iter()
            .filter(|f| {
                let f_str = f.to_string_lossy().replace('\\', "/");
                includes_ref.iter().any(|inc| {
                    let inc = inc.replace('\\', "/");
                    f_str.contains(&inc)
                })
            })
            .cloned()
            .collect()
    };
    let scan_result = scanner::ScanResult {
        files: scan_files,
        root: scan_result.root,
    };
    let traverse_ms = t_traverse.elapsed().as_millis() as u64;

    // M5: 增量扫描 — git diff 过滤
    let t_diff = Instant::now();
    let mut line_mapper: Option<git_diff::LineMapper> = None;
    let line_filter = if let Some(diff_spec) = diff {
        match git_diff::get_diff(root, diff_spec) {
            Ok(mut diffs) => {
                // git diff 返回的路径相对于 repo root，而 scan_result.root 是绝对路径。
                // 找到 git root，把 diff path 转成绝对路径，统一基准。
                let git_root = git_diff::find_git_root(root)
                    .ok()
                    .and_then(|p| {
                        let canon = std::fs::canonicalize(&p).ok()?;
                        let s = canon.to_string_lossy();
                        let stripped = s.strip_prefix("\\\\?\\").unwrap_or(&s);
                        Some(PathBuf::from(stripped))
                    })
                    .unwrap_or_else(|| root.to_path_buf());
                let git_root_str = git_root.to_string_lossy().replace('\\', "/");
                let git_root_prefix = if git_root_str.ends_with('/') {
                    git_root_str.clone()
                } else {
                    format!("{}/", git_root_str)
                };
                let scan_root_str = scan_result.root.to_string_lossy().replace('\\', "/");
                let scan_root_prefix = if scan_root_str.ends_with('/') {
                    scan_root_str.clone()
                } else {
                    format!("{}/", scan_root_str)
                };
                // 将 diff 文件路径转为 scan-root-relative
                for d in &mut diffs {
                    let p = d.path.replace('\\', "/");
                    // 先转成绝对路径
                    let abs = if p.starts_with(&git_root_prefix) || p == git_root_str {
                        p.clone()
                    } else {
                        format!("{}{}", git_root_prefix, p)
                    };
                    // 再转成 scan-root-relative
                    d.path = if let Some(rel) = abs.strip_prefix(&scan_root_prefix) {
                        rel.to_string()
                    } else if abs == scan_root_str {
                        String::new()
                    } else {
                        // 不在 scan root 下，保留绝对路径（会被文件过滤排除）
                        abs
                    };
                }

                let diff_files: std::collections::HashSet<String> =
                    diffs.iter().map(|d| d.path.clone()).collect();
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
                line_mapper = Some(git_diff::LineMapper::from_diffs(&diffs));
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
    let diff_ms = t_diff.elapsed().as_millis() as u64;

    // 语义对比模式：解析 diff 规格的「旧侧」引用
    let semantic_old_ref: Option<String> = if semantic_diff {
        let spec = diff.expect("validated above: --semantic-diff requires --diff");
        match resolve_old_ref(root, spec) {
            Ok(old_ref) => Some(old_ref),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to resolve old ref for --semantic-diff (--diff {spec}): {e}"
                ));
            }
        }
    } else {
        None
    };

    eprintln!(
        "Scanning {} .java files ({} rules enabled)...",
        line_filter.0.len(),
        enabled_count
    );

    // 解析器选择：LazyParser 首次真实解析（缓存 miss）时才启动 daemon 池，
    // AST 缓存全命中的增量扫描零 JVM 启动成本；池启动失败自动回退 per-file JVM 并显眼告警。
    // 池大小 = min(CPU 核数, 4, 文件数)，常驻 JVM 内存约 32-512MB/个。
    // 可通过环境变量 JAVAGUARD_PARSER_MODE=cli 强制使用单次模式（调试/对比用）。
    let n_files = line_filter.0.len();
    let force_cli = std::env::var("JAVAGUARD_PARSER_MODE").is_ok_and(|m| m.eq_ignore_ascii_case("cli"));
    let java_cmd = java_cmd
        .map(|c| c.to_string())
        .or_else(|| std::env::var("JAVA_CMD").ok())
        .unwrap_or_else(|| "java".to_string());
    let parser: Arc<dyn JavaParser> = if n_files > 0 && !force_cli {
        let pool_size = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, n_files.min(4));
        Arc::new(LazyParser::new(&jar_path, &java_cmd, pool_size))
    } else {
        Arc::new(cli_parser)
    };

    // 解析 + 检查（并行，受 CPU 核数限制；daemon 池按实例加锁，无冲突风险）
    let mut collector = ViolationCollector::new();
    let timings_stat = Timings::default();
    let parsed = std::sync::atomic::AtomicUsize::new(0);
    let parse_errors = std::sync::atomic::AtomicUsize::new(0);

    let t_check = Instant::now();
    let results: Vec<FileCheck> = {
        let files = &line_filter.0;
        if files.is_empty() {
            Vec::new()
        } else {
            let n_workers = thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .clamp(1, files.len().min(8));
            let collected: Mutex<Vec<FileCheck>> = Mutex::new(Vec::with_capacity(files.len()));
            thread::scope(|s| {
                for w in 0..n_workers {
                    let collected = &collected;
                    let parser = &parser;
                    let cache = &cache;
                    let rule_list = &rule_list;
                    let line_filter = &line_filter.1;
                    let mapper = &line_mapper;
                    let old_ref = &semantic_old_ref;
                    let root = &scan_result.root;
                    let parsed = &parsed;
                    let parse_errors = &parse_errors;
                    let timings = &timings_stat;
                    let encoding = effective_encoding;
                    s.spawn(move || {
                        for idx in (w..files.len()).step_by(n_workers) {
                            let check = check_one_file(
                                &files[idx],
                                parser.as_ref(),
                                cache,
                                rule_list,
                                line_filter,
                                mapper.as_ref(),
                                old_ref.as_deref(),
                                root,
                                encoding,
                                timings,
                            );
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
    let check_ms = t_check.elapsed().as_millis() as u64;

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
    // - 语义对比模式（有 LineMapper）：用新旧行号映射把 baseline 精确翻译到新文件行号，零容差匹配
    // - 普通模式：距离容忍匹配（同一 (文件, 规则) 行号差 ≤ tolerance 视为已知违规）
    let violations: Vec<_> = if let Some(baseline_path) = baseline {
        match load_baseline(baseline_path) {
            Ok(entries) => {
                let before = collector.count();
                let filtered = if let Some(mapper) = &line_mapper {
                    let f =
                        filter_baseline_mapped(collector.violations().to_vec(), &entries, mapper);
                    eprintln!(
                        "Baseline: {} of {} violations are new (mapped by diff hunks)",
                        f.len(),
                        before
                    );
                    f
                } else {
                    let f = filter_baseline(
                        collector.violations().to_vec(),
                        &entries,
                        baseline_tolerance,
                    );
                    eprintln!(
                        "Baseline: {} of {} violations are new (tolerance: {baseline_tolerance} lines)",
                        f.len(),
                        before
                    );
                    f
                };
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
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.rule_id.cmp(&b.rule_id))
    });

    // M5: 导出 baseline（快照当前全部违规，供后续 --baseline 使用）
    if let Some(out_path) = baseline_out {
        write_baseline(out_path, &violations)?;
    }

    // 输出报告
    let t_report = Instant::now();
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
    let report_ms = t_report.elapsed().as_millis() as u64;

    if timings {
        let read_ms = timings_stat
            .read_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        let parse_ms = timings_stat
            .parse_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        let rules_ms = timings_stat
            .rules_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "Timings: total {duration_ms} ms | rules-load {rules_load_ms} | traverse {traverse_ms} | diff {diff_ms} | check {check_ms} (Σ read {read_ms} / Σ parse {parse_ms} / Σ rules {rules_ms}, Σ across workers) | report {report_ms}"
        );
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
///
/// 条目形如 `{"file": "A.java", "line": 10, "rule_id": "J001"}`。
fn load_baseline(path: &str) -> anyhow::Result<Vec<(String, usize, String)>> {
    let content = std::fs::read_to_string(path)?;
    let baseline: Vec<serde_json::Value> =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse baseline JSON: {e}"))?;

    let mut list = Vec::with_capacity(baseline.len());
    for v in &baseline {
        let file = v.get("file").and_then(|f| f.as_str()).unwrap_or("");
        let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
        let rule_id = v.get("rule_id").and_then(|r| r.as_str()).unwrap_or("");
        list.push((file.to_string(), line, rule_id.to_string()));
    }

    Ok(list)
}

/// 把当前违规列表写为 baseline JSON（与 `load_baseline` 格式兼容）。
fn write_baseline(path: &str, violations: &[Violation]) -> anyhow::Result<()> {
    let list: Vec<serde_json::Value> = violations
        .iter()
        .map(|v| serde_json::json!({"file": v.file, "line": v.line, "rule_id": v.rule_id.0}))
        .collect();
    let json = serde_json::to_string_pretty(&list)?;
    std::fs::write(path, json)?;
    eprintln!("Baseline written to {path} ({} entries)", violations.len());
    Ok(())
}

/// 解析 git diff 规格的「旧侧」引用（--semantic-diff 用）。
///
/// - `A...B`：旧侧 = merge-base(A, B)（与 `git diff A...B` 语义一致）
/// - `A..B`：旧侧 = A
/// - `X`（单个引用）：旧侧 = X
fn resolve_old_ref(repo_root: &Path, diff_spec: &str) -> anyhow::Result<String> {
    if let Some((a, b)) = diff_spec.split_once("...") {
        let out = std::process::Command::new("git")
            .current_dir(repo_root)
            .args(["merge-base", a, b])
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run git merge-base {a} {b}: {e}"))?;
        if !out.status.success() {
            return Err(anyhow::anyhow!(
                "git merge-base {a} {b} failed (no common ancestor?)"
            ));
        }
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    if let Some((a, _)) = diff_spec.split_once("..") {
        return Ok(a.to_string());
    }
    Ok(diff_spec.to_string())
}

/// Baseline 精确过滤（语义对比模式）：用 LineMapper 把 baseline 行号精确翻译为
/// 新文件行号后做零容差匹配；被删除的行（翻译为 None）不参与匹配。
fn filter_baseline_mapped(
    violations: Vec<Violation>,
    baseline: &[(String, usize, String)],
    mapper: &git_diff::LineMapper,
) -> Vec<Violation> {
    let mut known: std::collections::HashSet<(String, usize, String)> =
        std::collections::HashSet::new();
    for (bf, bl, br) in baseline {
        if let Some(nl) = mapper.translate(bf, *bl) {
            known.insert((bf.clone(), nl, br.clone()));
        }
    }
    violations
        .into_iter()
        .filter(|v| !known.contains(&(v.file.clone(), v.line, v.rule_id.0.clone())))
        .collect()
}

/// Baseline 距离容忍过滤：只保留 baseline 之外的「新增违规」。
///
/// 匹配规则：
/// - 按 `(file, rule_id)` 分组匹配（不跨文件、不跨规则）；
/// - 行号差 ≤ `tolerance` 的最近未匹配 baseline 条目视为同一违规（抗行号漂移）；
/// - 每个 baseline 条目最多吸收一个违规（1:1 分配），避免单个条目误吞多个新增。
fn filter_baseline(
    violations: Vec<Violation>,
    baseline: &[(String, usize, String)],
    tolerance: usize,
) -> Vec<Violation> {
    let mut used = vec![false; baseline.len()];
    violations
        .into_iter()
        .filter(|v| {
            let mut best: Option<(u64, usize)> = None; // (行号差, baseline 索引)
            for (i, (bf, bl, br)) in baseline.iter().enumerate() {
                if used[i] || *bf != v.file || *br != v.rule_id.0 {
                    continue;
                }
                let dist = (*bl as i64 - v.line as i64).unsigned_abs();
                if dist <= tolerance as u64 && best.is_none_or(|(d, _)| dist < d) {
                    best = Some((dist, i));
                }
            }
            match best {
                Some((_, i)) => {
                    used[i] = true;
                    false // 命中 baseline → 已知违规，过滤
                }
                None => true, // 未命中 → 新增违规，保留
            }
        })
        .collect()
}

/// 计算 java-parser.jar 的指纹（规范路径 + 大小 + mtime），用于缓存失效控制。
fn parser_fingerprint(jar: &Path) -> anyhow::Result<String> {
    let meta = std::fs::metadata(jar)?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(format!("{}|{}|{mtime_ns}", jar.display(), meta.len()))
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

    // 部署布局（exe 同级或上级 java-parser/）优先；源码布局次之；构建机源码路径最后兜底
    let mut candidates = vec![
        PathBuf::from("java-parser/target/java-parser.jar"),
        PathBuf::from("../java-parser/target/java-parser.jar"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // 布局 A：exe 与 java-parser/ 平级
            candidates.push(parent.join("java-parser/java-parser.jar"));
            // 布局 B：exe 在 bin/ 下，java-parser/ 在其上级（deploy 标准布局）
            if let Some(grand) = parent.parent() {
                candidates.push(grand.join("java-parser/java-parser.jar"));
            }
        }
    }
    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("java-parser/target/java-parser.jar"));

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }

    Err(anyhow::anyhow!(
        "java-parser.jar not found. Set --parser-jar or JAVAGUARD_PARSER_JAR env."
    ))
}

/// 项目级配置文件 java-guard.toml 的模型。
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ProjectConfig {
    /// 规则配置
    rules: RulesConfig,
    /// 规则包配置（`[rule_packs]`，多包加载 / 合并 / 覆盖）
    rule_packs: rulepack::RulePacksConfig,
    /// 扫描配置
    scan: ScanConfig,
    /// gate 配置
    gate: Option<guard_core::gate::GateConfig>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct RulesConfig {
    /// 规则配置文件路径（相对于本配置文件目录解析）
    rules_file: Option<String>,
    /// 启用的规则 ID 列表（为空则全部启用；覆盖 rules_file 中的 enabled 字段）
    enable: Vec<String>,
    /// 禁用的规则 ID 列表（覆盖 rules_file 中的 enabled 字段）
    disable: Vec<String>,
    /// 最低严重级别
    min_severity: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ScanConfig {
    /// 路径白名单
    include: Vec<String>,
    /// 路径黑名单
    exclude: Vec<String>,
    /// 源文件编码（auto/utf-8/gbk 等，默认 auto）
    encoding: Option<String>,
}

/// 一条规则的完整定义（TOML [[rules]] 条目）。
///
/// 包含规则的元数据（name, group, description）和脚本路径（script_path）。
/// 脚本路径相对于规则文件所在目录解析，也支持绝对路径。
/// 内置 Rust 规则使用 `builtin:xxx` 前缀（如 `builtin:j008_empty_catch`）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RuleEntry {
    /// 规则 ID（如 "J001"），全局唯一
    pub id: String,
    /// 规则名称（如 "no_system_out"），用于日志和输出
    pub name: String,
    /// 规则分组（如 "naming", "code-style", "dependency-security"）
    #[serde(default)]
    pub group: Option<String>,
    /// 规则描述（如 "禁止使用 System.out / System.err 直接打印"）
    #[serde(default)]
    pub description: Option<String>,
    /// 规则脚本路径（YAML/Rhai 文件）或 `builtin:xxx` 标识
    /// 相对于规则文件所在目录解析
    pub script_path: String,
    /// 该规则适用的文件类型（目前固定 ["java"]）
    #[serde(default = "default_applies_to")]
    pub applies_to: Vec<String>,
    /// 严重级别：info / minor / major / critical
    #[serde(default = "default_severity")]
    pub severity: String,
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 规则分组内的参数（如 max_lines 等，传递给 Rhai 脚本的 `config` 变量）
    #[serde(default)]
    pub params: Option<toml::Value>,
}

fn default_applies_to() -> Vec<String> {
    vec!["java".to_string()]
}

fn default_severity() -> String {
    "info".to_string()
}

fn default_true() -> bool {
    true
}

/// 规则文件的完整结构（javaguard.rules.toml）。
///
/// 一个规则文件即一个**规则包**（Rule Pack）：可选的 `[pack]` 段声明包元数据
/// （name / version / api_version / engine 兼容范围），`[[rules]]` 为包内规则列表。
/// 不写 `[pack]` 段时按「无声明」处理，行为与引入包概念前完全一致（向后兼容）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RulesFile {
    /// 包元数据（可选）。声明后引擎会做 API 版本与引擎版本范围的硬校验。
    #[serde(default)]
    pub pack: Option<guard_core::RulePackMeta>,
    /// 规则列表
    pub rules: Vec<RuleEntry>,
}

/// 加载项目配置文件（TOML 格式）。
///
/// 文件不存在时返回默认值（不报错）。
fn load_project_config(path: &str) -> anyhow::Result<ProjectConfig> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(ProjectConfig::default());
    }
    let content =
        std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("read config {path}: {e}"))?;
    let cfg: ProjectConfig =
        toml::from_str(&content).map_err(|e| anyhow::anyhow!("parse config {path}: {e}"))?;
    Ok(cfg)
}

/// 加载规则文件的原始结构（javaguard.rules.toml）。
///
/// 文件不存在时返回 `None`（不报错）。返回的 `[pack]` 段已通过版本契约校验。
fn load_rules_file_raw(path: &str) -> anyhow::Result<Option<RulesFile>> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("read rules file {path}: {e}"))?;
    let file: RulesFile =
        toml::from_str(&content).map_err(|e| anyhow::anyhow!("parse rules file {path}: {e}"))?;
    // 规则包版本契约校验（硬错误）：防止「新规则跑在旧引擎上静默失效」。
    // 未声明 [pack] 的旧规则文件跳过校验，完全向后兼容。
    if let Some(pack) = &file.pack {
        pack.validate()
            .map_err(|e| anyhow::anyhow!("rules file {path}: {e}"))?;
        if let Some(e) = pack.engine_compat_error(env!("CARGO_PKG_VERSION")) {
            return Err(anyhow::anyhow!("rules file {path}: {e}"));
        }
    }
    // 校验 rule ID 唯一性
    let mut seen = std::collections::HashSet::new();
    for rule in &file.rules {
        if !seen.insert(&rule.id) {
            return Err(anyhow::anyhow!("duplicate rule ID: {}", rule.id));
        }
    }
    Ok(Some(file))
}

/// 加载规则文件（javaguard.rules.toml），返回规则列表。仅供测试使用。
///
/// 文件不存在时返回空列表（不报错）。
#[cfg(test)]
fn load_rules_file(path: &str) -> anyhow::Result<Vec<RuleEntry>> {
    Ok(load_rules_file_raw(path)?.map(|f| f.rules).unwrap_or_default())
}

/// 根据 RuleEntry 加载实际的规则执行器。
///
/// - `builtin:xxx` → 查找内置 Rust 规则
/// - `*.yml`/`*.yaml` → 加载 YAML 声明式规则
/// - `*.rhai` → 加载 Rhai 脚本规则
fn load_rule_from_entry(
    entry: &RuleEntry,
    config_dir: &Path,
    builtin: &[Arc<dyn Rule<CompilationUnit>>],
) -> Option<Arc<dyn Rule<CompilationUnit>>> {
    let script_path = &entry.script_path;

    if let Some(name) = script_path.strip_prefix("builtin:") {
        // 内置 Rust 规则
        return builtin.iter().find(|r| r.id().0 == name).cloned().map(|r| {
            // 用 RuleEntry 的元数据覆盖内置规则的元数据
            Arc::new(BuiltinRuleOverride::new(r, entry.clone())) as Arc<dyn Rule<CompilationUnit>>
        });
    }

    // 解析脚本路径（相对于配置文件所在目录）
    let resolved = if Path::new(script_path).is_absolute() {
        PathBuf::from(script_path)
    } else {
        config_dir.join(script_path)
    };

    if !resolved.exists() {
        eprintln!(
            "warn: rule {} script not found: {}",
            entry.id,
            resolved.display()
        );
        return None;
    }

    let ext = resolved.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "yml" | "yaml" => match rule_yaml::load_rule_file(&resolved) {
            Ok(mut yaml_rule) => {
                // 用 RuleEntry 的元数据覆盖 YAML 文件的元数据
                yaml_rule.id = entry.id.clone();
                if let Some(ref desc) = entry.description {
                    yaml_rule.title = desc.clone();
                }
                yaml_rule.severity = entry.severity.clone();
                yaml_rule.enabled = entry.enabled;
                // 加载期校验（必须在元数据覆盖之后，才能同时校验 TOML 侧的 severity）。
                // 未校验的错误不会报错、只会静默走偏：
                //   - match_fields 键写错        → 规则永不命中（隐形失效）
                //   - within/not_within 拼错 kind → 上下文约束恒不满足
                //   - match_members 用在非注解上  → 该键被忽略，规则退化成「匹配所有同类节点」→ 大量误报
                if let Err(bad) = yaml_rule.validate() {
                    eprintln!(
                        "warn: skip rule {} ({}): {}",
                        entry.id,
                        resolved.display(),
                        bad.join("; ")
                    );
                    return None;
                }
                Some(Arc::new(YamlRuleAdapter::new(yaml_rule)))
            }
            Err(e) => {
                eprintln!("warn: skip rule {}: {}", entry.id, e);
                None
            }
        },
        "rhai" => match rule_rhai::rule::load_rhai_rule_file(&resolved) {
            Ok(mut rhai_rule) => {
                // 用 RuleEntry 的元数据覆盖 Rhai 文件的元数据
                rhai_rule.id = entry.id.clone();
                if let Some(ref desc) = entry.description {
                    rhai_rule.title = desc.clone();
                }
                rhai_rule.severity = entry.severity.clone();
                rhai_rule.enabled = entry.enabled;
                // 将 toml::Value 转为 serde_yaml::Value 传递 params
                if let Some(ref params) = entry.params {
                    rhai_rule.params = toml_value_to_yaml(params);
                }
                if let Err(bad) = rhai_rule.validate() {
                    eprintln!(
                        "warn: skip rule {} ({}): {}",
                        entry.id,
                        resolved.display(),
                        bad.join("; ")
                    );
                    return None;
                }
                Some(Arc::new(RhaiRuleAdapter::new(rhai_rule)))
            }
            Err(e) => {
                eprintln!("warn: skip rule {}: {}", entry.id, e);
                None
            }
        },
        _ => {
            eprintln!(
                "warn: skip rule {}: unsupported file extension '{}'",
                entry.id, ext
            );
            None
        }
    }
}

/// 将 toml::Value 递归转换为 serde_yaml::Value。
fn toml_value_to_yaml(v: &toml::Value) -> serde_yaml::Value {
    match v {
        toml::Value::String(s) => serde_yaml::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_yaml::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_yaml::Value::Number(serde_yaml::Number::from(*f)),
        toml::Value::Boolean(b) => serde_yaml::Value::Bool(*b),
        toml::Value::Array(arr) => {
            serde_yaml::Value::Sequence(arr.iter().map(toml_value_to_yaml).collect())
        }
        toml::Value::Table(map) => {
            let mut yaml_map = serde_yaml::Mapping::new();
            for (k, v) in map {
                yaml_map.insert(serde_yaml::Value::String(k.clone()), toml_value_to_yaml(v));
            }
            serde_yaml::Value::Mapping(yaml_map)
        }
        toml::Value::Datetime(dt) => serde_yaml::Value::String(dt.to_string()),
    }
}

/// 包装内置规则，用 RuleEntry 元数据覆盖 severity / enabled。
///
/// description() 委托给内置规则（lifetime 限制无法返回 RuleEntry 的 &str）。
/// RuleEntry 的 description 在 `rules` 命令中直接使用。
struct BuiltinRuleOverride {
    inner: Arc<dyn Rule<CompilationUnit>>,
    entry: RuleEntry,
}

impl BuiltinRuleOverride {
    fn new(inner: Arc<dyn Rule<CompilationUnit>>, entry: RuleEntry) -> Self {
        Self { inner, entry }
    }
}

impl Rule<CompilationUnit> for BuiltinRuleOverride {
    fn id(&self) -> &RuleId {
        self.inner.id()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn severity(&self) -> guard_core::rule::Severity {
        self.entry
            .severity
            .parse()
            .unwrap_or_else(|_| self.inner.severity())
    }

    fn enabled(&self) -> bool {
        self.entry.enabled
    }

    fn check_unit(&self, unit: &CompilationUnit) -> Vec<Violation> {
        self.inner.check_unit(unit)
    }

    fn span_policy(&self) -> guard_core::rule::SpanPolicy {
        self.inner.span_policy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guard_core::rule::Severity;

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
            Command::Rules { action: None }
        ));
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "rules", "lock"]).command,
            Command::Rules {
                action: Some(RulesAction::Lock)
            }
        ));
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "rules", "verify"]).command,
            Command::Rules {
                action: Some(RulesAction::Verify { locked: false })
            }
        ));
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "rules", "verify", "--locked"]).command,
            Command::Rules {
                action: Some(RulesAction::Verify { locked: true })
            }
        ));
        assert!(matches!(
            Cli::parse_from(vec!["java-guard", "version"]).command,
            Command::Version
        ));
    }

    #[test]
    fn cli_scan_lock_flags_parse() {
        let cli = Cli::parse_from(vec!["java-guard", "scan", ".", "--locked"]);
        let Command::Scan { locked, no_lock, .. } = cli.command else {
            panic!("expected scan");
        };
        assert!(locked && !no_lock);

        let cli = Cli::parse_from(vec!["java-guard", "scan", ".", "--no-lock"]);
        let Command::Scan { locked, no_lock, .. } = cli.command else {
            panic!("expected scan");
        };
        assert!(!locked && no_lock);
    }

    #[test]
    fn load_project_config_missing_returns_default() {
        let cfg = load_project_config("__nonexistent_config_12345.toml").unwrap();
        assert!(cfg.rules.enable.is_empty());
        assert!(cfg.rules.disable.is_empty());
        assert!(cfg.scan.include.is_empty());
        assert!(cfg.scan.exclude.is_empty());
        assert!(cfg.gate.is_none());
    }

    #[test]
    fn load_project_config_parses_toml() {
        let dir = std::env::temp_dir().join("javaguard_cfg_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("java-guard.toml");
        let toml_content = r#"
[rules]
enable = ["J001", "J003"]
disable = ["J008"]
min_severity = "major"

[scan]
include = ["src/main"]
exclude = ["build"]
"#;
        std::fs::write(&path, toml_content).unwrap();
        let cfg = load_project_config(path.to_str().unwrap()).unwrap();
        assert_eq!(
            cfg.rules.enable,
            vec!["J001".to_string(), "J003".to_string()]
        );
        assert_eq!(cfg.rules.disable, vec!["J008".to_string()]);
        assert_eq!(cfg.rules.min_severity.as_deref(), Some("major"));
        assert_eq!(cfg.scan.include, vec!["src/main".to_string()]);
        assert_eq!(cfg.scan.exclude, vec!["build".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_parses_toml() {
        let dir = std::env::temp_dir().join("javaguard_rules_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[[rules]]
id = "J001"
name = "no_system_out"
group = "code-style"
description = "禁止使用 System.out / System.err 直接打印"
script_path = "rules/J001_no_system_out.yml"
severity = "info"
enabled = true

[[rules]]
id = "J008"
name = "empty_catch"
group = "potential-bug"
description = "空 catch 块检测"
script_path = "builtin:j008_empty_catch"
severity = "warning"
enabled = false
"#;
        std::fs::write(&path, toml_content).unwrap();
        let entries = load_rules_file(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "J001");
        assert_eq!(entries[0].name, "no_system_out");
        assert_eq!(entries[0].group.as_deref(), Some("code-style"));
        assert_eq!(entries[0].severity, "info");
        assert!(entries[0].enabled);
        assert_eq!(entries[1].id, "J008");
        assert!(!entries[1].enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_duplicate_id_errors() {
        let dir = std::env::temp_dir().join("javaguard_rules_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"

[[rules]]
id = "J001"
name = "b"
script_path = "y.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        assert!(load_rules_file(path.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_without_pack_is_backward_compatible() {
        let dir = std::env::temp_dir().join("javaguard_rules_test_no_pack");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        let entries = load_rules_file(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_with_compatible_pack_passes() {
        let dir = std::env::temp_dir().join("javaguard_rules_test_pack_ok");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[pack]
name = "test-pack"
version = "1.0.0"
api_version = 1
engine = ">=0.1 <1"

[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        let entries = load_rules_file(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_with_future_api_version_errors() {
        let dir = std::env::temp_dir().join("javaguard_rules_test_pack_api");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[pack]
name = "future-pack"
version = "2.0.0"
api_version = 999

[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        let err = load_rules_file(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("future-pack"), "msg: {msg}");
        assert!(msg.contains("rule API version"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_with_incompatible_engine_errors() {
        let dir = std::env::temp_dir().join("javaguard_rules_test_pack_engine");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[pack]
name = "engine-mismatch"
version = "1.0.0"
api_version = 1
engine = ">=99.0"

[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        let err = load_rules_file(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("engine-mismatch"), "msg: {msg}");
        assert!(msg.contains("engine version"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rules_file_with_invalid_engine_range_errors() {
        let dir = std::env::temp_dir().join("javaguard_rules_test_pack_bad_range");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("javaguard.rules.toml");
        let toml_content = r#"
[pack]
name = "bad-range"
engine = ">=abc"

[[rules]]
id = "J001"
name = "a"
script_path = "x.yml"
"#;
        std::fs::write(&path, toml_content).unwrap();
        assert!(load_rules_file(path.to_str().unwrap()).is_err());
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
        let entries = load_baseline(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains(&("A.java".to_string(), 10, "J001".to_string())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn baseline_matches_exact_line() {
        let baseline = vec![("A.java".to_string(), 10, "J001".to_string())];
        let vs = vec![Violation::new("J001", Severity::Minor, "A.java", 10, "x")];
        assert!(filter_baseline(vs, &baseline, 5).is_empty());
    }

    #[test]
    fn baseline_tolerates_line_drift() {
        let baseline = vec![("A.java".to_string(), 10, "J001".to_string())];
        // 上面插入 2 行 → 行号漂移到 12，容差内视为已知
        let vs = vec![Violation::new("J001", Severity::Minor, "A.java", 12, "x")];
        assert!(filter_baseline(vs, &baseline, 5).is_empty());
        // 漂移 6 行超出容差 → 视为新增
        let vs2 = vec![Violation::new("J001", Severity::Minor, "A.java", 16, "x")];
        assert_eq!(filter_baseline(vs2, &baseline, 5).len(), 1);
        // 容差为 0 退化为精确行号匹配
        let vs3 = vec![Violation::new("J001", Severity::Minor, "A.java", 11, "x")];
        assert_eq!(filter_baseline(vs3, &baseline, 0).len(), 1);
    }

    #[test]
    fn baseline_requires_same_file_and_rule() {
        let baseline = vec![("A.java".to_string(), 10, "J001".to_string())];
        // 文件不同 / 规则不同 → 都不匹配
        let vs = vec![
            Violation::new("J001", Severity::Minor, "B.java", 10, "x"),
            Violation::new("J003", Severity::Minor, "A.java", 10, "x"),
        ];
        assert_eq!(filter_baseline(vs, &baseline, 5).len(), 2);
    }

    #[test]
    fn baseline_one_to_one_matching() {
        // 两条 baseline 各吸收一个相邻违规 → 全过滤
        let baseline = vec![
            ("A.java".to_string(), 10, "J001".to_string()),
            ("A.java".to_string(), 13, "J001".to_string()),
        ];
        let vs = vec![
            Violation::new("J001", Severity::Minor, "A.java", 11, "x"),
            Violation::new("J001", Severity::Minor, "A.java", 12, "x"),
        ];
        assert!(filter_baseline(vs, &baseline, 5).is_empty());
        // 只有一条 baseline：最近的违规被吸收，另一个仍是新增
        let baseline1 = vec![("A.java".to_string(), 10, "J001".to_string())];
        let vs2 = vec![
            Violation::new("J001", Severity::Minor, "A.java", 11, "x"),
            Violation::new("J001", Severity::Minor, "A.java", 12, "x"),
        ];
        assert_eq!(filter_baseline(vs2, &baseline1, 5).len(), 1);
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

    #[test]
    fn read_source_file_utf8() {
        let dir = std::env::temp_dir().join("javaguard_enc_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("utf8.java");
        std::fs::write(&path, "class A {\n}").unwrap();
        let s = read_source_file(&path, "auto").unwrap();
        assert!(s.contains("class A"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_source_file_gbk() {
        let dir = std::env::temp_dir().join("javaguard_enc_test2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("gbk.java");
        // GBK 编码的中文注释："// 测试中文"
        let gbk_bytes = vec![
            0x2F, 0x2F, 0x20, 0xB2, 0xE2, 0xCA, 0xD4, 0xD6, 0xD0, 0xCE, 0xC4,
        ];
        std::fs::write(&path, &gbk_bytes).unwrap();
        let s = read_source_file(&path, "auto").unwrap();
        assert!(
            s.contains("测试"),
            "auto-detect should decode GBK to readable Chinese, got: {s:?}"
        );
        // 显式指定 GBK
        let s2 = read_source_file(&path, "gbk").unwrap();
        assert!(s2.contains("测试"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_source_file_utf8_bom() {
        let dir = std::env::temp_dir().join("javaguard_enc_test3");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bom.java");
        let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        bytes.extend_from_slice(b"class A {}");
        std::fs::write(&path, &bytes).unwrap();
        let s = read_source_file(&path, "auto").unwrap();
        assert!(s.starts_with("class A"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_source_file_explicit_encoding() {
        let dir = std::env::temp_dir().join("javaguard_enc_test4");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("shiftjis.java");
        // Shift-JIS 编码的日文："// テスト"
        let sjis_bytes = vec![0x2F, 0x2F, 0x20, 0x83, 0x65, 0x83, 0x58, 0x83, 0x67];
        std::fs::write(&path, &sjis_bytes).unwrap();
        let s = read_source_file(&path, "shift-jis").unwrap();
        assert!(s.contains("テスト"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_source_file_missing_file_errors() {
        let result = read_source_file(std::path::Path::new("/no/such/file.java"), "auto");
        assert!(result.is_err());
    }

    fn mapper_with_hunks(hunks: Vec<git_diff::Hunk>) -> git_diff::LineMapper {
        git_diff::LineMapper::from_diffs(&[git_diff::FileDiff {
            path: "src/A.java".to_string(),
            kind: git_diff::DiffKind::Modified,
            line_ranges: vec![],
            hunks,
            is_new: false,
        }])
    }

    #[test]
    fn semantic_difference_removes_translated_lines() {
        // 旧行 12 上方插入 3 行 → 旧违规 (12) 翻译为 15；新违规在 15 → 已知，过滤
        let mapper = mapper_with_hunks(vec![git_diff::Hunk {
            old_start: 5,
            old_len: 0,
            new_start: 8,
            new_len: 3,
        }]);
        let old_vs = vec![Violation::new(
            "J001",
            Severity::Minor,
            "src/A.java",
            12,
            "x",
        )];
        let new_vs = vec![
            Violation::new("J001", Severity::Minor, "src/A.java", 15, "x"), // 已知
            Violation::new("J001", Severity::Minor, "src/A.java", 30, "x"), // 新增
        ];
        let kept = semantic_difference(new_vs, old_vs, &mapper);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].line, 30);
    }

    #[test]
    fn semantic_difference_keeps_new_violations() {
        let mapper = mapper_with_hunks(vec![]);
        let old_vs = vec![Violation::new(
            "J001",
            Severity::Minor,
            "src/A.java",
            10,
            "x",
        )];
        let new_vs = vec![
            Violation::new("J001", Severity::Minor, "src/A.java", 11, "x"), // 行号变了
            Violation::new("J003", Severity::Minor, "src/A.java", 10, "x"), // 规则变了
        ];
        let kept = semantic_difference(new_vs, old_vs, &mapper);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn semantic_difference_deleted_lines_not_known() {
        // 旧行 5-7 被删除；旧违规位于被删行 → 不参与匹配，新位置违规视为新增
        let mapper = mapper_with_hunks(vec![git_diff::Hunk {
            old_start: 5,
            old_len: 3,
            new_start: 5,
            new_len: 0,
        }]);
        let old_vs = vec![Violation::new(
            "J001",
            Severity::Minor,
            "src/A.java",
            6,
            "x",
        )];
        let new_vs = vec![Violation::new(
            "J001",
            Severity::Minor,
            "src/A.java",
            5,
            "x",
        )];
        let kept = semantic_difference(new_vs, old_vs, &mapper);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn filter_baseline_mapped_translates_lines() {
        // baseline 行 10 因上方插入 2 行翻译为 12；违规在 12 → 已知
        let mapper = mapper_with_hunks(vec![git_diff::Hunk {
            old_start: 3,
            old_len: 0,
            new_start: 5,
            new_len: 2,
        }]);
        let baseline = vec![("src/A.java".to_string(), 10, "J001".to_string())];
        let vs = vec![
            Violation::new("J001", Severity::Minor, "src/A.java", 12, "x"),
            Violation::new("J001", Severity::Minor, "src/A.java", 11, "x"),
        ];
        let kept = filter_baseline_mapped(vs, &baseline, &mapper);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].line, 11);
    }

    #[test]
    fn filter_baseline_mapped_deleted_line_not_known() {
        // baseline 位于被删除的行 → 不匹配，违规视为新增
        let mapper = mapper_with_hunks(vec![git_diff::Hunk {
            old_start: 10,
            old_len: 2,
            new_start: 10,
            new_len: 0,
        }]);
        let baseline = vec![("src/A.java".to_string(), 11, "J001".to_string())];
        let vs = vec![Violation::new(
            "J001",
            Severity::Minor,
            "src/A.java",
            11,
            "x",
        )];
        let kept = filter_baseline_mapped(vs, &baseline, &mapper);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn resolve_old_ref_forms() {
        assert_eq!(
            resolve_old_ref(std::path::Path::new("."), "HEAD~1").unwrap(),
            "HEAD~1"
        );
        assert_eq!(
            resolve_old_ref(std::path::Path::new("."), "main..feature").unwrap(),
            "main"
        );
        assert_eq!(
            resolve_old_ref(std::path::Path::new("."), "v1.0").unwrap(),
            "v1.0"
        );
    }

    #[test]
    fn write_baseline_roundtrip() {
        let dir = std::env::temp_dir().join("javaguard_baseline_out_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("out.json");
        let vs = vec![
            Violation::new("J001", Severity::Minor, "A.java", 10, "x"),
            Violation::new("J009", Severity::Major, "B.java", 3, "x"),
        ];
        write_baseline(path.to_str().unwrap(), &vs).unwrap();
        let entries = load_baseline(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&("A.java".to_string(), 10, "J001".to_string())));
        assert!(entries.contains(&("B.java".to_string(), 3, "J009".to_string())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 规则加载期校验 ──
    //
    // TOML（javaguard.rules.toml）驱动的加载路径过去**不调用** validate()，
    // 非法规则会被照常装载：失效的约束被静默忽略，规则退化成「匹配所有同类节点」
    // 并产生大量误报。以下两个用例锁住「非法即跳过」的行为。

    fn rule_entry(id: &str, script_path: &str) -> RuleEntry {
        RuleEntry {
            id: id.to_string(),
            name: id.to_lowercase(),
            group: None,
            description: None,
            script_path: script_path.to_string(),
            applies_to: vec!["java".to_string()],
            severity: "minor".to_string(),
            enabled: true,
            params: None,
        }
    }

    #[test]
    fn load_rule_from_entry_skips_match_members_on_non_annotation() {
        let dir = std::env::temp_dir().join("javaguard_rule_validate_test");
        let _ = std::fs::create_dir_all(&dir);

        let bad = dir.join("bad.yml");
        std::fs::write(
            &bad,
            r#"
id: BAD
title: "match_members 用在 MethodCall 上"
severity: minor
pattern:
  type: MethodCall
  match_members:
    basePackages: "*"
message: "x"
"#,
        )
        .unwrap();
        assert!(
            load_rule_from_entry(&rule_entry("BAD", "bad.yml"), &dir, &[]).is_none(),
            "非法规则应被跳过，而不是带着失效配置进入扫描"
        );

        // 同一份规则换成 Annotation pattern 后应当通过
        let good = dir.join("good.yml");
        std::fs::write(
            &good,
            r#"
id: GOOD
title: "match_members 用在 Annotation 上"
severity: minor
pattern:
  type: Annotation
  match_fields:
    name: ComponentScan
  match_members:
    basePackages: "*"
message: "x"
"#,
        )
        .unwrap();
        assert!(load_rule_from_entry(&rule_entry("GOOD", "good.yml"), &dir, &[]).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rule_from_entry_skips_bad_ancestor_kind_and_import_context() {
        let dir = std::env::temp_dir().join("javaguard_rule_validate_test2");
        let _ = std::fs::create_dir_all(&dir);

        let typo = dir.join("typo.yml");
        std::fs::write(
            &typo,
            r#"
id: TYPO
title: "祖先 kind 拼写错误"
severity: minor
pattern:
  type: MethodCall
  within:
    - kind: ForEachStatement
message: "x"
"#,
        )
        .unwrap();
        assert!(load_rule_from_entry(&rule_entry("TYPO", "typo.yml"), &dir, &[]).is_none());

        let import_ctx = dir.join("import_ctx.yml");
        std::fs::write(
            &import_ctx,
            r#"
id: IMPCTX
title: "import 上写上下文谓词"
severity: minor
pattern:
  type: Import
  in_type:
    annotations: [RestController]
message: "x"
"#,
        )
        .unwrap();
        assert!(load_rule_from_entry(&rule_entry("IMPCTX", "import_ctx.yml"), &dir, &[]).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== 规则包分层守护测试（防漂移）=====
    //
    // 「通用 / 定制」分层后，规则内容存在两份载体：
    //   1. javaguard.rules.toml（单文件旧布局镜像，默认消费路径）；
    //   2. config/rules-*/rules-pack.toml（可独立发布与版本化的规则包）。
    // 以下测试保证两者逐条一致：任何一侧改动而另一侧未同步时，CI 直接失败。

    /// config/ 下的三个默认规则包：`(目录名, 期望 namespace)`。
    const DEFAULT_PACKS: [(&str, Option<&str>); 3] = [
        ("rules-core", None),
        ("rules-dep-security", Some("dep")),
        ("rules-spring", Some("spring")),
    ];

    fn load_repo_rules_example() -> RulesFile {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("javaguard.rules.toml");
        load_rules_file_raw(path.to_str().unwrap())
            .expect("javaguard.rules.toml must load")
            .expect("javaguard.rules.toml must exist")
    }

    fn load_repo_pack(dir_name: &str) -> (PathBuf, RulesFile) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join(dir_name);
        let manifest_path = root.join(rulepack::PACK_MANIFEST_FILE);
        let content = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let file: RulesFile = toml::from_str(&content)
            .unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));
        (root, file)
    }

    #[test]
    fn pack_manifests_match_rules_example() {
        let example = load_repo_rules_example();
        let example_by_id: std::collections::HashMap<&str, &RuleEntry> =
            example.rules.iter().map(|r| (r.id.as_str(), r)).collect();

        let mut pack_rules: Vec<(String, RuleEntry)> = Vec::new();
        for (dir_name, expect_ns) in DEFAULT_PACKS {
            let (root, file) = load_repo_pack(dir_name);

            // [pack] 元数据：name 与目录一致、版本契约声明完整、namespace 符合分层约定
            let meta = file.pack.as_ref().unwrap_or_else(|| {
                panic!("pack '{dir_name}': manifest must declare a [pack] section")
            });
            meta.validate().unwrap_or_else(|e| panic!("pack '{dir_name}': {e}"));
            assert_eq!(meta.name.as_deref(), Some(dir_name), "pack dir/name mismatch");
            assert!(meta.version.is_some(), "pack '{dir_name}' must declare version");
            assert_eq!(meta.namespace.as_deref(), expect_ns, "pack '{dir_name}' namespace");

            for rule in &file.rules {
                // 定制包默认全关：启用走 [[rule_packs.overrides]]，包内不得默认 enabled
                if expect_ns.is_some() {
                    assert!(
                        !rule.enabled,
                        "pack '{dir_name}' rule {} must default to enabled = false",
                        rule.id
                    );
                }
                pack_rules.push((dir_name.to_string(), rule.clone()));
            }
        }

        // 覆盖完备性：三包并集 == example（不重不漏）
        let pack_short_ids: Vec<String> =
            pack_rules.iter().map(|(_, r)| r.id.clone()).collect();
        let mut sorted = pack_short_ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            pack_short_ids.len(),
            "duplicate short id across default packs: {pack_short_ids:?}"
        );
        let mut example_ids: Vec<&str> = example_by_id.keys().copied().collect();
        example_ids.sort();
        assert_eq!(sorted, example_ids, "default packs union must equal javaguard.rules.toml");

        // 逐条比对：name / group / description / severity / enabled（core 包）/
        // 脚本内容（example 侧与包侧字节一致，防两份载体内容漂移）
        for (pack_name, rule) in &pack_rules {
            let ex = example_by_id
                .get(rule.id.as_str())
                .unwrap_or_else(|| panic!("rule {} missing in example", rule.id));
            assert_eq!(rule.name, ex.name, "rule {} name drift", rule.id);
            assert_eq!(rule.group, ex.group, "rule {} group drift", rule.id);
            assert_eq!(rule.description, ex.description, "rule {} description drift", rule.id);
            assert_eq!(rule.severity, ex.severity, "rule {} severity drift", rule.id);
            if *pack_name == "rules-core" {
                assert_eq!(rule.enabled, ex.enabled, "rule {} enabled drift", rule.id);
            }

            if rule.script_path.starts_with("builtin:") {
                assert_eq!(rule.script_path, ex.script_path, "rule {} builtin drift", rule.id);
                continue;
            }
            let pack_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join(pack_name);
            let pack_script = pack_root.join(&rule.script_path);
            let example_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&ex.script_path);
            let pack_bytes = std::fs::read(&pack_script)
                .unwrap_or_else(|e| panic!("read {}: {e}", pack_script.display()));
            let example_bytes = std::fs::read(&example_script)
                .unwrap_or_else(|e| panic!("read {}: {e}", example_script.display()));
            assert_eq!(
                pack_bytes, example_bytes,
                "rule {} script content drift between pack '{}' and example",
                rule.id, pack_name
            );
            // 脚本不得逃逸包根（与 rulepack::resolve_pack_script 同策略）
            let canon_root = pack_root.canonicalize().unwrap();
            let canon = pack_script.canonicalize().unwrap();
            assert!(
                canon.starts_with(&canon_root),
                "rule {} script escapes pack root '{}'",
                rule.id,
                pack_name
            );
        }

        // example 引用的所有脚本都存在（路径迁移后不留悬空引用）
        for rule in &example.rules {
            if rule.script_path.starts_with("builtin:") {
                continue;
            }
            let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&rule.script_path);
            assert!(p.is_file(), "example rule {} script missing: {}", rule.id, p.display());
        }
    }

    /// 三个默认包可直接经 `[rule_packs]` 加载：合并后的生效 id 与 example 一致
    /// （core 裸 id；dep/spring 规则为 `<ns>:<id>`），且 api/engine 契约对本引擎成立。
    #[test]
    fn default_packs_load_and_merge_to_example_set() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cfg = rulepack::RulePacksConfig {
            search_paths: vec![root.join("config").to_string_lossy().into_owned()],
            packs: DEFAULT_PACKS
                .iter()
                .map(|(name, _)| rulepack::PackRef {
                    name: (*name).to_string(),
                    version: None,
                    path: None,
                })
                .collect(),
            ..Default::default()
        };
        // 本地规则文件指向不存在的路径：合并结果应完全来自包层
        let merged = rulepack::resolve_and_merge(
            &cfg,
            &root.join("__no_local_rules__.toml"),
            &root,
        )
        .expect("default packs must resolve and merge");

        assert_eq!(merged.packs.len(), 3);
        let example = load_repo_rules_example();
        assert_eq!(merged.rules.len(), example.rules.len());

        // 包合并顺序 = packs 声明顺序，与 example 条目顺序不同，按短 id 映射比对
        let merged_by_short: std::collections::HashMap<String, &RuleEntry> = merged
            .rules
            .iter()
            .map(|r| (rulepack::short_id(&r.id).to_string(), r))
            .collect();
        assert_eq!(
            merged_by_short.len(),
            example.rules.len(),
            "short id must stay unique after namespace qualification"
        );
        for example_rule in &example.rules {
            let m = merged_by_short.get(example_rule.id.as_str()).unwrap_or_else(|| {
                panic!("rule {} missing in merged default packs", example_rule.id)
            });
            assert_eq!(m.severity, example_rule.severity);
            // enabled 仅对通用包（裸 id）要求与 example 一致；
            // 定制包（dep:*/spring:*）默认全关，属分层设计而非漂移
            if !m.id.contains(':') {
                assert_eq!(m.enabled, example_rule.enabled, "rule {} enabled drift", m.id);
            } else {
                assert!(!m.enabled, "namespaced rule {} must default to disabled", m.id);
            }
            // 包内脚本路径已解析为绝对路径且存在
            assert!(
                m.script_path.starts_with("builtin:")
                    || Path::new(&m.script_path).is_absolute(),
                "pack rule script must resolve to an absolute path: {}",
                m.script_path
            );
        }
        // sources 与规则一一对应：core 包规则带包名标签，本地缺失文件不产生来源
        assert_eq!(merged.sources.len(), merged.rules.len());
        for (r, s) in merged.rules.iter().zip(&merged.sources) {
            if r.id.contains(':') {
                assert!(
                    s == "pack 'rules-dep-security'" || s == "pack 'rules-spring'",
                    "namespaced rule {} sourced from unexpected '{s}'",
                    r.id
                );
            } else {
                assert_eq!(s, "pack 'rules-core'", "bare id {} unexpected source", r.id);
            }
        }
    }

    // ── M3: rules add / vendor 的文本编辑 helpers ─────────────────────────────

    fn temp_cfg_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("javaguard_maincfg_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_pack_to_config_creates_file_and_appends_entry() {
        let dir = temp_cfg_dir("append_create");
        let cfg = dir.join("java-guard.toml");

        append_pack_to_config(&cfg, "rules-spring", Some("1.0.0"), "../team/rules-spring")
            .expect("append into missing config must create it");

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("[[rule_packs.packs]]"));
        assert!(text.contains("name = \"rules-spring\""));
        assert!(text.contains("version = \"1.0.0\""));
        // Windows 反斜杠统一写成正斜杠
        assert!(!text.contains('\\'), "path must use forward slashes: {text}");
        // 必须仍是合法 TOML 且字段可解析
        let value: toml::Value = toml::from_str(&text).expect("generated config must parse");
        assert_eq!(
            value["rule_packs"]["packs"][0]["path"].as_str(),
            Some("../team/rules-spring")
        );
    }

    #[test]
    fn append_pack_to_config_rejects_duplicate_and_keeps_existing_content() {
        let dir = temp_cfg_dir("append_dup");
        let cfg = dir.join("java-guard.toml");
        std::fs::write(
            &cfg,
            "[scan]\nexclude = [\"target\"]\n\n[[rule_packs.packs]]\nname = \"rules-core\"\n",
        )
        .unwrap();

        // 同名重复 → 报错
        assert!(append_pack_to_config(&cfg, "rules-core", None, "config/rules-core").is_err());

        // 不同名 → 追加且原内容保持在前
        append_pack_to_config(&cfg, "rules-spring", None, "config/rules-spring").unwrap();
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("[scan]"));
        assert!(text.contains("exclude = [\"target\"]"));
        assert!(
            text.find("name = \"rules-core\"").unwrap()
                < text.find("name = \"rules-spring\"").unwrap()
        );
        let value: toml::Value = toml::from_str(&text).expect("appended config must parse");
        assert_eq!(value["rule_packs"]["packs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn ensure_search_path_first_appends_section_when_missing() {
        let dir = temp_cfg_dir("sp_missing");
        let cfg = dir.join("java-guard.toml");
        std::fs::write(&cfg, "[scan]\nmin_severity = \"info\"\n").unwrap();

        ensure_search_path_first(&cfg, "vendor/rules").unwrap();
        let text = std::fs::read_to_string(&cfg).unwrap();
        let value: toml::Value = toml::from_str(&text).expect("result must parse");
        assert_eq!(
            value["rule_packs"]["search_paths"][0].as_str(),
            Some("vendor/rules")
        );
        assert_eq!(value["scan"]["min_severity"].as_str(), Some("info"));
    }

    #[test]
    fn ensure_search_path_first_inserts_into_existing_section() {
        let dir = temp_cfg_dir("sp_insert");
        let cfg = dir.join("java-guard.toml");
        std::fs::write(
            &cfg,
            "[rule_packs]\nallow_remote = false\n\n[[rule_packs.packs]]\nname = \"rules-core\"\n",
        )
        .unwrap();

        ensure_search_path_first(&cfg, "vendor/rules").unwrap();
        let text = std::fs::read_to_string(&cfg).unwrap();
        // search_paths 必须落在 [rule_packs] 段内、packs 条目之前
        let sp = text.find("search_paths").expect("search_paths inserted");
        let packs = text.find("[[rule_packs.packs]]").unwrap();
        let header = text.find("[rule_packs]").unwrap();
        assert!(header < sp && sp < packs, "inserted at wrong place: {text}");
        let value: toml::Value = toml::from_str(&text).expect("result must parse");
        assert_eq!(
            value["rule_packs"]["search_paths"][0].as_str(),
            Some("vendor/rules")
        );
    }

    #[test]
    fn ensure_search_path_first_moves_existing_entry_to_front() {
        let dir = temp_cfg_dir("sp_front");
        let cfg = dir.join("java-guard.toml");
        std::fs::write(
            &cfg,
            "[rule_packs]\nsearch_paths = [\"//nas/shared-packs\", \"vendor/rules\"]\n",
        )
        .unwrap();

        ensure_search_path_first(&cfg, "vendor/rules").unwrap();
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("search_paths = [\"vendor/rules\", \"//nas/shared-packs\"]"));
    }

    #[test]
    fn insert_search_path_first_handles_empty_and_indented_lines() {
        assert_eq!(
            insert_search_path_first("search_paths = []", "vendor/rules"),
            "search_paths = [\"vendor/rules\"]"
        );
        assert_eq!(
            insert_search_path_first("  search_paths = [\"a\"]", "vendor/rules"),
            "  search_paths = [\"vendor/rules\", \"a\"]"
        );
        assert_eq!(
            insert_search_path_first("search_paths = [\"vendor/rules\", \"a\"]", "vendor/rules"),
            "search_paths = [\"vendor/rules\", \"a\"]"
        );
    }
}
