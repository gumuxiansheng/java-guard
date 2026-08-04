# JavaGuard

轻量级 Java 代码静态扫描工具。规则可扩展，单二进制交付，增量扫描秒级完成。

## 快速开始

```bash
# 扫描当前目录
java-guard scan .

# 增量扫描（只检查最近一次提交的变更）
java-guard scan . --diff HEAD~1

# 输出 JSON 报告
java-guard scan . -f json -o report.json

# CI gate 模式（违规超阈值时退出码 1）
java-guard scan . --gate
```

## 安装

### 前置条件

- **JDK 8+**（java-parser.jar 运行时）

### 步骤

1. 从 `deploy/bin/` 获取 `java-guard` 二进制，加入 PATH
2. 从 `deploy/java-parser/` 获取 `java-parser.jar`
3. 设置环境变量：

   ```bash
   # Linux/macOS
   export JAVAGUARD_PARSER_JAR=/path/to/java-parser.jar

   # Windows PowerShell
   $env:JAVAGUARD_PARSER_JAR = "C:\path\to\java-parser.jar"
   ```

4. 从 `deploy/rules/` 获取规则文件，放置到项目根目录的 `rules/` 下

### 验证

```bash
java-guard version
java-guard rules
```

## 内置规则

| 规则 ID | 严重级别 | 类型 | 说明 |
|---------|---------|------|------|
| J001 | minor | YAML | 禁止 System.out / System.err 直接打印 |
| J003 | minor | YAML | import 不使用通配符 |
| J004 | minor | YAML | 类名使用 PascalCase |
| J005 | minor | YAML | 方法名使用 camelCase |
| J006 | minor | Rhai | 方法不超过 50 行（可配置） |
| J007 | minor | YAML | 常量使用 UPPER_SNAKE_CASE |
| J008 | major | Rust | 空 catch 块 |
| J009 | major | Rust | 潜在死循环检测 |

## 主要特性

### 三层规则引擎

- **YAML 声明式**：简单模式匹配，无需写代码
- **Rhai 脚本**：复杂逻辑，可访问完整 AST
- **Java 插件**（预留）：JSON-RPC 协议扩展

### 增量扫描

```bash
# git diff 模式：只扫描变更文件和行范围
java-guard scan . --diff HEAD~1

# baseline 模式：过滤已知违规
java-guard scan . --baseline baseline.json
```

### 报告格式

- **console**（默认）：彩色终端输出
- **json**：含扫描统计信息，适合程序处理
- **sarif**：SARIF 2.1.0，兼容 GitHub Code Scanning / Azure DevOps
- **csv**：表格格式

### CI 集成

```yaml
# GitHub Actions
- name: JavaGuard Scan
  run: java-guard scan . --gate --gate-config gate-config.yml
```

Gate 配置示例：

```yaml
gate:
  max_critical: 0
  max_major: 0
  max_minor: 10
  max_info: 50
```

## 架构

```
java-guard (Rust CLI 二进制, ~7.5MB)
  ├── guard-core    — 共享核心（reporter / gate / git_diff / rule trait）
  ├── java-ast      — JavaParser 桥接 + AST 包装层
  ├── rule-yaml     — YAML 声明式规则引擎
  ├── rule-rhai     — Rhai 脚本规则引擎
  └── rule-plugin   — Java 插件接口（预留）

java-parser.jar (~1.8MB)
  └── JavaParser 3.28.2，Java 源码 → JSON AST
```

工作流程：

```
.java 文件 → java-parser.jar (JVM) → JSON AST → Rust 规则引擎 → 违规报告
```

## 自定义规则

### YAML 规则

```yaml
id: NO_SYSTEM_OUT
title: "禁止使用 System.out"
severity: minor
pattern:
  type: MethodCall
  match_fields:
    callee: ["System.out", "System.err"]
    method: ["print", "println", "printf"]
message: "请使用日志框架（SLF4J）"
```

### Rhai 脚本规则

```yaml
id: CUSTOM001
title: "自定义规则"
severity: minor
params:
  threshold: 10
script: |
  let violations = [];
  let threshold = config["threshold"];
  if threshold == () { threshold = 10; }
  // 访问 ast JSON 进行检查
  violations
```

详见 [规则编写指南](docs/RULE_AUTHORING.md)。

## CLI 参数

```
java-guard scan [OPTIONS] [PATH]

Arguments:
  [PATH]                    扫描路径 [default: .]

Options:
  -f, --format <FORMAT>     报告格式：console/json/csv/sarif [default: console]
  -o, --output <OUTPUT>     输出到文件
  -x, --exclude <EXCLUDE>   排除目录（逗号分隔）
  -I, --include <INCLUDE>   包含路径白名单（逗号分隔）
  -r, --rules-dir <DIR>     YAML 规则目录 [default: rules/]
      --diff <SPEC>         增量扫描：git diff 范围
      --baseline <FILE>     Baseline 文件（只报告新增违规）
      --gate                CI gate 模式
      --gate-config <FILE>  Gate 配置文件
      --enable <IDS>        启用规则（逗号分隔）
      --disable <IDS>       禁用规则（逗号分隔）
      --min-severity <LVL>  最低严重级别 [default: info]
      --parser-jar <PATH>   java-parser.jar 路径 [env: JAVAGUARD_PARSER_JAR]
      --java-cmd <PATH>     Java 运行时路径 [env: JAVA_CMD]
      --config <PATH>       配置文件 [default: java-guard.yml]
```

## 文档

- [用户手册](deploy/docs/user-manual.md) — 安装、使用、CLI 参考、规则说明、CI 集成
- [需求说明](docs/REQUIREMENT.md) — 功能需求与里程碑
- [技术方案](docs/TECHNICAL_DESIGN.md) — 架构设计与模块说明
- [规则编写指南](docs/RULE_AUTHORING.md) — YAML / Rhai 规则开发

## 状态

✅ MVP 已实现（M1–M8 完成，125 tests 全通过）：
- CLI 扫描、YAML / Rhai 规则引擎
- 增量扫描（git diff + baseline）
- JSON / SARIF / CSV / 控制台报告
- CI gate（阈值退出码）
- Java 插件接口（预留）

> **已知架构缺口**（详见 [技术方案](docs/TECHNICAL_DESIGN.md)）：
> - `DaemonParser`（常驻 JVM）当前为 `CliParser`（每文件启动 JVM），大项目性能待优化
> - AST 解析缓存尚未实现
> - 规则执行为单文件内串行（文件级已并行化）

## License

MIT
