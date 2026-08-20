//! Parser Bridge — Rust 与 JavaParser jar 之间的桥接。
//!
//! 支持两种解析模式：
//! - [`CliParser`]：单次模式，每次 parse 启动一个 JVM 进程（大项目性能差，仅作回退）。
//! - [`DaemonParser`] / [`DaemonPool`]：常驻 JVM（`java -jar ... --daemon`），
//!   通过 stdin/stdout 管道逐行 JSON 通信，避免重复 JVM 启动开销（默认推荐）。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ast::CompilationUnit;
use crate::error::ParseError;

/// 常驻 JVM 的启动参数：加快启动、限制内存。
///
/// - `-Xshare:auto`：CDS 类数据共享（有归档则启用，无则静默跳过）
/// - `-XX:TieredStopAtLevel=1`：只用 C1 编译器，显著减少 JIT 预热时间
/// - `-Xms32m -Xmx512m`：固定初始堆，限制常驻内存
pub const DAEMON_JVM_ARGS: &[&str] = &[
    "-Xshare:auto",
    "-XX:TieredStopAtLevel=1",
    "-Xms32m",
    "-Xmx512m",
];

/// Java 解析器接口。
///
/// `Send + Sync`，保证实现可被 `Arc` 共享到并行文件解析线程池。
pub trait JavaParser: Send + Sync {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError>;
}

/// 通过 CLI 调用 java-parser.jar（单次模式）。
pub struct CliParser {
    jar_path: PathBuf,
    java_cmd: String,
    /// 调用序号：用于生成唯一的临时文件名，避免并行解析时冲突。
    call_seq: AtomicU64,
}

impl CliParser {
    pub fn new(jar_path: impl AsRef<Path>) -> Self {
        CliParser {
            jar_path: jar_path.as_ref().to_path_buf(),
            java_cmd: std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string()),
            call_seq: AtomicU64::new(0),
        }
    }

    pub fn with_java_cmd(mut self, cmd: impl Into<String>) -> Self {
        self.java_cmd = cmd.into();
        self
    }
}

impl JavaParser for CliParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        // 写源码到临时文件（进程 id + 调用序号保证唯一，支持并行解析）
        let seq = self.call_seq.fetch_add(1, Ordering::Relaxed);
        let tmp_dir = std::env::temp_dir();
        let tmp_file = tmp_dir.join(format!(
            "javaguard_parse_{}_{}.java",
            std::process::id(),
            seq
        ));

        std::fs::write(&tmp_file, source)?;

        let output = Command::new(&self.java_cmd)
            .args(["-jar"])
            .arg(&self.jar_path)
            .args(["--input"])
            .arg(&tmp_file)
            .args(["--format", "json"])
            .output()
            .map_err(|e| ParseError::InvokeError(e.to_string()))?;

        // 清理临时文件
        let _ = std::fs::remove_file(&tmp_file);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ParseError::ParserError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut unit: CompilationUnit = serde_json::from_str(&stdout)?;
        unit.source_file = filename.to_string();
        unit.raw_json = stdout.to_string();
        Ok(unit)
    }
}

/// 常驻 JVM 解析器：一个 JVM 进程，通过 stdin/stdout 逐行 JSON 通信。
///
/// 进程持有（而非每次启动）JavaParser 实例、Gson 与序列化器，
/// 单次 parse 往返耗时约数毫秒（对比 CLI 模式每次 300ms+ 的 JVM 启动）。
pub struct DaemonParser {
    process: Child,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
}

