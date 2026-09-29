//! 端到端流水线集成测试。
//!
//! 运行真实编译产物 `java-guard`，对 `tests/fixtures/` 下的 Java 文件跑完整流水线
//! （扫描 → 启动 JVM 解析 → 匹配 YAML/Rhai/内置规则 → 生成 JSON 报告），
//! 验证「规则真的能拦住坏代码」这一最关键的链路。
//!
//! 依赖：java-parser.jar 已构建（`mvn package`）且系统能调用 `java`。
//! 二者缺失时测试静默跳过（避免无 JVM 环境假绿，但本机已具备）。

use std::path::PathBuf;
use std::process::Command;

/// 从 `java -version` 输出中解析主版本号。
///
/// 输出形如 `java version "1.8.0_202"` / `openjdk version "17.0.9"` / `java version "22.0.2"`。
fn parse_major_version(version_output: &str) -> Option<u32> {
    let quoted = version_output.split('"').nth(1)?;
    let mut parts = quoted.split(['.', '_', '-']);
    let first = parts.next()?;
    // 老式 `1.8.0` 记法：主版本在第二段
    if first == "1" {
        parts.next()?.parse().ok()
    } else {
        first.parse().ok()
    }
}

/// 探测可用且版本足够的 java 命令。
///
/// java-parser.jar 现已以 Java 8 为目标编译（class file version 52），
/// 因此 JDK 8 及以上均可运行。这里优先挑选 JDK 8 以验证向后兼容，
/// 更高版本作为兜底。
fn java_cmd() -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(v) = std::env::var("JAVAGUARD_TEST_JAVA") {
        candidates.push(v);
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        candidates.push(format!("{home}/bin/java"));
    }
    // 优先使用 JDK 8，验证向后兼容（本机默认 PATH java 即 JDK 8）
    candidates.push(r"C:\Program Files\Java\jdk1.8.0_202\bin\java.exe".to_string());
    candidates.push("java".to_string());
    // 更高版本作为兜底
    candidates.push(r"C:\Program Files\Java\jdk-17\bin\java.exe".to_string());
    candidates.push(r"C:\Program Files\Graalvm\graalvm-jdk-22.0.2+9.1\bin\java.exe".to_string());

    for cand in candidates {
        // 带路径的候选先判断文件是否存在，避免无谓 spawn
        if cand.contains(['/', '\\']) && !PathBuf::from(&cand).exists() {
            continue;
        }
        let Ok(out) = Command::new(&cand).arg("-version").output() else {
            continue;
        };
        // `java -version` 写的是 stderr
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        if parse_major_version(&text).is_some_and(|v| v >= 8) {
            return Some(cand);
        }
    }
    None
}

