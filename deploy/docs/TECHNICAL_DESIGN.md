# JavaGuard 技术方案

> ⚠️ **本文档描述目标架构（设计意图），与当前代码存在以下偏差，阅读时请注意：**
>
> | 设计描述 | 当前实现 | 状态 |
> |---------|---------|------|
> | `DaemonParser`（常驻 JVM，管道通信） | 仅实现 `CliParser`（每文件启动一个 JVM 进程 + 临时文件） | ❌ 未实现 |
> | AST 解析缓存（内容 hash） | 无 | ❌ 未实现 |
> | 并行解析（多个 DaemonParser 实例） | 文件级并行解析已通过 `std::thread::scope` 实现（受 CPU 核数限制） | 🟡 部分实现 |
> | YAML `any_of` / `name_regex` / `args_count` Pattern 字段 | 实际为 `match_fields` 映射（glob/正则/列表），见 RULE_AUTHORING.md | 🟡 API 不同 |
> | Rhai `check(node, ctx)` + `NodeWrapper` + `RuleContext` | 实际为脚本注入全局 `ast`（JSON map）并返回 ` violations` 数组 | 🟡 API 不同 |
> | `CatchBlock` Pattern + J002 方法过长 | J008 空 catch 为 Rust 内置规则；方法过长为 J006（Rhai） | 🟡 规则 ID/类型不同 |
>
> **规则编写请以 [RULE_AUTHORING.md](RULE_AUTHORING.md) 为准**（描述真实 API）。

## 1. 总体架构

```
┌──────────────────────────────────────────────────────────┐
│                     java-guard (CLI)                      │
│                   Rust 单二进制可执行文件                   │
│                                                          │
│  ┌──────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │  Config   │  │  Rule Engine │  │  Reporter          │ │
│  │  Loader   │  │  (调度+过滤)  │  │  (JSON/SARIF/CSV)  │ │
│  └──────────┘  └──────┬───────┘  └────────────────────┘ │
│                       │                                  │
│         ┌─────────────┼─────────────┐                   │
│         │             │             │                   │
│    ┌────▼────┐  ┌─────▼─────┐  ┌───▼──────────┐        │
│    │ YAML    │  │  Rhai     │  │ Java Plugin  │        │
│    │ Rules   │  │  Rules    │  │ (预留)        │        │
│    └────┬────┘  └─────┬─────┘  └───┬──────────┘        │
│         │             │             │                   │
│         └─────────────┼─────────────┘                   │
│                       │                                  │
│              ┌────────▼─────────┐                       │
│              │  AST Wrapper     │  ← Java AST 包装层     │
│              │  (节点统一模型)   │     (Rust struct)     │
│              └────────┬─────────┘                       │
│                       │                                  │
│              ┌────────▼─────────┐                       │
│              │  Parser Bridge   │  ← JNI / CLI 调用     │
│              └────────┬─────────┘                       │
│                       │                                  │
│              ┌────────▼─────────┐                       │
│              │  JavaParser jar  │  ← 独立 JVM 进程       │
│              │  (.java → JSON)  │     或常驻 JVM        │
│              └──────────────────┘                       │
│                                                          │
│  ┌──────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │ Git Diff │  │  File Walker │  │  Cache             │ │
│  │ (增量)   │  │  (文件扫描)   │  │  (AST 缓存)        │ │
│  └──────────┘  └──────────────┘  └────────────────────┘ │
└──────────────────────────────────────────────────────────┘

外部依赖:
  - java-parser.jar (JavaParser 封装，随包分发)
  - JRE 17+ (目标机器需预装)
```

## 2. 模块设计

### 2.1 Crate 拆分

