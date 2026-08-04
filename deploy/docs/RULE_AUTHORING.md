# JavaGuard 规则编写指南

> 本文件描述 **当前真实实现** 的规则 API。设计文档（TECHNICAL_DESIGN.md）描述目标架构，
> 二者在 Rhai/YAML 细节上已有差异，请以本文件为准。

## 规则类型

JavaGuard 支持三种规则来源，按复杂度递增：

| 类型 | 适用场景 | 编写速度 | 执行性能 |
|------|---------|---------|---------|
| YAML 声明式 | pattern matching（绝大多数规则） | 秒级 | 最快 |
| Rhai 脚本 | 需要自定义 AST 遍历逻辑 | 分钟级 | 快 |
| Rust 内置 | 需要复杂控制流（如空 catch 检测） | 编译期 | 最快 |
| Java 插件 | 数据流分析 | 小时级 | 较慢（预留，尚未启用） |

## 内置规则清单

| ID | 类型 | 说明 |
|----|------|------|
| J001 | YAML | 禁止 `System.out` / `System.err` 的 `print/println/printf` |
| J003 | YAML | 禁止通配符 import（`import xxx.*`） |
| J004 | YAML | 类名应使用 PascalCase |
| J005 | YAML | 方法名应使用 camelCase |
| J006 | Rhai | 方法体过长（默认 > 50 行） |
| J007 | YAML | 常量（`static final` 字段）应使用 UPPER_SNAKE_CASE |
| J008 | Rust 内置 | 禁止空 catch 块 |

## YAML 声明式规则

### 基本结构

```yaml
id: J001                          # 全局唯一，字母+数字
title: 禁止使用 System.out/err 打印 # 简短描述
severity: minor                    # info | minor | major | critical
category: code-smell               # code-smell | bug | security | convention
pattern:                           # 匹配模式（见下文）
  type: MethodCall
  match_fields:
    callee:
      - System.out
      - System.err
    method:
      - print
      - println
      - printf
message: "不要使用 {callee}.{method}，请使用日志框架（SLF4J）"
```

### `match_fields` 取值语法

每个 `match_fields` 是一个 `字段名 → 期望值` 的映射。期望值支持两种写法：

```yaml
# 1) 单个值：精确匹配 / glob / 正则
method: "println"        # 精确匹配 "println"
method: "print*"         # glob：* 匹配任意字符序列
name: "^[a-z]"           # 正则：以 ^ 开头或以 $ 结尾时按正则匹配
callee: "System.*"       # glob：匹配 System.out / System.err 等

# 2) 列表：任意一个命中即可（any_of 语义）
method:
  - print
  - println
  - printf
```

匹配判定顺序：列表中的每一项单独按「精确 / glob / 正则」规则判定，任意一项命中即字段匹配成功；
所有字段都匹配成功，该 AST 节点才算命中。

> ⚠️ **未知字段名会被静默忽略**（不会报错也不会命中），加载时会校验并跳过非法规则。
> 各 `type` 允许使用的字段名见下表。

### Pattern 类型与可用字段

| `type` | 匹配目标 | 允许字段 |
|--------|---------|---------|
| `MethodCall` | 方法调用 | `callee`, `method`, `method_name` |
| `Import` | import 语句 | `package`, `is_wildcard`, `is_static` |
| `Annotation` | 注解使用 | `name`, `type` |
| `ClassDeclaration` | 类/接口/枚举/注解声明 | `name`, `modifier`, `modifiers` |
| `MethodDeclaration` | 方法声明 | `name`, `return_type`, `modifier`, `modifiers` |
| `FieldDeclaration` | 字段声明 | `name`, `field_type`, `type`, `modifier`, `modifiers` |

> 说明：
> - `Import` 的 `is_wildcard` / `is_static` 取值为 `"true"` 或 `"false"`。
> - `modifier` / `modifiers` 表示修饰符约束（如 `public`、`final`），命中条件为「存在任一修饰符匹配」。
> - 方法声明 / 字段声明 / 类声明均会 **递归进入嵌套类**，因此深层嵌套类中的方法或字段也能被检出。

### 各类型示例