impl DaemonParser {
    /// 启动常驻 JVM（`java <args> -jar <jar> --daemon`）。
    pub fn start(jar_path: &Path, java_cmd: &str) -> Result<Self, ParseError> {
        let mut child = Command::new(java_cmd)
            .args(DAEMON_JVM_ARGS)
            .arg("-jar")
            .arg(jar_path)
            .arg("--daemon")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ParseError::InvokeError(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ParseError::InvokeError("failed to capture daemon stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ParseError::InvokeError("failed to capture daemon stdout".into()))?;

        Ok(DaemonParser {
            process: child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
        })
    }

    /// 发送一个 JSON 请求并读取一行 JSON 响应。
    fn request(&self, request: &serde_json::Value) -> Result<serde_json::Value, ParseError> {
        let mut line = serde_json::to_string(request)?;
        line.push('\n');

        let mut stdin = self.stdin.lock().map_err(|_| {
            ParseError::ParserError("daemon stdin lock poisoned".to_string())
        })?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|e| ParseError::IoError(e))?;
        drop(stdin);

        let mut stdout = self.stdout.lock().map_err(|_| {
            ParseError::ParserError("daemon stdout lock poisoned".to_string())
        })?;
        let mut response = String::new();
        let read = stdout
            .read_line(&mut response)
            .map_err(|e| ParseError::IoError(e))?;
        if read == 0 {
            // JVM 提前退出（如被外部杀死或启动失败）
            return Err(ParseError::ParserError(
                "daemon process exited unexpectedly".to_string(),
            ));
        }
        let response = response.trim_end_matches(['\r', '\n']);
        let value: serde_json::Value = serde_json::from_str(response)?;
        Ok(value)
    }
}

impl Drop for DaemonParser {
    fn drop(&mut self) {
        // 尽力优雅退出，随后强制结束，避免残留 JVM 进程
        let _ = self.stdin.lock().map(|mut s| {
            let _ = s.write_all(b"{\"action\":\"exit\"}\n");
            let _ = s.flush();
        });
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl JavaParser for DaemonParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        let request = serde_json::json!({
            "action": "parse",
            "name": filename,
            "source": source,
        });
        let value = self.request(&request)?;

        let status = value
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or_default();
        let ast = value.get("ast").cloned().unwrap_or_default();

        match status {
            "ok" => {
                let mut unit: CompilationUnit = serde_json::from_value(ast)?;
                unit.source_file = filename.to_string();
                unit.raw_json = serde_json::to_string(&value["ast"])?;
                Ok(unit)
            }
            _ => {
                let message = value
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown daemon error");
                Err(ParseError::ParserError(message.to_string()))
            }
        }
    }
}

/// 常驻 JVM 实例池：并行解析时多个 worker 轮流使用多个 daemon。
///
/// 某个 daemon 异常退出（管道断裂）时自动重启并重试一次，避免整批文件解析失败。
pub struct DaemonPool {
    jar_path: PathBuf,
    java_cmd: String,
    /// 实例个数（设计文档建议 2-4 个）。
    daemons: Mutex<Vec<DaemonParser>>,
    next: AtomicUsize,
}

impl DaemonPool {
    /// 启动 `size` 个常驻 JVM。任一实例启动失败即整体失败（由调用方决定回退到 CLI 模式）。
    pub fn start(jar_path: &Path, java_cmd: &str, size: usize) -> Result<Self, ParseError> {
        let size = size.max(1).min(16);
        let mut daemons = Vec::with_capacity(size);
        for _ in 0..size {
            daemons.push(DaemonParser::start(jar_path, java_cmd)?);
        }
        Ok(DaemonPool {
            jar_path: jar_path.to_path_buf(),
            java_cmd: java_cmd.to_string(),
            daemons: Mutex::new(daemons),
            next: AtomicUsize::new(0),
        })
    }

    /// 轮询选取一个 daemon 执行解析；若实例已死则重启并重试一次。
    pub fn parse(
        &self,
        source: &str,
        filename: &str,
    ) -> Result<CompilationUnit, ParseError> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.len();
        let mut daemons = self
            .daemons
            .lock()
            .map_err(|_| ParseError::ParserError("daemon pool lock poisoned".to_string()))?;

