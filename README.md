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

# 查看所有规则
java-guard rules
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

4. 将 `javaguard.rules.toml` 和 `java-guard.toml` 放置到项目根目录

### 验证

```bash
java-guard version
java-guard rules
```

## 配置文件

### 项目配置（`java-guard.toml`）

项目级配置，包含扫描行为和 CI gate 设置。

```toml
# 规则配置文件路径（相对于本文件所在目录解析）
# 不写时自动查找同目录下的 javaguard.rules.toml
[rules]
rules_file = "javaguard.rules.toml"

# 只启用指定规则（覆盖 rules_file 中的 enabled 字段）
# enable = ["J001", "J003"]

# 禁用指定规则（覆盖 rules_file 中的 enabled 字段）
# disable = ["J008"]

# 最低严重级别：info / minor / major / critical
# min_severity = "info"

[scan]
# 路径白名单（只扫描匹配路径）
# include = ["src/main", "src/test"]

# 路径黑名单（排除目录）
# exclude = ["build", "dist", "generated"]

# 源文件编码：auto（自动探测）/ utf-8 / gbk / shift-jis 等
# encoding = "auto"

[gate]
# 违规超过阈值时退出码 1（配合 --gate 使用）
max_critical = 0
max_major = 0
max_minor = 10
max_info = 50
```

### 规则配置（`javaguard.rules.toml`）

独立的规则定义文件，每条 `[[rules]]` 包含规则的元数据和脚本路径。用户可以通过此文件清晰地了解系统支持哪些规则校验。

```toml
# ── 命名规范 ──────────────────────────────────────────────────────────────────

[[rules]]
id = "J004"
name = "class_naming"
group = "naming"
description = "类名必须使用 PascalCase（大写驼峰）"
script_path = "rules/J004_class_naming.yml"
severity = "info"
enabled = true

[[rules]]
id = "J005"
name = "method_naming"
group = "naming"
description = "方法名必须使用 camelCase（小写驼峰）"
script_path = "rules/J005_method_naming.yml"
severity = "info"
enabled = true

# ── 代码规范 ──────────────────────────────────────────────────────────────────

[[rules]]
id = "J001"
name = "no_system_out"
group = "code-style"
description = "禁止使用 System.out / System.err 直接打印，请使用日志框架（SLF4J）"
script_path = "rules/J001_no_system_out.yml"
severity = "info"
enabled = true

[[rules]]
id = "J006"
name = "long_method"
group = "code-style"
description = "单个方法不超过 50 行（可通过 params 自定义阈值）"
script_path = "rules/rhai/J006_long_method.rhai"
severity = "info"
enabled = true

[rules.params]
max_lines = 50

# ── 依赖安全 ──────────────────────────────────────────────────────────────────

[[rules]]
id = "J010"
name = "no_fastjson_import"
group = "dependency-security"
description = "禁止 import fastjson / fastjson2，推荐使用 jackson"
script_path = "rules/J010_no_fastjson.yml"
severity = "warning"
enabled = true

# ── 潜在缺陷 ──────────────────────────────────────────────────────────────────

[[rules]]
id = "J008"
name = "empty_catch"
group = "potential-bug"
description = "空 catch 块：catch 内部没有任何语句，会吞掉异常"
script_path = "builtin:J008"
severity = "warning"
enabled = true

[[rules]]
id = "J009"
name = "infinite_loop"
group = "potential-bug"
description = "潜在死循环：for(;;)、while(true)、或缺少更新条件的循环"
script_path = "builtin:J009"
severity = "warning"
enabled = true
```

### `[[rules]]` 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 是 | 规则 ID（如 `J001`），全局唯一 |
| `name` | string | 是 | 规则名称（如 `no_system_out`），用于日志和输出 |
| `group` | string | 否 | 规则分组（如 `naming`、`code-style`、`dependency-security`） |
| `description` | string | 否 | 规则描述，说明这条规则检查什么 |
| `script_path` | string | 是 | 脚本路径（YAML/Rhai 文件），或 `builtin:xxx` 引用内置规则 |
| `severity` | string | 否 | 严重级别：`info` / `minor` / `major` / `critical`（默认 `info`） |
| `enabled` | bool | 否 | 是否启用（默认 `true`） |
| `applies_to` | list | 否 | 适用文件类型（默认 `["java"]`） |
| `params` | table | 否 | 规则参数（传递给 Rhai 脚本的 `config` 变量） |

### `script_path` 路径规则

- **相对路径**：相对于 `javaguard.rules.toml` 所在目录解析，如 `rules/J001_no_system_out.yml`
- **绝对路径**：直接使用，如 `/opt/rules/custom.yml`
- **内置规则**：`builtin:xxx` 前缀引用 Rust 内置规则，如 `builtin:J008`

## 内置规则

| 规则 ID | 分组 | 严重级别 | 名称 | 说明 |
|---------|------|---------|------|------|
| J001 | code-style | info | no_system_out | 禁止 System.out / System.err 直接打印 |
| J003 | code-style | info | no_wildcard_import | import 不使用通配符 |
| J004 | naming | info | class_naming | 类名使用 PascalCase |
| J005 | naming | info | method_naming | 方法名使用 camelCase |
| J006 | code-style | info | long_method | 方法不超过 50 行（可配置） |
| J007 | naming | info | constant_naming | 常量使用 UPPER_SNAKE_CASE |
| J008 | potential-bug | warning | empty_catch | 空 catch 块检测 |
| J009 | potential-bug | warning | infinite_loop | 潜在死循环检测 |
| J010 | dependency-security | warning | no_fastjson_import | 禁止 import fastjson，推荐 jackson |
| J011 | dependency-security | info | commons_lang3_stringutils | 推荐使用 commons-lang3 StringUtils |
| J012 | dependency-security | warning | no_fastjson_usage | 禁止代码中使用 fastjson 类 |
| J013 | spring-convention | warning | controller_no_map_param | Spring Controller 禁止 Map 传参 |
| J014 | dependency-security | warning | no_nonjackson_json_import | 禁止引入非 jackson 的 JSON 框架 |
| J015 | dependency-security | warning | no_nonjackson_json_usage | 禁止使用非 jackson JSON 框架的类 |