```
java-guard/
├── Cargo.toml
├── crates/
│   ├── guard-core/          # 共享核心（从 SqlGuard 抽出）
│   │   ├── reporter/        #   报告输出（JSON/SARIF/CSV/控制台）
│   │   ├── config/          #   配置加载与校验
│   │   ├── git_diff/        #   git diff 集成
│   │   └── rule/            #   Rule trait + Violation 类型
│   ├── java-ast/            # Java AST 包装层
│   │   ├── ast.rs           #   统一节点模型
│   │   ├── bridge.rs        #   JavaParser 桥接（CLI/JSON）
│   │   └── cache.rs         #   AST 解析缓存
│   ├── rule-yaml/           # YAML 声明式规则引擎
│   │   ├── loader.rs        #   YAML 规则加载与编译
│   │   ├── matcher.rs       #   pattern 匹配器
│   │   └── patterns/        #   各 pattern 类型实现
│   ├── rule-rhai/           # Rhai 脚本规则引擎
│   │   ├── engine.rs        #   Rhai 引擎初始化
│   │   ├── bindings.rs      #   AST 节点 → Rhai 对象映射
│   │   └── context.rs       #   RuleContext 实现
│   └── rule-plugin/         # Java 插件加载（预留）
│       └── loader.rs        #   jar 加载与反射调用
├── src/
│   ├── main.rs              # CLI 入口
│   ├── cli.rs               # clap 定义
│   ├── scanner.rs           # 扫描调度器
│   └── gate.rs              # CI gate 逻辑
├── java-parser/             # Java 解析器 jar 源码
│   ├── pom.xml
│   └── src/main/java/
│       └── com/javaguard/parser/
│           ├── Main.java         # CLI 入口
│           ├── AstSerializer.java # AST → JSON 序列化
│           └── Config.java
├── rules/                   # 内置规则库
│   ├── yaml/                #   YAML 声明式规则
│   │   ├── J001-no-system-out.yml
│   │   ├── J003-no-wildcard-import.yml
│   │   └── ...
│   └── rhai/                #   Rhai 脚本规则
│       ├── J002-method-too-long.rhai
│       ├── J006-missing-override.rhai
│       └── ...
├── docs/
│   ├── REQUIREMENT.md
│   ├── TECHNICAL_DESIGN.md
│   └── RULE_AUTHORING.md    # 规则编写指南
└── tests/
    ├── integration/
    └── fixtures/            # 测试用 Java 样本
```

### 2.2 guard-core（从 SqlGuard 抽取共享）

从 SqlGuard 中提取以下模块为独立 crate，JavaGuard 和 SqlGuard 共同依赖：

```rust
// guard-core/src/lib.rs
pub mod reporter;   // Violation, ReportFormat, 输出器
pub mod config;     // 配置加载、路径展开、profile
pub mod git_diff;   // git diff 解析、文件列表、行范围
pub mod rule;       // Rule trait, Severity, Violation, RuleContext
```

**抽取策略**：guard-core 是 JavaGuard 项目内的新 crate，从 SqlGuard **借鉴设计**（类型定义、接口设计），但代码独立编写，不依赖 SqlGuard 仓库。等 JavaGuard 稳定后，再考虑将 guard-core 发布为独立 crate，SqlGuard 反向迁移依赖。

### 2.3 java-ast：AST 包装层

#### 统一节点模型

设计原则：**对规则脚本暴露简化的 struct，不直接暴露 JavaParser 的 AST 类型**。与 SqlGuard 对 sqlparser 的处理思路一致。

```rust
// java-ast/src/ast.rs

/// Java AST 根节点（对应一个 .java 文件的编译单元）
pub struct CompilationUnit {
    pub package: Option<String>,
    pub imports: Vec<ImportDecl>,
    pub types: Vec<TypeDecl>,        // 顶层类/接口/枚举/注解
    pub source_file: String,
    pub source_lines: Vec<String>,   // 原始源码行（用于行级匹配）
}

pub struct ImportDecl {
    pub package: String,
    pub is_wildcard: bool,
    pub is_static: bool,
    pub line: usize,
}

pub enum TypeDecl {
    Class(ClassDecl),
    Interface(InterfaceDecl),
    Enum(EnumDecl),
    Annotation(AnnotationDecl),
}

pub struct ClassDecl {
    pub name: String,
    pub modifiers: Vec<Modifier>,
    pub annotations: Vec<Annotation>,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub members: Vec<MemberDecl>,
    pub line: usize,
    pub end_line: usize,
}

pub enum MemberDecl {
    Field(FieldDecl),
    Method(MethodDecl),
    Constructor(ConstructorDecl),
    Initializer(InitializerDecl),
    NestedType(TypeDecl),
}

pub struct MethodDecl {
    pub name: String,
    pub modifiers: Vec<Modifier>,
    pub annotations: Vec<Annotation>,
    pub return_type: Option<String>,
    pub parameters: Vec<ParamDecl>,
    pub body: Option<BlockStmt>,
    pub line: usize,
    pub end_line: usize,
}

pub enum Stmt {
    Expression(ExprStmt),
    VariableDecl(VarDeclStmt),
    If(IfStmt),
    For(ForStmt),
    While(WhileStmt),
    Try(TryStmt),
    Return(ReturnStmt),
    Throw(ThrowStmt),
    Block(BlockStmt),
    // ...
}

pub enum Expr {
    MethodCall {
        callee: Option<String>,     // "System.out" 或 "obj"
        method_name: String,        // "println"
        arguments: Vec<Expr>,
        line: usize,
    },
    FieldAccess {
        target: Box<Expr>,
        field: String,
        line: usize,
    },
    Name(String, usize),
    Literal(Literal, usize),
    BinaryOp {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
        line: usize,
    },
    // ...
}

pub struct BlockStmt {
    pub statements: Vec<Stmt>,
    pub line: usize,
    pub end_line: usize,
}
```

