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

🚧 设计阶段，尚未实现。详见 [技术方案](docs/TECHNICAL_DESIGN.md)。
