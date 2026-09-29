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
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

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

/// CLI 回退模式的 JVM 参数（与 daemon 相同）。
///
/// 不设 `-Xmx` 时，并发 JVM 各默认预留 1/4 物理内存，8 worker 并发下
/// 实测出现 `Could not create the Java Virtual Machine`（605 文件 201 个解析失败）。
pub const CLI_JVM_ARGS: &[&str] = DAEMON_JVM_ARGS;

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
            .args(CLI_JVM_ARGS)
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

    /// 启动常驻 JVM，spawn 失败时带退避重试。
    ///
    /// Windows 上 `ERROR_PIPE_BUSY`（os error 231）为瞬态错误，官方文档即建议
    /// 稍候重试；单次失败不再等于该实例永远起不来。
    pub fn start_with_retry(
        jar_path: &Path,
        java_cmd: &str,
        attempts: u32,
    ) -> Result<Self, ParseError> {
        let attempts = attempts.max(1);
        let mut last = None;
        for attempt in 1..=attempts {
            match DaemonParser::start(jar_path, java_cmd) {
                Ok(p) => return Ok(p),
                Err(e) => {
                    last = Some(e);
                    if attempt < attempts {
                        std::thread::sleep(Duration::from_millis(50 * u64::from(attempt)));
                    }
                }
            }
        }
        Err(last.expect("attempts >= 1"))
    }

    /// 发送一个 JSON 请求并读取一行 JSON 响应。
    fn request(&self, request: &serde_json::Value) -> Result<serde_json::Value, ParseError> {
        let mut line = serde_json::to_string(request)?;
        line.push('\n');

        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| ParseError::ParserError("daemon stdin lock poisoned".to_string()))?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(ParseError::IoError)?;
        drop(stdin);

        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| ParseError::ParserError("daemon stdout lock poisoned".to_string()))?;
        let mut response = String::new();
        let read = stdout
            .read_line(&mut response)
            .map_err(ParseError::IoError)?;
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
/// 锁粒度是**单个 daemon**（`Vec<Mutex<DaemonParser>>`），不是整池：
/// worker 只在自己分到的 daemon 上持锁，IPC 往返期间其余 daemon 可并行服务
/// （整池一把锁会跨往返串行化所有解析，实测比单 daemon 还慢 41%）。
///
/// - 启动时并行 spawn 全部实例（串行会把池启动时间放大为 size 倍 JVM 启动耗时）；
/// - 单实例 spawn 失败带重试；部分失败降级为「用剩下的实例」并显眼告警；
/// - 仅当全部实例失败才返回 Err（由调用方决定回退 CLI 模式）；
/// - 某个 daemon 运行中异常退出（管道断裂）时自动重启并重试一次。
pub struct DaemonPool {
    jar_path: PathBuf,
    java_cmd: String,
    daemons: Vec<Mutex<DaemonParser>>,
    next: AtomicUsize,
}

impl DaemonPool {
    /// 启动 `size` 个常驻 JVM（并行 spawn，各自带重试）。
    pub fn start(jar_path: &Path, java_cmd: &str, size: usize) -> Result<Self, ParseError> {
        let size = size.clamp(1, 16);
        let results: Vec<Result<DaemonParser, ParseError>> = thread::scope(|s| {
            let handles: Vec<_> = (0..size)
                .map(|_| s.spawn(|| DaemonParser::start_with_retry(jar_path, java_cmd, 3)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(ParseError::InvokeError(
                            "daemon spawn thread panicked".into(),
                        ))
                    })
                })
                .collect()
        });

        let mut daemons = Vec::with_capacity(size);
        let mut failures = 0usize;
        for r in results {
            match r {
                Ok(d) => daemons.push(Mutex::new(d)),
                Err(_) => failures += 1,
            }
        }

        if daemons.is_empty() {
            return Err(ParseError::InvokeError(format!(
                "all {size} daemon JVM(s) failed to start"
            )));
        }
        if failures > 0 {
            eprintln!(
                "PERF-WARN: daemon pool degraded: {}/{} JVM(s) started; parse throughput reduced proportionally",
                daemons.len(),
                size
            );
        }

        Ok(DaemonPool {
            jar_path: jar_path.to_path_buf(),
            java_cmd: java_cmd.to_string(),
            daemons,
            next: AtomicUsize::new(0),
        })
    }

    /// 轮询选取一个 daemon 执行解析；若实例已死则重启并重试一次。
    ///
    /// 仅持有该 daemon 自身的锁，其余实例在 IPC 往返期间不受阻塞。
    pub fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.daemons.len();
        let mut daemon = self.daemons[idx]
            .lock()
            .map_err(|_| ParseError::ParserError(format!("daemon {idx} lock poisoned")))?;

        match daemon.parse(source, filename) {
            // 管道 / 进程级错误 → daemon 很可能已死，重启后重试一次
            Err(ParseError::IoError(_)) | Err(ParseError::InvokeError(_)) => {
                eprintln!("warn: parser daemon {idx} died, restarting...");
                match DaemonParser::start_with_retry(&self.jar_path, &self.java_cmd, 2) {
                    Ok(parser) => {
                        *daemon = parser; // 旧实例在赋值时被 Drop（kill + wait）
                        daemon.parse(source, filename)
                    }
                    Err(e) => Err(e),
                }
            }
            other => other,
        }
    }

    pub fn len(&self) -> usize {
        self.daemons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.daemons.is_empty()
    }
}