```yaml
# J003 — 禁止通配符 import
id: J003
title: 禁止通配符 import
severity: minor
pattern:
  type: Import
  match_fields:
    is_wildcard: "true"
message: "禁止使用通配符 import: {package}"

# J004 — 类名 PascalCase（正则：小写开头即违规）
id: J004
title: 类名应使用 PascalCase
severity: minor
pattern:
  type: ClassDeclaration
  match_fields:
    name: "^[a-z]"
message: "类名 '{name}' 应使用 PascalCase"

# J005 — 方法名 camelCase（正则：大写开头即违规）
id: J005
title: 方法名应使用 camelCase
severity: minor
pattern:
  type: MethodDeclaration
  match_fields:
    name: "^[A-Z]"
message: "方法名 '{name}' 应使用 camelCase"

# J007 — 常量 UPPER_SNAKE_CASE
id: J007
title: 常量应使用 UPPER_SNAKE_CASE
severity: minor
pattern:
  type: FieldDeclaration
  match_fields:
    modifier: "final"
    name: "[a-z]"
message: "常量（static final 字段）'{name}' 应使用 UPPER_SNAKE_CASE"
```

### 消息占位符

`message` 中可用 `{key}` 从匹配的节点取值，运行时会被替换为实际值：

| 占位符 | 含义 | 适用 pattern |
|--------|------|------|
| `{callee}` | 方法调用者 | `MethodCall` |
| `{method}` | 方法名 | `MethodCall` |
| `{name}` | 节点名称（类/方法/字段/注解名） | 全部 |
| `{return_type}` | 方法返回类型 | `MethodDeclaration` |
| `{field_type}` | 字段类型 | `FieldDeclaration` |
| `{package}` | 包名 | `Import` |
| `{line}` | 命中行号 | 全部 |

未提供的占位符会原样保留在消息中（不会报错）。

## Rhai 脚本规则

### 脚本约定

- 全局变量 `ast` 被注入为 **AST 的 JSON 对象**（与 `java-parser` 输出的 JSON 结构一致）。
- 脚本应 **返回一个数组**，每个元素是 `{ line: int, message: string, end_line?: int }` 的 map。
- 严重级别（severity）取自规则 YAML 的 `severity` 字段，脚本无需也不能设置。

```yaml
# rules/rhai/J006_long_method.yml
id: J006
title: 方法不超过 50 行
severity: minor
category: code-smell
script: |
  let violations = [];
  for t in ast.types {
    for member in t.members {
      if member.kind == "MethodDeclaration" {
        let lines = member.end_line - member.line;
        if lines > 50 {
          violations.push(#{
            line: member.line,
            message: "方法长度 " + lines + " 行，超过 50 行限制"
          });
        }
      }
    }
  }
  violations
```

对应的 Rust 侧加载（节选自 `rule-rhai/src/engine.rs`）：

```rust
// engine.run(rule, unit, file):
//   1. 将 unit.raw_json（或回退 JSON）转换为 Rhai Dynamic
//   2. 注入全局变量 ast
//   3. 执行脚本，要求返回数组
//   4. 逐元素解析为 Violation（line / message / end_line）
```

### AST JSON 结构（节选）

```json
{
  "package": "com.example",
  "imports": [ { "package": "java.util", "isWildcard": false, "isStatic": false, "line": 3 } ],
  "types": [
    {
      "kind": "ClassDeclaration",
      "name": "UserService",
      "modifiers": ["public"],
      "members": [
        {
          "kind": "MethodDeclaration",
          "name": "findById",
          "modifiers": ["public"],
          "returnType": "User",
          "line": 7,
          "endLine": 12
        }
      ],
      "line": 6,
      "endLine": 50
    }
  ],
  "sourceFile": "UserService.java"
}
```

> 编写 Rhai 规则时，直接按上面的 JSON 字段访问即可（如 `member.kind`、`member.end_line`）。

## 规则加载与校验

- 规则目录默认是 `rules/`，YAML 规则放根目录，Rhai 规则放 `rules/rhai/`。
- 加载时会校验：
  - `match_fields` 的字段名是否合法（见上表），未知字段会 **跳过该规则并打印警告**；
  - `severity` 是否合法（非法则告警并降级为 `minor`）；
  - Rhai 脚本是否为空。

## 命令行覆盖

```bash
# 列出所有可用规则
java-guard rules

# 仅使用某批规则
java-guard scan . --enable J001,J003

# 禁用某条规则
java-guard scan . --disable J008

# 仅报告不低于某严重级别的违规
java-guard scan . --min-severity major

# 输出 JSON 报告到文件
java-guard scan . -f json -o report.json
```

## 增量扫描与 CI Gate

```bash
# 只检查最近一次提交变更的文件与行
java-guard scan . --diff HEAD~1

# 只报告相对 baseline 的新增违规
java-guard scan . --baseline baseline.json

# CI gate：违规超阈值时退出码为 1
java-guard scan . --gate --gate-config gate.yml
```
