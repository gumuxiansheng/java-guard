# JavaGuard 开发计划

## 当前阶段：M8 — 全部里程碑已完成（M1–M8）

> 本计划最初将工作拆分为 M1–M8。截至 2026-07-31，所有里程碑均已落地：
> CLI 扫描、YAML / Rhai 规则引擎、增量扫描（git diff + baseline）、
> JSON / SARIF / CSV / 控制台报告、CI gate、Java 插件接口（预留）。

### M1 任务分解（已实现）

| # | 任务 | 语言 | 产出 |
|---|------|------|------|
| M1-1 | Rust workspace 骨架 | Rust | Cargo.toml + crates 结构 + 编译通过 |
| M1-2 | guard-core 基础类型 | Rust | Severity, Violation, Rule trait, ReportFormat |
| M1-3 | Java AST 模型定义 | Rust | CompilationUnit + 所有节点类型 |
| M1-4 | JavaParser CLI jar | Java | Main.java + AstSerializer.java + pom.xml |
| M1-5 | Parser Bridge（CLI 模式） | Rust | CliParser 实现 + JSON 反序列化 |
| M1-6 | 端到端测试 | Both | .java 文件 → JSON AST → Rust CompilationUnit |

### M1 完成标准（已满足）
- `java -jar java-parser.jar --input Foo.java` 输出正确 JSON AST
- `cargo test` 通过：Rust 侧能反序列化 JSON AST 为 CompilationUnit
- 基础 Java 语法覆盖：class/method/field/import/annotation/statement

## 里程碑总览（已全部完成）

| 阶段 | 内容 | 状态 |
|------|------|------|
| M2 | Rust CLI 壳 + 配置加载 + 文件扫描 + 控制台报告 | ✅ |
| M3 | YAML 声明式规则引擎 + 内置规则 | ✅ |
| M4 | Rhai 脚本规则 + AST 包装层 + 内置规则 | ✅ |
| M5 | 增量扫描 + git diff + baseline | ✅ |
| M6 | JSON/SARIF/CSV 报告 | ✅ |
| M7 | CI gate + 退出码 | ✅ |
| M8 | Java 插件接口（预留） | ✅ |

## 后续优化方向（非阻塞，待排期）

- **DaemonParser（常驻 JVM）**：当前为 `CliParser`，每文件启动一个 JVM 进程。
  与「增量单文件 < 200ms、全量 10w 行 < 10s」的 NFR 目标仍有差距，需实现常驻 JVM + 管道通信。
- **AST 解析缓存**：基于文件内容 hash 缓存 `CompilationUnit`，跳过未变更文件。
- **规则执行并行化**：当前仅在文件级并行解析，单文件内的多规则检查仍是串行。
- **`// noqa` 抑制与规则抑制配置**。

> 规则编写方式以 [RULE_AUTHORING.md](RULE_AUTHORING.md) 为准——该文档描述的是**当前真实实现的 API**，
> 而本文档与 [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) 描述的是**目标架构**（部分能力如 DaemonParser / AST 缓存尚未实现）。