## 主要特性

### 三层规则引擎

- **YAML 声明式**：简单模式匹配，无需写代码
- **Rhai 脚本**：复杂逻辑，可访问完整 AST
- **Java 插件**（预留）：JSON-RPC 协议扩展

### 增量扫描

```bash
# git diff 模式：只扫描变更文件和行范围
java-guard scan . --diff HEAD~1

# 语义对比模式：解析旧版本（git show）并做违规集合差，只报真正新增的违规
java-guard scan . --diff HEAD~1 --semantic-diff

# baseline 模式：过滤已知违规（行号漂移容差 5 行内视为同一违规）
java-guard scan . --baseline baseline.json

# 调整 baseline 匹配容差
java-guard scan . --baseline baseline.json --baseline-tolerance 10

# 导出当前违规为 baseline（供后续 --baseline 使用）
java-guard scan . --baseline-out baseline.json
```

增量过滤语义（规则级可配）：

- `--diff` 模式：文件级（只解析变更文件）+ 行级（违规报告期过滤）两级过滤；**分析仍基于完整 AST**，不丢上下文
- 行级过滤按规则的 `span_policy` 判定：
  - `anchor`（默认）：锚点行落在变更行范围才报告，适合行级规则（System.out、空 catch）
  - `intersect`：违规区间与变更行范围相交即报告，适合结构类规则（方法超长 J006、死循环 J009）
- `--baseline` 模式：按 `(文件, 规则)` 分组做**距离容忍匹配**（行号差 ≤ 容差视为同一违规，1:1 分配），抗重构导致的行号漂移；容差可用 `--baseline-tolerance` 调整（默认 5）
- `--semantic-diff`（语义对比模式，需配合 `--diff`）：解析旧版本源码（`git show <旧侧>:<路径>`）并与当前版本分别跑完整规则，按 `新违规 − 旧违规` 输出**集合差**

### 报告格式

- **console**（默认）：彩色终端输出
- **json**：含扫描统计信息，适合程序处理
- **sarif**：SARIF 2.1.0，兼容 GitHub Code Scanning / Azure DevOps
- **csv**：表格格式

### CI 集成

```yaml
# GitHub Actions
- name: JavaGuard Scan
  run: java-guard scan . --gate
```

Gate 配置在 `java-guard.toml` 中定义：

```toml
[gate]
max_critical = 0
max_major = 0
max_minor = 10
max_info = 50
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
.javaguard.rules.toml → 加载规则元数据 + 脚本路径
                  ↓
.java 文件 → java-parser.jar (JVM) → JSON AST → Rust 规则引擎 → 违规报告
```

## 自定义规则

### 添加新规则

1. 在 `javaguard.rules.toml` 中添加规则条目：

```toml
[[rules]]
id = "CUSTOM001"
name = "no_todo_comments"
group = "code-style"
description = "禁止 TODO 注释（应在提交前解决）"
script_path = "rules/custom/no_todo_comments.yml"
severity = "info"
enabled = true
```

2. 创建对应的脚本文件（YAML 或 Rhai）

### YAML 规则示例

```yaml
id: CUSTOM001
title: "禁止 TODO 注释"
severity: info
pattern:
  type: Annotation
  match_fields:
    name: "TODO"
message: "禁止 TODO 注释，请在提交前解决"
```

### Rhai 脚本规则示例

```yaml
id: CUSTOM001
title: "自定义规则"
severity: minor
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
  -r, --rules-file <FILE>   规则配置文件路径（TOML）
      --diff <SPEC>         增量扫描：git diff 范围
      --semantic-diff       语义对比模式：解析旧版本并做违规集合差（需 --diff）
      --baseline-out <FILE> 导出当前违规为 baseline JSON
      --baseline <FILE>     Baseline 文件（只报告新增违规）
      --baseline-tolerance <N>  Baseline 匹配容差：行号差 ≤ N 视为同一违规 [default: 5]
      --gate                CI gate 模式
      --gate-config <FILE>  Gate 配置文件
      --enable <IDS>        启用规则（逗号分隔）
      --disable <IDS>       禁用规则（逗号分隔）
      --min-severity <LVL>  最低严重级别 [default: info]
      --parser-jar <PATH>   java-parser.jar 路径 [env: JAVAGUARD_PARSER_JAR]
      --java-cmd <PATH>     Java 运行时路径 [env: JAVA_CMD]
      --config <PATH>       项目配置文件 [default: java-guard.toml]
```

## 文档

- [用户手册](deploy/docs/user-manual.md) — 安装、使用、CLI 参考、规则说明、CI 集成
- [需求说明](docs/REQUIREMENT.md) — 功能需求与里程碑
- [技术方案](docs/TECHNICAL_DESIGN.md) — 架构设计与模块说明
- [规则编写指南](docs/RULE_AUTHORING.md) — YAML / Rhai 规则开发

## 状态

MVP 已实现（M1–M8 完成）：
- CLI 扫描、YAML / Rhai 规则引擎
- TOML 规则配置（`javaguard.rules.toml`）
- 增量扫描（git diff + baseline）
- JSON / SARIF / CSV / 控制台报告
- CI gate（阈值退出码）
- Java 插件接口（预留）

## License

MIT
