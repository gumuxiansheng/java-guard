# JavaGuard

轻量级 Java 代码静态扫描工具。规则可扩展，单二进制交付，增量扫描秒级完成。

## 快速开始

```bash
# 扫描当前目录
java-guard scan .

# 增量扫描（只检查最近一次提交的变更）
java-guard scan . --diff HEAD~1

# 指定规则目录和输出格式
java-guard scan . -r ./rules -f json -o report.json
```

## 文档

- [需求说明](docs/REQUIREMENT.md)
- [技术方案](docs/TECHNICAL_DESIGN.md)
- [规则编写指南](docs/RULE_AUTHORING.md)

## 架构概览

```
java-guard (Rust CLI)
  ├── guard-core    — 共享核心（reporter / config / git_diff / rule trait）
  ├── java-ast      — JavaParser 桥接 + AST 包装层
  ├── rule-yaml     — YAML 声明式规则引擎
  ├── rule-rhai     — Rhai 脚本规则引擎
  └── rule-plugin   — Java 插件接口（预留）
```

## 状态

✅ MVP 已实现（M1–M8 完成）：CLI 扫描、YAML / Rhai 规则引擎、增量扫描（git diff + baseline）、JSON / SARIF / CSV / 控制台报告、CI gate、Java 插件接口（预留）。

> ⚠️ 已知架构缺口（尚未实现，详见 [技术方案](docs/TECHNICAL_DESIGN.md) 顶部说明）：
> - `DaemonParser`（常驻 JVM）当前为 `CliParser`（每文件启动一个 JVM 进程），与「增量扫描 < 200ms / 全量 10w 行 < 10s」的性能目标有差距。
> - AST 解析缓存尚未实现。
> - 文件级解析已并行化（受 CPU 核数限制），但规则执行仍为单文件内串行。