#### Parser Bridge

```rust
// java-ast/src/bridge.rs

pub trait JavaParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError>;
}

/// 通过 CLI 调用 JavaParser jar
pub struct CliParser {
    jar_path: PathBuf,
    java_cmd: String,       // "java" 或自定义路径
    jvm_args: Vec<String>,  // ["-Xmx512m", "-jar", jar_path]
}

impl JavaParser for CliParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        // 1. 写源码到临时文件
        // 2. 调用 `java -jar java-parser.jar --input <tmp> --format json`
        // 3. 读取 stdout JSON
        // 4. 反序列化为 CompilationUnit
    }
}

/// 常驻 JVM 进程（通过 stdin/stdout 管道通信，避免每次启动 JVM）
pub struct DaemonParser {
    process: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl DaemonParser {
    pub fn start(jar_path: &Path) -> Result<Self, ParseError> {
        // 启动 `java -jar java-parser.jar --daemon`
        // 进入 REPL 模式：stdin 接收 JSON 请求，stdout 返回 JSON AST
    }
}

impl JavaParser for DaemonParser {
    fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
        // 通过管道发送 parse 请求，接收 JSON 响应
    }
}
```

**解析模式选择**：
- 单文件扫描 / 增量扫描：使用 `DaemonParser`（JVM 常驻，避免重复启动开销）
- 批量扫描：`DaemonParser` 实例池（2-4 个并行），并行解析多文件

### 2.4 rule-yaml：声明式规则引擎

#### YAML 规则结构

```yaml
# rules/yaml/J001-no-system-out.yml
id: J001
title: 禁止使用 System.out.println / System.err.println
severity: minor
category: code-smell
tags: [convention, logging]
pattern:
  type: MethodCall
  callee:
    any_of:
      - System.out
      - System.err
  method:
    any_of:
      - println
      - printf
      - print
message: "不要使用 {callee}.{method}，请使用日志框架（SLF4J）"
```

#### Pattern 匹配器

```rust
// rule-yaml/src/matcher.rs

pub trait Pattern {
    fn matches(&self, cu: &CompilationUnit) -> Vec<Match>;
}

pub struct MethodCallPattern {
    callee: Option<StringMatcher>,   // 支持 any_of / regex / exact
    method: Option<StringMatcher>,
    args_count: Option<Range<usize>>,
}

impl Pattern for MethodCallPattern {
    fn matches(&self, cu: &CompilationUnit) -> Vec<Match> {
        let mut results = Vec::new();
        // 遍历 CompilationUnit 中所有 Expr::MethodCall 节点
        for node in cu.walk_method_calls() {
            if let Some(callee) = &self.callee {
                if !callee.matches(&node.callee) { continue; }
            }
            if let Some(method) = &self.method {
                if !method.matches(&node.method_name) { continue; }
            }
            results.push(Match {
                line: node.line,
                message: self.render_message(node),
            });
        }
        results
    }
}

pub enum StringMatcher {
    Exact(String),
    AnyOf(Vec<String>),
    Regex(Regex),
}
```

#### 支持的 Pattern 类型