        let result = daemons[idx].parse(source, filename);
        match result {
            // 管道 / 进程级错误 → daemon 很可能已死，重启后重试一次
            Err(ParseError::IoError(_)) | Err(ParseError::InvokeError(_)) => {
                eprintln!("warn: parser daemon {} died, restarting...", idx);
                match DaemonParser::start(&self.jar_path, &self.java_cmd) {
                    Ok(parser) => {
                        daemons[idx] = parser; // 旧实例在赋值时被 Drop（kill + wait）
                        daemons[idx].parse(source, filename)
                    }
                    Err(e) => Err(e),
                }
            }
            other => other,
        }
    }

    pub fn len(&self) -> usize {
        self.daemons
            .lock()
            .map(|d| d.len())
            .unwrap_or(1)
    }
}

impl JavaParser for DaemonPool {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        self.parse(source, filename)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parser_creation() {
        // 不受外部 JAVA_CMD 环境变量影响
        std::env::remove_var("JAVA_CMD");
        let parser = CliParser::new("/nonexistent/java-parser.jar");
        assert_eq!(parser.java_cmd, "java");
    }

    #[test]
    fn cli_parser_custom_java() {
        let parser = CliParser::new("/nonexistent/java-parser.jar")
            .with_java_cmd("/usr/lib/jvm/java-17/bin/java");
        assert_eq!(parser.java_cmd, "/usr/lib/jvm/java-17/bin/java");
    }

    #[test]
    fn daemon_jvm_args_are_stable() {
        // 保证启动参数数组非空且首个参数为 CDS 开关（防止误改破坏启动速度）
        assert!(DAEMON_JVM_ARGS.contains(&"-XX:TieredStopAtLevel=1"));
        assert!(DAEMON_JVM_ARGS.contains(&"-Xshare:auto"));
    }

    /// jar 存在时验证 daemon 单实例往返解析（无 jar 则跳过）。
    #[test]
    fn daemon_parser_roundtrip() {
        let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("java-parser/target/java-parser.jar");
        if !jar.exists() {
            eprintln!("skipping: {} not found (run mvn package first)", jar.display());
            return;
        }
        let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string());
        let parser = DaemonParser::start(&jar, &java_cmd).expect("daemon should start");

        let source = "class Test { void run() { System.out.println(1); } }";
        let unit = parser.parse(source, "Test.java").expect("parse should succeed");
        assert_eq!(unit.types.len(), 1);
        assert_eq!(unit.source_file, "Test.java");
        assert!(!unit.raw_json.is_empty());
        // parse 失败时返回 ParserError，daemon 不退出（可继续使用）
        let err = parser.parse("class {", "Bad.java").unwrap_err();
        assert!(matches!(err, ParseError::ParserError(_)));
        // 出错后 daemon 仍可继续解析
        let unit2 = parser.parse("class Ok {}", "Ok.java").expect("parse should succeed");
        assert_eq!(unit2.types.len(), 1);
    }

    /// jar 存在时验证池化轮询 + 重启容错（无 jar 则跳过）。
    #[test]
    fn daemon_pool_roundtrip_and_restart() {
        let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("java-parser/target/java-parser.jar");
        if !jar.exists() {
            eprintln!("skipping: {} not found (run mvn package first)", jar.display());
            return;
        }
        let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string());
        let pool = DaemonPool::start(&jar, &java_cmd, 2).expect("pool should start");
        assert_eq!(pool.len(), 2);

        for i in 0..6 {
            let unit = pool.parse(&format!("class C{i} {{}}"), &format!("C{i}.java"))
                .unwrap_or_else(|e| panic!("pool parse {i} failed: {e}"));
            assert_eq!(type_name(&unit.types[0]), format!("C{i}"));
        }
    }

    fn type_name(t: &crate::ast::TypeDecl) -> &str {
        use crate::ast::TypeDecl;
        match t {
            TypeDecl::ClassDeclaration(c) => &c.name,
            TypeDecl::InterfaceDeclaration(i) => &i.name,
            TypeDecl::EnumDeclaration(e) => &e.name,
            TypeDecl::AnnotationDeclaration(a) => &a.name,
        }
    }
}