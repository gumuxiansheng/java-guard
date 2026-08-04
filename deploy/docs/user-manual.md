# JavaGuard 用户手册

> 版本 0.1.0 | 更新日期 2026-08-05

## 目录

1. [简介](#1-简介)
2. [安装](#2-安装)
3. [快速开始](#3-快速开始)
4. [命令参考](#4-命令参考)
5. [内置规则](#5-内置规则)
6. [报告格式](#6-报告格式)
7. [增量扫描](#7-增量扫描)
8. [CI Gate](#8-ci-gate)
9. [配置文件](#9-配置文件)
10. [自定义规则](#10-自定义规则)
11. [FAQ](#11-faq)

---

## 1. 简介

JavaGuard 是一个轻量级 Java 代码静态分析工具。特点：

- **单二进制交付**：Rust 编译为原生二进制，无需安装 Rust 工具链
- **三层规则引擎**：YAML 声明式 + Rhai 脚本 + Java 插件（预留）
- **增量扫描**：基于 git diff，只检查变更行
- **多格式报告**：控制台（彩色）、JSON、SARIF 2.1.0、CSV
- **CI 集成**：Gate 模式按阈值返回退出码，直接嵌入 CI/CD pipeline

### 架构

```
java-guard (Rust CLI 二进制)
  ├── guard-core    — 共享核心（reporter / gate / git_diff / rule trait）
  ├── java-ast      — JavaParser 桥接 + AST 包装层
  ├── rule-yaml     — YAML 声明式规则引擎
  ├── rule-rhai     — Rhai 脚本规则引擎
  └── rule-plugin   — Java 插件接口（预留）

java-parser.jar (Java 进程)
  └── 基于 JavaParser 3.28.2，将 Java 源码解析为 JSON AST
```

### 工作原理

```
Java 源码 → java-parser.jar (JVM) → JSON AST → Rust 规则引擎 → 违规报告
```

每次扫描时，`java-guard` 二进制为每个 `.java` 文件启动一个 JVM 进程调用 `java-parser.jar`，获取 JSON 格式的 AST，然后在 Rust 侧执行规则匹配。

---

## 2. 安装

### 前置条件

- **JDK 8+**（java-parser.jar 需要运行时；Java 8 即可，已以 class file version 52 编译）
- `java` 命令在 PATH 中可用

### 获取二进制

从 `deploy/` 目录获取：

```
deploy/
├── bin/
│   └── java-guard.exe      # Windows x64 二进制（约 7.5MB）
├── java-parser/
│   └── java-parser.jar     # Java AST 解析器（约 1.8MB）
├── rules/                  # 内置规则文件
│   ├── J001_no_system_out.yml
│   ├── J003_no_wildcard_import.yml
│   ├── J004_class_naming.yml
│   ├── J005_method_naming.yml
│   ├── J007_constant_naming.yml
│   └── rhai/
│       └── J006_long_method.yml
├── java-guard.yml.example  # 配置文件示例
├── gate-config.yml.example # Gate 配置示例
└── docs/
    └── user-manual.md      # 本文档
```

### 安装步骤

1. 将 `deploy/bin/` 加入系统 PATH（或记住完整路径）
2. 设置环境变量指向 `java-parser.jar`：

   **Windows (PowerShell)：**
   ```powershell
   $env:JAVAGUARD_PARSER_JAR = "C:\path\to\java-parser.jar"
   ```

   **Linux/macOS：**
   ```bash
   export JAVAGUARD_PARSER_JAR=/path/to/java-parser.jar
   ```

   或每次扫描时通过 `--parser-jar` 参数指定。

3. 将 `deploy/rules/` 复制到你的项目或固定位置。

### 验证安装

```bash
java-guard version
java-guard rules
```

---

## 3. 快速开始

### 扫描项目

```bash
# 扫描当前目录（递归查找 .java 文件）
java-guard scan .

# 扫描指定目录
java-guard scan src/main/java

# 扫描单个文件
java-guard scan src/Main.java
```

### 增量扫描

只检查最近一次提交变更的代码：

```bash
java-guard scan . --diff HEAD~1
```

### 输出 JSON 报告

```bash
java-guard scan . -f json -o report.json
```

### CI Gate 模式

```bash
# 有 major 违规则 CI 失败
java-guard scan . --gate

# 自定义阈值
java-guard scan . --gate --gate-config gate-config.yml
```

---

## 4. 命令参考

### `java-guard scan`

扫描 Java 代码。

```
Usage: java-guard scan [OPTIONS] [PATH]

Arguments:
  [PATH]  扫描路径（文件或目录） [default: .]

Options:
  -f, --format <FORMAT>              报告格式：console / json / csv / sarif [default: console]
  -o, --output <OUTPUT>              输出到文件（默认 stdout）
  -x, --exclude <EXCLUDE>            排除的目录名（逗号分隔）
                                     默认: target,build,.git,node_modules
  -I, --include <INCLUDE>            包含路径白名单（逗号分隔，如 src/main）
  -r, --rules-dir <RULES_DIR>        YAML 规则目录 [default: rules/]
      --diff <DIFF>                  增量扫描：git diff 范围（如 HEAD~1 或 main...feature）
      --baseline <BASELINE>          Baseline 文件路径（只报告新增违规）
      --gate                         CI gate 模式
      --gate-config <GATE_CONFIG>    Gate 配置文件（YAML）
      --enable <ENABLE>              启用规则（逗号分隔，覆盖默认全启用）
      --disable <DISABLE>            禁用规则（逗号分隔）
      --min-severity <MIN_SEVERITY>  最低严重级别 [default: info]
      --parser-jar <PARSER_JAR>      java-parser.jar 路径
                                     [env: JAVAGUARD_PARSER_JAR]
      --java-cmd <JAVA_CMD>          Java 运行时路径 [env: JAVA_CMD]
      --config <CONFIG>              配置文件路径 [default: java-guard.yml]
  -h, --help                         打印帮助
```

#### 严重级别

从高到低：

| 级别 | 说明 |
|------|------|
| `critical` | 严重问题，必须立即修复 |
| `major` | 重要问题，应尽快修复 |
| `minor` | 次要问题，建议修复 |
| `info` | 提示信息 |

`--min-severity` 过滤掉低于指定级别的违规。

### `java-guard rules`

列出所有可用规则（内置 + YAML + Rhai）。

### `java-guard version`

显示版本信息。

---

## 5. 内置规则

| 规则 ID | 严重级别 | 类型 | 说明 |
|---------|---------|------|------|
| J001 | minor | YAML | 禁止使用 System.out / System.err 直接打印 |
| J003 | minor | YAML | import 不使用通配符（`import xxx.*`） |
| J004 | minor | YAML | 类名使用 PascalCase（首字母大写） |
| J005 | minor | YAML | 方法名使用 camelCase（首字母小写） |
| J006 | minor | Rhai | 方法不超过 50 行（可配置） |
| J007 | minor | YAML | 常量使用 UPPER_SNAKE_CASE |
| J008 | major | Rust | 空 catch 块（异常被静默吞没） |
| J009 | major | Rust | 潜在死循环检测 |

### J009 死循环检测策略

J009 采用保守策略，仅在能确信循环不会终止时报告：

1. **确定性死循环**：条件编译期恒真（`while(true)`、`for(;;)`、常量传播 `final boolean T = true`）+ 条件变量循环内不被修改 + 无 `break`/`return`/`throw`
2. **for 计数器不推进**：缺 update 表达式 + 条件变量体内未修改 + 无退出
3. **for update 无效**：`i = i`、`i = 0`、`i += 0` 等不改变计数器的 update
4. **for update 方向矛盾**：`i++` 配 `i > 0` 条件、`i--` 配 `i < 10` 条件
5. **while/do 条件变量从不更新**：条件为单变量 + 体内从不修改 + 无退出

**保守原则**：`if (cond) break` 视为可能退出（不报）；`throw` 始终视为退出；lambda 内 `return` 不退出外层循环。

---

## 6. 报告格式

### Console（默认）

彩色终端输出，按文件分组，按 severity → file → line 排序：

```
src/Main.java (2 issues)
  15: major
    J008 empty catch block: exception is silently swallowed
  23: minor
    J001 不要使用 System.out.println，请使用日志框架（SLF4J）

2 violations in 1 file
Parsed 5 files, 0 errors, 2 violations
```

### JSON

```json
{
  "version": "1.0",
  "scan_info": {
    "timestamp": "2026-08-05T00:00:00Z",
    "files_scanned": 5,
    "parse_errors": 0,
    "duration_ms": 1234
  },
  "violations": [
    {
      "rule_id": "J008",
      "severity": "major",
      "file": "src/Main.java",
      "line": 15,
      "end_line": 18,
      "message": "empty catch block: exception is silently swallowed"
    }
  ],
  "stats": {
    "files_scanned": 5,
    "parse_errors": 0,
    "total_violations": 2,
    "by_severity": { "critical": 0, "major": 1, "minor": 1, "info": 0 },
    "by_rule": { "J008": 1, "J001": 1 }
  }
}
```

### SARIF 2.1.0

兼容 GitHub Code Scanning、Azure DevOps 等平台的 SARIF 格式：

```json
{
  "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/Schemata/sarif-schema-2.1.0.json",
  "runs": [{
    "results": [{
      "ruleId": "J008",
      "level": "error",
      "message": { "text": "empty catch block..." },
      "locations": [{
        "physicalLocation": {
          "artifactLocation": { "uri": "src/Main.java" },
          "region": { "startLine": 15, "endLine": 18 }
        }
      }]
    }],
    "tool": {
      "driver": {
        "name": "java-guard",
        "version": "0.1.0",
        "rules": [...]
      }
    }
  }],
  "version": "2.1.0"
}
```

严重级别映射：

| JavaGuard | SARIF level |
|-----------|-------------|
| critical  | error       |
| major     | error       |
| minor     | warning     |
| info      | note        |

### CSV

```
rule_id,severity,file,line,end_line,message
J008,major,src/Main.java,15,18,"empty catch block: exception is silently swallowed"
```

---

## 7. 增量扫描

### git diff 模式

只扫描 git 变更的文件和行范围：

```bash
# 扫描最近一次提交的变更
java-guard scan . --diff HEAD~1

# 扫描 feature 分支相对 main 的变更
java-guard scan . --diff main...HEAD

# 扫描指定 commit 范围
java-guard scan . --diff abc123..def456
```

工作原理：
1. 调用 `git diff --unified=0 --diff-filter=d <spec>` 获取变更
2. 解析每个文件的变更行范围（hunk）
3. 只扫描有变更的文件
4. 只报告变更行范围内的违规

> **注意**：如果 git diff 失败（非 git 仓库等），自动 fallback 到全量扫描。

### Baseline 模式

过滤掉已知违规，只报告新增：

```bash
# 先生成当前违规快照
java-guard scan . -f json -o baseline.json

# 后续扫描时只报新增违规
java-guard scan . --baseline baseline.json
```

Baseline 文件格式（JSON 数组）：

```json
[
  { "file": "src/Main.java", "line": 15, "rule_id": "J008" },
  { "file": "src/Utils.java", "line": 42, "rule_id": "J001" }
]
```

> **提示**：Baseline 和 `--diff` 可组合使用，先 diff 缩小文件范围，再 baseline 过滤已知问题。

---

## 8. CI Gate

Gate 模式用于 CI/CD pipeline，按违规数量阈值决定通过/失败。

### 基本用法

```bash
# 使用默认阈值（critical=0, major=0）
java-guard scan . --gate

# 自定义阈值
java-guard scan . --gate --gate-config gate-config.yml
```

### Gate 配置文件

```yaml
# gate-config.yml
gate:
  max_critical: 0
  max_major: 0
  max_minor: 10
  max_info: 50
```

### 退出码

| 场景 | 退出码 |
|------|--------|
| Gate PASS（所有级别在阈值内） | 0 |
| Gate FAIL（任一级别超阈值） | 1 |

### CI 示例

**GitHub Actions：**

```yaml
- name: JavaGuard Scan
  run: |
    java-guard scan . --gate --gate-config gate-config.yml
```

**GitLab CI：**

```yaml
javaguard:
  script:
    - java-guard scan . --gate --gate-config gate-config.yml
```

> **注意**：`--gate` 模式下，即使有违规但未超阈值也返回 exit 0。不使用 `--gate` 时默认也返回 exit 0（不因有违规而失败）。

---

## 9. 配置文件

在项目根目录放置 `java-guard.yml`，或通过 `--config` 指定：

```yaml
# java-guard.yml
rules:
  min_severity: info
  
  # 启用的规则（不指定则全启用）
  # enable:
  #   - J001
  #   - J008
  #   - J009
  
  # 禁用的规则
  # disable:
  #   - J003
  
  # 规则参数
  params:
    J006:
      max_lines: 50    # J006 方法行数阈值，默认 50
```

### 优先级

CLI 参数 > 配置文件 > 默认值

例如：配置文件中 `min_severity: info`，CLI 传入 `--min-severity major`，则使用 `major`。

---

## 10. 自定义规则

### YAML 声明式规则

适用于简单的模式匹配，无需写代码：

```yaml
# rules/NO_TODO.yml
id: NO_TODO
title: "禁止代码中遗留 TODO 注释"
severity: info
category: code-smell
pattern:
  type: MethodCall
  match_fields:
    callee:
      - "System.out"
    method:
      - "println"
message: "发现 TODO 注释，请完成后删除"
```

支持的 pattern type：

| type | 说明 | match_fields |
|------|------|-------------|
| `ClassDeclaration` | 类声明 | name, modifiers, annotations |
| `MethodDeclaration` | 方法声明 | name, return_type, modifiers, annotations, parameters |
| `FieldDeclaration` | 字段声明 | name, var_type, modifiers |
| `MethodCall` | 方法调用 | callee, method, arguments |
| `ImportDeclaration` | import 语句 | package, is_wildcard, is_static |

match_fields 值支持：
- 精确字符串：`"println"`
- 列表（任一匹配）：`["print", "println", "printf"]`
- 正则表达式（含正则元字符时自动识别）：`"^[A-Z]"`

详见 `docs/RULE_AUTHORING.md`。

### Rhai 脚本规则

适用于需要复杂逻辑的规则：

```yaml
# rules/rhai/CUSTOM_RULE.yml
id: CUSTOM001
title: "自定义规则说明"
severity: minor
category: code-smell
enabled: true
params:
  threshold: 10
script: |
  let violations = [];
  let threshold = config["threshold"];
  if threshold == () { threshold = 10; }
  
  // 通过 ast 访问 JSON AST
  let types = ast["types"];
  for t in types {
    for member in t["members"] {
      if member["kind"] == "MethodDeclaration" {
        // ... 检查逻辑
        violations.push(#{
          line: member["line"],
          end_line: member["end_line"],
          message: "描述信息"
        });
      }
    }
  }
  violations
```

Rhai 脚本可访问：
- `ast`：完整 JSON AST（来自 java-parser）
- `config`：规则参数 map
- `violations`：违规数组（push 添加）

> 详见 [Rhai 语言文档](https://rhai.rs/) 和 `docs/RULE_AUTHORING.md`。

---

## 11. FAQ

### Q: 扫描时报 "java-parser.jar not found"

设置环境变量 `JAVAGUARD_PARSER_JAR` 指向 jar 路径，或通过 `--parser-jar` 参数指定。

### Q: 扫描速度慢

当前每文件启动一个 JVM 进程解析。对于大型项目，建议：
- 使用 `--diff` 增量扫描，只检查变更文件
- 使用 `--include` 限制扫描范围

> 后续版本将实现 DaemonParser（常驻 JVM），大幅提升性能。

### Q: 规则没有触发

1. 运行 `java-guard rules` 确认规则已加载
2. 检查 `--min-severity` 是否过滤掉了
3. 检查 `--enable`/`--disable` 是否排除了
4. YAML 规则的 `match_fields` 是否匹配目标代码

### Q: 增量扫描漏报

行级过滤基于违规的 `line` 字段单点判断。如果违规跨多行（如超长方法），可能需要配合 `--baseline` 使用。

### Q: 如何在 CI 中只检查新增代码

```bash
# 方案一：git diff
java-guard scan . --diff origin/main...HEAD --gate

# 方案二：baseline
java-guard scan . --baseline baseline.json --gate
```

### Q: 支持哪些 Java 版本

JavaParser 3.28.2 支持 Java 1.0 到 Java 21 的语法。java-parser.jar 以 Java 8 为目标编译，运行时需要 JDK 8+。

### Q: 如何禁用某条规则

```bash
# CLI 禁用
java-guard scan . --disable J001,J003

# 配置文件禁用
# java-guard.yml
rules:
  disable:
    - J001
    - J003
```

---

*JavaGuard v0.1.0 | MIT License*