| Pattern 类型 | 匹配目标 | 示例规则 |
|-------------|---------|---------|
| `MethodCall` | 方法调用 | J001: 禁止 System.out |
| `Import` | import 语句 | J003: 禁止通配符 import |
| `Annotation` | 注解使用 | 检测 `@SuppressWarnings` 滥用 |
| `ClassDeclaration` | 类声明 | J004: 类名 PascalCase |
| `MethodDeclaration` | 方法声明 | J005: 方法名 camelCase |
| `FieldDeclaration` | 字段声明 | J007: 常量 UPPER_SNAKE |
| `CatchBlock` | catch 块 | J008: 禁止空 catch |

### 2.5 rule-rhai：脚本规则引擎

#### Rhai 引擎初始化

```rust
// rule-rhai/src/engine.rs

pub struct RhaiRuleEngine {
    engine: rhai::Engine,
    rules: Vec<RhaiRule>,
}

struct RhaiRule {
    id: String,
    severity: Severity,
    script: AST,  // Rhai 编译后的 AST
}

impl RhaiRuleEngine {
    pub fn new() -> Self {
        let mut engine = rhai::Engine::new();
        register_bindings(&mut engine);
        // ...
    }

    pub fn load_rule(&mut self, path: &Path) -> Result<(), RuleError> {
        let source = fs::read_to_string(path)?;
        let ast = self.engine.compile(&source)?;
        // 从脚本元数据提取 id / severity（约定脚本头部注释或专用函数）
        let metadata = extract_metadata(&ast)?;
        self.rules.push(RhaiRule { ... });
        Ok(())
    }

    pub fn check(&self, cu: &CompilationUnit, ctx: &mut RuleContext) {
        for rule in &self.rules {
            let node = wrap_compilation_unit(cu);
            let result = self.engine.call_fn::<()>(
                &mut Scope::new(),
                &rule.script,
                "check",
                (node, ctx.clone())
            );
            // ctx 在 Rhai 侧被修改，收集 Violation
        }
    }
}
```

#### AST 节点 → Rhai 对象映射

```rust
// rule-rhai/src/bindings.rs

#[derive(Clone)]
pub struct NodeWrapper {
    inner: Arc<CompilationUnit>,
    path: Vec<usize>,  // 节点在 AST 中的路径（用于定位）
}

impl NodeWrapper {
    pub fn kind(&self) -> String { ... }
    pub fn name(&self) -> String { ... }
    pub fn line(&self) -> usize { ... }
    pub fn end_line(&self) -> usize { ... }
    pub fn children(&self) -> Vec<NodeWrapper> { ... }
    pub fn parent(&self) -> Option<NodeWrapper> { ... }
    pub fn text(&self) -> String { ... }      // 原始源码文本
    pub fn get(&self, key: &str) -> Dynamic { ... } // 动态属性访问
}
```

Rhai 脚本中通过 `node.get("modifiers")` 访问属性，引擎返回 `Dynamic` 类型，无需为每种节点写专门的 Rhai 类型。

#### RuleContext

```rust
// rule-rhai/src/context.rs

#[derive(Clone)]
pub struct RuleContext {
    violations: Vec<Violation>,
    config: HashMap<String, Dynamic>,
    file_path: String,
}

impl RuleContext {
    pub fn report(&mut self, location: &str, message: &str, severity: &str) {
        self.violations.push(Violation { ... });
    }

    pub fn config(&self, key: &str, default: Dynamic) -> Dynamic {
        self.config.get(key).cloned().unwrap_or(default)
    }
}
```

### 2.6 rule-plugin：Java 插件加载（预留）

```rust
// rule-plugin/src/loader.rs

pub trait PluginRule: Send + Sync {
    fn id(&self) -> &str;
    fn severity(&self) -> Severity;
    fn analyze(&self, ast_json: &str, ctx: &mut PluginContext) -> Vec<Violation>;
}

pub struct PluginLoader {
    jar_path: PathBuf,
}

impl PluginLoader {
    /// 加载 jar 中所有实现 PluginRule 接口的类
    pub fn load(&self) -> Result<Vec<Box<dyn PluginRule>>, PluginError> {
        // 方案 A（MVP 后）：启动 JVM 子进程，通过 JSON-RPC 调用 jar 中的规则
        // 方案 B（长期）：JNI in-process，直接加载 jar（复杂度高）
        //
        // MVP 阶段只定义 trait 和空实现，不实际加载
        todo!("reserved for future implementation")
    }
}
```

