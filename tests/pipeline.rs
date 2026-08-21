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
        rule_ids.iter().any(|r| *r == "J001"),
        "expected J001 violations, got: {rule_ids:?}"
    );
    // J008 空 catch 块：RuleViolations.java / BadCode.java 命中
    assert!(
        rule_ids.iter().any(|r| *r == "J008"),
        "expected J008 (empty catch) violations, got: {rule_ids:?}"
    );
    // J003 禁止通配符 import：RuleViolations.java 的 `import java.util.*`
    assert!(
        rule_ids.iter().any(|r| *r == "J003"),
        "expected J003 (wildcard import) violations, got: {rule_ids:?}"
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