#[test]
fn parse_major_version_handles_old_and_new_schemes() {
    assert_eq!(parse_major_version(r#"java version "1.8.0_202""#), Some(8));
    assert_eq!(parse_major_version(r#"openjdk version "17.0.9""#), Some(17));
    assert_eq!(parse_major_version(r#"java version "22.0.2" 2024-07-16"#), Some(22));
    assert_eq!(parse_major_version("no version here"), None);
}

#[test]
fn pipeline_reports_violations_on_fixtures() {
    let bin = env!("CARGO_BIN_EXE_java-guard");
    let manifest = env!("CARGO_MANIFEST_DIR");

    let jar = PathBuf::from(manifest).join("java-parser/target/java-parser.jar");
    if !jar.exists() {
        eprintln!("skip: {} not built (run mvn package)", jar.display());
        return;
    }
    let java = match java_cmd() {
        Some(j) => j,
        None => {
            eprintln!("skip: java runtime not available");
            return;
        }
    };

    let fixtures = PathBuf::from(manifest).join("tests/fixtures");
    let rules_file = PathBuf::from(manifest).join("javaguard.rules.toml");

    let output = Command::new(bin)
        .arg("scan")
        .arg(&fixtures)
        .arg("-f")
        .arg("json")
        .arg("--parser-jar")
        .arg(&jar)
        .arg("--rules-file")
        .arg(&rules_file)
        // 用不存在的配置文件，确保不读取 cwd 下任何 java-guard.toml 干扰断言
        .arg("--config")
        .arg("__none__.toml")
        .env("JAVA_CMD", &java)
        .output()
        .expect("failed to execute java-guard");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "binary exited non-zero\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        stdout
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON report");
    let violations = parsed["violations"].as_array().expect("violations array");
    assert!(
        !violations.is_empty(),
        "expected at least one violation, got empty report: {stdout}"
    );

    let rule_ids: Vec<&str> = violations
        .iter()
        .map(|v| v["rule_id"].as_str().unwrap_or(""))
        .collect();

    // J001 禁止 System.out.println：三个 fixture 均命中
    assert!(
        rule_ids.contains(&"J001"),
        "expected J001 violations, got: {rule_ids:?}"
    );
    // J008 空 catch 块：RuleViolations.java / BadCode.java 命中
    assert!(
        rule_ids.contains(&"J008"),
        "expected J008 (empty catch) violations, got: {rule_ids:?}"
    );
    // J003 禁止通配符 import：RuleViolations.java 的 `import java.util.*`
    assert!(
        rule_ids.contains(&"J003"),
        "expected J003 (wildcard import) violations, got: {rule_ids:?}"
    );
    // J016 catch 中静默抛错（未先记录日志）：RuleViolations.java 的 silentRethrow 命中
    assert!(
        rule_ids.contains(&"J016"),
        "expected J016 (rethrow without logging) violations, got: {rule_ids:?}"
    );
    // J017 禁止直接使用日志实现：RuleViolations.java 的 org.apache.log4j.Logger 命中
    assert!(
        rule_ids.contains(&"J017"),
        "expected J017 (direct log impl import) violations, got: {rule_ids:?}"
    );

    // 每条 violation 都应带合法行号与文件路径
    for v in violations {
        assert!(v["line"].as_u64().unwrap_or(0) > 0, "violation missing line: {v}");
        assert!(!v["file"].as_str().unwrap_or("").is_empty(), "violation missing file: {v}");
    }
}

/// 端到端验证 J009 死循环规则在「真实 JVM 解析路径」下的行为：
///
/// - 死循环（`for(;;)` / `while(true)` / `for` 无更新）必须被捕获；
/// - 正常 `for (int i = 0; i < n; i++)` 不得被误报（回归：旧序列化器丢弃
///   ForStmt 的 condition，曾把所有 for 循环都判为死循环）。
///
/// 使用 `tests/fixtures/LoopCases.java` 作为单一输入，行号与该 fixture 严格对应。
#[test]
fn pipeline_j009_infinite_loop_real_parse() {
    let bin = env!("CARGO_BIN_EXE_java-guard");
    let manifest = env!("CARGO_MANIFEST_DIR");

    let jar = PathBuf::from(manifest).join("java-parser/target/java-parser.jar");
    if !jar.exists() {
        eprintln!("skip: {} not built (run mvn package)", jar.display());
        return;
    }
    let java = match java_cmd() {
        Some(j) => j,
        None => {
            eprintln!("skip: java runtime not available");
            return;
        }
    };

    let fixture = PathBuf::from(manifest).join("tests/fixtures/LoopCases.java");
    let rules_file = PathBuf::from(manifest).join("javaguard.rules.toml");

    let output = Command::new(bin)
        .arg("scan")
        .arg(&fixture)
        .arg("-f")
        .arg("json")
        .arg("--parser-jar")
        .arg(&jar)
        .arg("--rules-file")
        .arg(&rules_file)
        .arg("--config")
        .arg("__none__.toml")
        .env("JAVA_CMD", &java)
        .output()
        .expect("failed to execute java-guard");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "binary exited non-zero\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        stdout
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON report");
    let violations = parsed["violations"].as_array().expect("violations array");

    let j009_lines: Vec<u64> = violations
        .iter()
        .filter(|v| v["rule_id"].as_str() == Some("J009"))
        .map(|v| v["line"].as_u64().unwrap_or(0))
        .collect();

    // 三处死循环必须被捕获
    assert!(
        j009_lines.contains(&13),
        "expected J009 at LoopCases.java:13 (for(;;)), got: {j009_lines:?}\nstdout: {stdout}"
    );
    assert!(
        j009_lines.contains(&20),
        "expected J009 at LoopCases.java:20 (while(true)), got: {j009_lines:?}\nstdout: {stdout}"
    );
    assert!(
        j009_lines.contains(&27),
        "expected J009 at LoopCases.java:27 (for without update), got: {j009_lines:?}\nstdout: {stdout}"
    );

    // 回归守护：正常 for 循环（第 6 行，带 i++ 更新）不得被误报
    assert!(
        !j009_lines.contains(&6),
        "REGRESSION: J009 falsely reported normal for loop at LoopCases.java:6\nstdout: {stdout}"
    );
}

/// 回归：被显式 `--enable` 的规则必须无视其 `enabled=false` 真正执行。
///
/// 仓库 `javaguard.rules.toml` 中 J104（布尔字段禁止 is 前缀）默认 `enabled=false`。
/// 历史上 `--enable` 只把规则留在列表里，却因 `run_rules` 的 `enabled()` 二次闸门
/// 仍被跳过 —— 表现为「0 rules enabled」。本测试锁定修复后的正确行为。
#[test]
fn pipeline_enable_overrides_disabled_flag() {
    let bin = env!("CARGO_BIN_EXE_java-guard");
    let manifest = env!("CARGO_MANIFEST_DIR");

    let jar = PathBuf::from(manifest).join("java-parser/target/java-parser.jar");
    if !jar.exists() {
        eprintln!("skip: {} not built", jar.display());
        return;
    }
    let java = match java_cmd() {
        Some(j) => j,
        None => {
            eprintln!("skip: java runtime not available");
            return;
        }
    };

    // 构造一个仅含 J104 触发点的临时 .java（isActive 命中；ok 不命中）
    let tmp = std::env::temp_dir().join("javaguard_enable_test");
    let _ = std::fs::create_dir_all(&tmp);
    let java_file = tmp.join("Sample.java");
    std::fs::write(
        &java_file,
        "package demo;\npublic class Sample {\n    private boolean isActive;\n    private boolean ok;\n}\n",
    )
    .unwrap();

    let rules_file = PathBuf::from(manifest).join("javaguard.rules.toml");

    // 1) 显式启用 J104 → 必须命中 isActive
    let out_enabled = Command::new(bin)
        .arg("scan")
        .arg(&tmp)
        .arg("-f")
        .arg("json")
        .arg("--parser-jar")
        .arg(&jar)
        .arg("--rules-file")
        .arg(&rules_file)
        .arg("--config")
        .arg("__none__.toml")
        .arg("--enable")
        .arg("J104")
        .arg("--no-cache")
        .env("JAVA_CMD", &java)
        .output()
        .expect("failed to execute java-guard");
    let stdout_enabled = String::from_utf8_lossy(&out_enabled.stdout);
    let parsed_en: serde_json::Value = serde_json::from_str(&stdout_enabled)
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}\n{stdout_enabled}"));
    let empty_en: Vec<serde_json::Value> = Vec::new();
    let ids_en: Vec<&str> = parsed_en["violations"]
        .as_array()
        .unwrap_or(&empty_en)
        .iter()
        .map(|v| v["rule_id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        ids_en.contains(&"J104"),
        "J104 must fire when explicitly --enable'd (enabled=false in toml)\nids: {ids_en:?}\n{stdout_enabled}"
    );

    // 2) 不启用 → J104 必须落空（默认 enabled=false 被列表层过滤移除）
    let out_default = Command::new(bin)
        .arg("scan")
        .arg(&tmp)
        .arg("-f")
        .arg("json")
        .arg("--parser-jar")
        .arg(&jar)
        .arg("--rules-file")
        .arg(&rules_file)
        .arg("--config")
        .arg("__none__.toml")
        .arg("--no-cache")
        .env("JAVA_CMD", &java)
        .output()
        .expect("failed to execute java-guard");
    let stdout_default = String::from_utf8_lossy(&out_default.stdout);
    let parsed_de: serde_json::Value = serde_json::from_str(&stdout_default)
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}\n{stdout_default}"));
    let empty_de: Vec<serde_json::Value> = Vec::new();
    let ids_de: Vec<&str> = parsed_de["violations"]
        .as_array()
        .unwrap_or(&empty_de)
        .iter()
        .map(|v| v["rule_id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !ids_de.contains(&"J104"),
        "J104 must NOT fire by default (enabled=false)\nids: {ids_de:?}\n{stdout_default}"
    );
}