### 2.7 Scanner：扫描调度器

```rust
// src/scanner.rs

pub struct Scanner {
    config: ScanConfig,
    yaml_engine: YamlRuleEngine,
    rhai_engine: RhaiRuleEngine,
    parser: Box<dyn JavaParser>,
    reporter: Reporter,
}

pub struct ScanConfig {
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub enabled_rules: HashSet<String>,
    pub disabled_rules: HashSet<String>,
    pub rule_params: HashMap<String, HashMap<String, Value>>,
    pub diff: Option<DiffConfig>,       // 增量扫描配置
    pub encoding: String,
}

pub struct DiffConfig {
    pub base_ref: String,               // "HEAD~1" 或 "main...feature"
    pub scope: DiffScope,               // Line / Method
}

impl Scanner {
    pub fn run(&self, root: &Path) -> Result<ScanResult, ScanError> {
        // 1. 收集文件列表（apply include/exclude）
        let files = self.collect_files(root)?;

        // 2. 如果增量模式，过滤变更文件 + 行范围
        let (files, line_filter) = if let Some(diff) = &self.config.diff {
            self.git_diff_filter(root, diff, files)?
        } else {
            (files, LineFilter::all())
        };

        // 3. 批量解析 AST
        let units = self.parse_files(&files)?;

        // 4. 执行规则
        let mut violations = Vec::new();
        for unit in &units {
            let mut ctx = RuleContext::new(&unit.source_file, &self.config.rule_params);

            self.yaml_engine.check(unit, &mut ctx);
            self.rhai_engine.check(unit, &mut ctx);

            // 行级过滤（增量扫描）
            for v in ctx.violations {
                if line_filter.allows(&unit.source_file, v.line) {
                    violations.push(v);
                }
            }
        }

        // 5. 生成报告
        Ok(ScanResult { violations, stats: ... })
    }

    fn parse_files(&self, files: &[PathBuf]) -> Result<Vec<CompilationUnit>, ParseError> {
        // 使用 DaemonParser 并行解析
        // 简单分批：files.chunks(batch_size)
    }
}
```

### 2.8 CLI 设计

```rust
// src/cli.rs

#[derive(Parser)]
#[clap(name = "java-guard", version, about = "Lightweight Java static analysis")]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 扫描 Java 代码
    Scan {
        /// 项目根目录
        #[clap(default_value = ".")]
        path: String,

        /// 配置文件路径
        #[clap(short = 'c', long)]
        config: Option<String>,

        /// 规则目录（可多次指定）
        #[clap(short = 'r', long)]
        rules: Vec<String>,

        /// 增量扫描：git diff 范围
        #[clap(long)]
        diff: Option<String>,

        /// 增量扫描范围粒度
        #[clap(long, default_value = "line")]
        diff_scope: DiffScope,

        /// 启用规则（覆盖配置文件）
        #[clap(long)]
        enable: Vec<String>,

        /// 禁用规则
        #[clap(long)]
        disable: Vec<String>,

        /// 最低严重级别
        #[clap(long, default_value = "info")]
        min_severity: Severity,

        /// 输出格式
        #[clap(short = 'f', long, default_value = "console")]
        format: Format,

        /// 输出文件
        #[clap(short = 'o', long)]
        output: Option<String>,

        /// Baseline 文件（只报告新增违规）
        #[clap(long)]
        baseline: Option<String>,

        /// CI gate 模式（违规超阈值时退出码 1）
        #[clap(long)]
        gate: bool,
    },

    /// 列出可用规则
    Rules {
        #[clap(short = 'r', long)]
        rules_dir: Vec<String>,
    },

    /// 初始化项目配置
    Init {
        #[clap(default_value = ".")]
        path: String,
    },
}
```

## 3. JavaParser CLI 设计

### 3.1 入口

