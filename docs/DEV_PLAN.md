# JavaGuard 开发计划

## 当前阶段：M1 — 解析层 + 项目骨架

### M1 任务分解

| # | 任务 | 语言 | 产出 |
|---|------|------|------|
| M1-1 | Rust workspace 骨架 | Rust | Cargo.toml + crates 结构 + 编译通过 |
| M1-2 | guard-core 基础类型 | Rust | Severity, Violation, Rule trait, ReportFormat |
| M1-3 | Java AST 模型定义 | Rust | CompilationUnit + 所有节点类型 |
| M1-4 | JavaParser CLI jar | Java | Main.java + AstSerializer.java + pom.xml |
| M1-5 | Parser Bridge（CLI 模式） | Rust | CliParser 实现 + JSON 反序列化 |
| M1-6 | 端到端测试 | Both | .java 文件 → JSON AST → Rust CompilationUnit |

### M1 完成标准
- `java -jar java-parser.jar --input Foo.java` 输出正确 JSON AST
- `cargo test` 通过：Rust 侧能反序列化 JSON AST 为 CompilationUnit
- 基础 Java 语法覆盖：class/method/field/import/annotation/statement

## 后续里程碑（概要）

| 阶段 | 内容 |
|------|------|
| M2 | Rust CLI 壳 + 配置加载 + 文件扫描 + 控制台报告 |
| M3 | YAML 声明式规则引擎 + 5 条内置规则 |
| M4 | Rhai 脚本规则 + AST 包装层 + 3 条内置规则 |
| M5 | 增量扫描 + git diff + baseline |
| M6 | JSON/SARIF/CSV 报告 |
| M7 | CI gate + 退出码 |
| M8 | Java 插件接口（预留） |