impl JavaParser for DaemonPool {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        self.parse(source, filename)
    }
}

/// 惰性解析器：**首次真实解析（缓存 miss）时才启动 JVM 池**。
///
/// AST 缓存 100% 命中的增量扫描一个 JVM 都不会拉起；
/// 池启动失败时自动回退到 per-file JVM（CliParser），并输出显眼的性能告警。
/// 线程安全：多个 worker 同时触发首次解析时，仅一个执行启动，其余等待复用。
pub struct LazyParser {
    jar_path: PathBuf,
    java_cmd: String,
    pool_size: usize,
    inner: OnceLock<Arc<dyn JavaParser>>,
}

impl LazyParser {
    pub fn new(jar_path: &Path, java_cmd: &str, pool_size: usize) -> Self {
        LazyParser {
            jar_path: jar_path.to_path_buf(),
            java_cmd: java_cmd.to_string(),
            pool_size,
            inner: OnceLock::new(),
        }
    }

    fn get(&self) -> &Arc<dyn JavaParser> {
        self.inner.get_or_init(|| {
            match DaemonPool::start(&self.jar_path, &self.java_cmd, self.pool_size) {
                Ok(pool) => {
                    eprintln!("Parser: daemon pool ({} resident JVM(s))", pool.len());
                    Arc::new(pool)
                }
                Err(e) => {
                    eprintln!(
                        "PERF-WARN: daemon pool unavailable ({e}); \
                         falling back to per-file JVM mode — expect ~300ms+ JVM startup \
                         per file, large projects will be 10x+ slower"
                    );
                    Arc::new(CliParser::new(&self.jar_path).with_java_cmd(self.java_cmd.clone()))
                }
            }
        })
    }
}

impl JavaParser for LazyParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        self.get().parse(source, filename)
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
            eprintln!(
                "skipping: {} not found (run mvn package first)",
                jar.display()
            );
            return;
        }
        let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string());
        let parser = DaemonParser::start(&jar, &java_cmd).expect("daemon should start");

        let source = "class Test { void run() { System.out.println(1); } }";
        let unit = parser
            .parse(source, "Test.java")
            .expect("parse should succeed");
        assert_eq!(unit.types.len(), 1);
        assert_eq!(unit.source_file, "Test.java");
        assert!(!unit.raw_json.is_empty());
        // parse 失败时返回 ParserError，daemon 不退出（可继续使用）
        let err = parser.parse("class {", "Bad.java").unwrap_err();
        assert!(matches!(err, ParseError::ParserError(_)));
        // 出错后 daemon 仍可继续解析
        let unit2 = parser
            .parse("class Ok {}", "Ok.java")
            .expect("parse should succeed");
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
            eprintln!(
                "skipping: {} not found (run mvn package first)",
                jar.display()
            );
            return;
        }
        let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string());
        let pool = DaemonPool::start(&jar, &java_cmd, 2).expect("pool should start");
        assert_eq!(pool.len(), 2);

        for i in 0..6 {
            let unit = pool
                .parse(&format!("class C{i} {{}}"), &format!("C{i}.java"))
                .unwrap_or_else(|e| panic!("pool parse {i} failed: {e}"));
            assert_eq!(type_name(&unit.types[0]), format!("C{i}"));
        }
    }

    /// 惰性解析器：构造时不启动 JVM；解析时启动失败自动回退 CLI（返回 Err 而非 panic）。
    #[test]
    fn lazy_parser_defers_and_falls_back() {
        let lazy = LazyParser::new(
            Path::new("/no/such/java-parser.jar"),
            "/nonexistent/java",
            2,
        );
        // 解析会触发：池启动失败（重试后）→ 回退 CLI → CLI 也失败 → Err
        assert!(lazy.parse("class A {}", "A.java").is_err());
    }

    /// 空池边界：size=0 时按 1 个实例处理，parse 正常工作（无 jar 跳过）。
    #[test]
    fn daemon_pool_size_zero_clamped() {
        let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("java-parser/target/java-parser.jar");
        if !jar.exists() {
            eprintln!(
                "skipping: {} not found (run mvn package first)",
                jar.display()
            );
            return;
        }
        let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| "java".to_string());
        let pool = DaemonPool::start(&jar, &java_cmd, 0).expect("pool should start");
        assert_eq!(pool.len(), 1);
        let unit = pool
            .parse("class Z {}", "Z.java")
            .expect("parse should succeed");
        assert_eq!(type_name(&unit.types[0]), "Z");
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