```
java -jar java-parser.jar [options]

Options:
  --input <file|dir>     输入文件或目录
  --output <file>        输出 JSON 文件（默认 stdout）
  --format json          输出格式（MVP 只有 json）
  --daemon               常驻模式，通过 stdin/stdout 通信
  --encoding UTF-8       源码编码
```

### 3.2 JSON AST 格式

```json
{
  "kind": "CompilationUnit",
  "package": "com.example",
  "imports": [
    { "package": "java.util", "isWildcard": false, "isStatic": false, "line": 3 }
  ],
  "types": [
    {
      "kind": "ClassDeclaration",
      "name": "UserService",
      "modifiers": ["public"],
      "annotations": [
        { "name": "Service", "line": 5 }
      ],
      "extends": null,
      "implements": ["IUserService"],
      "line": 6,
      "endLine": 50,
      "members": [
        {
          "kind": "MethodDeclaration",
          "name": "findById",
          "modifiers": ["public"],
          "returnType": "User",
          "parameters": [
            { "type": "Long", "name": "id" }
          ],
          "body": {
            "kind": "BlockStmt",
            "line": 8,
            "endLine": 12,
            "statements": [ ... ]
          },
          "line": 7,
          "endLine": 12
        }
      ]
    }
  ],
  "sourceFile": "UserService.java"
}
```

### 3.3 Daemon 协议

请求（stdin，每行一个 JSON）：
```json
{"action": "parse", "filename": "Foo.java", "source": "..."}
```

响应（stdout，每行一个 JSON）：
```json
{"status": "ok", "ast": { ... }}
```

错误：
```json
{"status": "error", "message": "Parse error at line 5: unexpected token"}
```

## 4. 数据流

### 4.1 全量扫描流程

```
CLI 解析参数
  → 加载配置 (java-guard.yml)
  → 加载规则 (YAML + Rhai)
  → 收集 .java 文件 (apply include/exclude)
  → 启动 DaemonParser (JVM 子进程)
  → 分批解析文件 → CompilationUnit 列表
  → 对每个 CompilationUnit 执行规则:
      → YAML 规则: pattern matching
      → Rhai 规则: AST 遍历
  → 收集 Violation
  → 应用 baseline 过滤
  → 输出报告
  → CI gate 检查 → 设置退出码
```

### 4.2 增量扫描流程

```
CLI 解析参数 (--diff HEAD~1)
  → git diff 获取变更文件列表 + 行范围
  → 只解析变更文件
  → 执行规则
  → 行级过滤: 只保留变更行范围内的 Violation
  → 输出报告
```

### 4.3 AST 缓存

```
首次扫描:
  .java 文件 → hash(内容) → ast-cache/<hash>.json

后续扫描:
  .java 文件 → hash(内容) → 命中缓存 → 直接加载 AST
  
缓存失效:
  - 文件内容变更（hash 不匹配）
  - JavaParser 版本升级
  - 手动 --no-cache
```

## 5. 错误处理

### 5.1 解析错误
- JavaParser 解析失败：记录 `PARSE_ERROR`，跳过该文件，继续扫描其他文件
- 最终报告中包含解析错误列表

### 5.2 规则错误
- Rhai 脚本运行时异常：捕获，记录 `RULE_ERROR`，跳过该规则对该文件的处理
- YAML 规则格式错误：启动时校验，加载失败直接退出并报错

### 5.3 配置错误
- 配置文件格式错误：启动时报错，不执行扫描
- 规则 ID 冲突：启动时报错

## 6. 构建与分发

### 6.1 Rust 侧

```toml
# Cargo.toml
[workspace]
members = [
    "crates/guard-core",
    "crates/java-ast",
    "crates/rule-yaml",
    "crates/rule-rhai",
    "crates/rule-plugin",
    ".",  # 主 bin
]

[package]
name = "java-guard"
version = "0.1.0"
edition = "2021"

[dependencies]
guard-core = { path = "crates/guard-core" }
java-ast = { path = "crates/java-ast" }
rule-yaml = { path = "crates/rule-yaml" }
rule-rhai = { path = "crates/rule-rhai" }
rule-plugin = { path = "crates/rule-plugin" }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
rhai = "1.19"
regex = "1"
walkdir = "2"
```

### 6.2 Java 侧

```xml
<!-- java-parser/pom.xml -->
<project>
  <artifactId>java-parser</artifactId>
  <packaging>jar</packaging>
  <properties>
    <java.version>17</java.version>
    <javaparser.version>3.28.2</javaparser.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>com.github.javaparser</groupId>
      <artifactId>javaparser-symbol-solver-core</artifactId>
      <version>${javaparser.version}</version>
    </dependency>
    <dependency>
      <groupId>com.google.code.gson</groupId>
      <artifactId>gson</artifactId>
      <version>2.10.1</version>
    </dependency>
  </dependencies>
  <!-- maven-shade-plugin 打 fat jar -->
</project>
```

### 6.3 分发包结构

```
java-guard/
├── java-guard(.exe)       # Rust 引擎二进制
├── java-parser.jar        # JavaParser 封装
├── rules/                 # 内置规则库
│   ├── yaml/
│   └── rhai/
└── config-example.yml     # 配置模板
```

## 7. 性能优化策略

### 7.1 JVM 启动优化
- DaemonParser 常驻 JVM，避免每文件启动
- 批量解析：一次请求解析多个文件，减少 IPC 开销
- JVM 参数：`-Xshare:on`（CDS）、`-XX:TieredStopAtLevel=1`（C1 编译，快速启动）

### 7.2 并行扫描
- 文件解析并行：多个 DaemonParser 实例（默认 CPU 核数 - 1）
- 规则执行并行：同一文件的 CompilationUnit 可被多规则并行检查（CPU 密集型，用 rayon）

### 7.3 AST 缓存
- 基于文件内容 hash 缓存解析结果
- 缓存格式：JSON 文件，存储在 `.java-guard-cache/` 目录
- 增量扫描时自动复用未变更文件的缓存

## 8. 测试策略

### 8.1 单元测试
- `java-ast`：各类 Java 语法的 AST 解析正确性
- `rule-yaml`：每种 pattern 类型的匹配逻辑
- `rule-rhai`：Rhai 脚本执行 + Violation 收集

### 8.2 集成测试
- 端到端：Java 样本文件 → 扫描 → 验证 Violation
- 增量扫描：git 仓库 → diff → 变更行过滤
- 报告格式：各 format 的输出正确性

### 8.3 规则测试
每条内置规则附带测试用例：
```
rules/yaml/J001-no-system-out.yml
tests/fixtures/J001/
  ├── should_pass.java     # 不触发
  ├── should_fail.java     # 触发
  └── expected.json        # 期望的 Violation
```

## 9. 演进路线

| 阶段 | 内容 |
|------|------|
| **v0.1 (MVP)** | CLI + JavaParser 桥接 + YAML 规则 + Rhai 规则 + 控制台/JSON 报告 + 全量扫描 |
| **v0.2** | 增量扫描 + git diff + baseline |
| **v0.3** | SARIF/CSV 报告 + CI gate |
| **v0.4** | Java 插件接口 + Spoon 分析器（数据流规则） |
| **v0.5** | AST 缓存 + 并行扫描优化 |
| **v0.6** | `// noqa` 注释抑制 + 规则抑制配置 |
| **v1.0** | 规则市场（规则包发布与安装）+ IDE 插件（LSP） |

## 10. 与 SqlGuard 的关系

### 共享 (guard-core)
| 模块 | SqlGuard | JavaGuard |
|------|----------|-----------|
| reporter | ✅ 直接复用 | ✅ 直接复用 |
| config | ✅ 直接复用 | ✅ 直接复用 |
| git_diff | ✅ 直接复用 | ✅ 直接复用 |
| rule (Rule trait, Severity, Violation) | ✅ 直接复用 | ✅ 直接复用 |

### 独立
| 模块 | SqlGuard | JavaGuard |
|------|----------|-----------|
| 解析层 | sqlparser (Rust) | JavaParser (Java jar) |
| AST 包装层 | SQL AST → Rhai | Java AST → Rhai |
| 规则格式 | Rhai 脚本 | YAML + Rhai + Java 插件 |
| 扫描对象 | MyBatis Mapper XML | .java 文件 |

guard-core 的抽取不改动 SqlGuard 现有代码。先在 JavaGuard 中以 path dependency 引用本地 guard-core crate，后续考虑发布为独立 crate 或 git workspace。
