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
| J010 | YAML | 禁止 `com.alibaba.fastjson.*`（fastjson），推荐 jackson |
| J011 | Rhai | `StringUtils` / `StringUtil` 应使用 `org.apache.commons.lang3` 包，禁用其它包 |
| J013 | Rhai | Spring `@Controller` / `@RestController` 的 HTTP 接口方法禁止以 `Map` 作为入参，应使用明确的请求参数对象（DTO） |
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
| `{member}` | 命中的注解参数取值 | `Annotation`（配合 `match_members`） |
| `{line}` | 命中行号 | 全部 |

未提供的占位符会原样保留在消息中（不会报错）。

## 上下文谓词（within / not_within / in_type / in_method）

`match_fields` 只能描述**节点自身**。要表达「在循环内」「在某个 Controller 里」这类
上下文约束，用下面四个谓词 —— 它们与 `match_fields` 是 **AND** 关系：

```yaml
id: J900
title: for-each 循环内禁止增删集合元素
severity: major
pattern:
  type: MethodCall
  match_fields:
    method: [remove, add, clear]
  within:                       # 命中节点必须位于下列任一 kind 的祖先之内（any_of）
    - kind: ForEachStmt
    - kind: WhileStmt
  not_within:                   # 且不得位于下列任一 kind 的祖先之内（none_of）
    - kind: CatchClause
  in_type:                      # 且「最近的」外层类/接口/枚举必须满足
    name: ".*Controller$"
    annotations: [RestController]
    is_interface: false
  in_method:                    # 且「最近的」外层方法/构造器必须满足
    name: "^get"
    return_type: void
    modifiers: [public]
    parameter_types: ["java.util.List"]
    parameter_names: [ids]
message: "在 for-each 内调用 {method}() 会抛 ConcurrentModificationException"
```

语义要点：

| 谓词 | 语义 | 组合方式 |
|------|------|---------|
| `within` | 祖先链中出现**任一**列出的 kind 即通过 | 列表内 **any_of** |
| `not_within` | 祖先链中出现**任一**列出的 kind 即失败 | 列表内 **none_of** |
| `in_type` | 最近的外层类型必须满足全部条件 | 各项 **AND**；`annotations` / `modifiers` 列表内 **any_of** |
| `in_method` | 最近的外层方法/构造器必须满足全部条件 | 同上 |

- `kind` 取值与 AST JSON 的 `kind` 字段一致（`ForEachStmt`、`TryStmt`、`LambdaExpr` …），
  加载期会校验，拼错会**直接跳过该规则并告警**，不会静默失效。
- `annotations` 无通配符时按「简单名或全限定名尾段」匹配：写 `Controller` 既能命中
  `@Controller` 也能命中 `@org.springframework.stereotype.Controller`；`@RestController`
  需要单独列出或用 `*Controller`。
- `Import` 位于任何类型之外，没有上下文，因此 import 类规则写这四个谓词会在加载期报错。

> 需要「同时嵌套在 A 与 B 内」这种 AND 组合时，改用 Rhai 规则。

## 注解参数匹配（match_members）

`AstSerializer` 会把注解参数序列化进 AST。用 `match_members` 匹配它们：

```yaml
id: J901
title: 禁止 @ComponentScan 显式指定扫描包
severity: major
pattern:
  type: Annotation
  match_fields:
    name: ComponentScan
  match_members:
    basePackages: "*"            # 带 basePackages 参数即命中
message: "@{name}({member}) 扩大了组件扫描范围，应改用自动装配"
```

- 每个键都必须**出现在注解参数中**且取值匹配（AND）；取值匹配规则同 `match_fields`。
- 单成员注解（`@Foo("x")`）统一记在键 `value` 下。
- 仅 `type: Annotation` 可用；用在其它 pattern 上会在加载期报错（否则该键被忽略，
  规则会退化成「匹配所有同类节点」并产生大量误报）。

## Rhai 脚本规则

### 脚本约定

- 全局变量 `ast` 被注入为 **AST 的 JSON 对象**（与 `java-parser` 输出的 JSON 结构一致）。
- 全局变量 `lines` / `line_count` / `source` 被注入为**源码文本**（见下节）。
- 脚本应 **返回一个数组**，每个元素是 `{ line: int, message: string, end_line?: int }` 的 map。
- 严重级别（severity）取自规则 YAML 的 `severity` 字段，脚本无需也不能设置。

### 源码文本变量（lines / line_count / source）

文本类规则（缩进、行宽、注释、换行符）需要原始源码，脚本可直接读取：

| 变量 | 类型 | 含义 |
|------|------|------|
| `lines` | 字符串数组 | 源码行，**1-based**：`lines[i]` 即第 i 行，`lines[0]` 恒为空串哨兵 |
| `line_count` | int | 源码**实际行数**，即遍历上界 |
| `source` | 字符串 | 文件**全文**，保留原始换行符（可判断 `\r\n`） |

⚠️ **`len(lines) == line_count + 1`**。遍历务必用 `line_count` 作上界，用 `len(lines)`
会越界读到不存在的行。

```yaml
# rules/rhai/J902_line_style.rhai 的头部
//! rule: J902
//! title: 单行不超过 80 字符且禁止 tab 缩进
//! severity: minor
//! params: max_len=80
let vs = [];
let max_len = config["max_len"];
if max_len == () { max_len = 80; }

let i = 1;
while i <= line_count {
    let t = lines[i].to_string();
    if t.contains("\t") {
        vs.push(#{ line: i, message: "第 " + i + " 行使用了 tab 缩进" });
    }
    if len(t) > max_len {
        vs.push(#{ line: i, message: "第 " + i + " 行长度 " + len(t) + " 超过限制" });
    }
    i = i + 1;
}
vs
```

- `len(t)` 数的是**字符数**（不是字节数），与「单行 ≤120 字符」的口径一致。
- 单元测试里手工构造的 AST 不带源码，此时 `lines == [""]`、`line_count == 0`、
  `source == ""`。文本规则应先判断 `line_count > 0` 再使用，避免误报。

### AST JSON 结构（节选）

```json
{
  "package": "com.example",
  "imports": [ { "package": "java.util", "is_wildcard": false, "is_static": false, "line": 3 } ],
  "types": [
    {
      "kind": "ClassDeclaration",
      "name": "UserService",
      "modifiers": ["public"],
      "annotations": [
        {
          "name": "ComponentScan",
          "line": 5,
          "members": [ { "key": "basePackages", "value": "{ \"com.example\" }" } ]
        }
      ],
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
> 注解参数在 `annotations[].members[]`（`key` / `value`），`value` 为参数的源码字面文本。

## 规则加载与校验

- 规则目录默认是 `rules/`，YAML 规则放根目录，Rhai 规则放 `rules/rhai/`。
  实际加载入口是 `javaguard.rules.toml` 中的 `script_path`（相对该 TOML 所在目录解析）。
- **加载期校验**（校验失败会跳过该规则并在 stderr 打印 `warn: skip rule ...`）：
  - `match_fields` 的字段名是否合法（见上表）；
  - `match_members` 是否只用在 `Annotation` pattern 上；
  - `within` / `not_within` 的 `kind` 是否是已知的 AST 节点 kind；
  - 上下文谓词是否写在了没有上下文的 `Import` pattern 上；
  - `severity` 是否合法（非法则跳过）；
  - Rhai 脚本是否为空。

> 这些校验存在的意义：上述错误在运行期都是**静默走偏**——要么规则永不命中（隐形失效），
> 要么失效的约束被忽略、规则退化成「匹配所有同类节点」而产生大量误报。


## 团队规则包分发

写好的规则要跨项目 / 跨团队复用时，把它组织成**规则包**（目录 + `rules-pack.toml` 清单），
再通过 `rules add` 注册、`rules vendor` 入库：

```text
my-team-packs/
├── rules-pack.toml     # 包清单
└── rules/
    ├── *.yml           # YAML 规则
    └── rhai/*.rhai     # Rhai 规则
```

`rules-pack.toml` 最小示例：

```toml
[pack]
name = "rules-spring"        # 包名，全局唯一
version = "1.0.0"            # 包版本，独立于引擎版本演进
api_version = 1              # 引擎 API 契约（硬闸门，不满足拒绝加载）
engine = ">=0.1.7 <0.2"      # 引擎版本范围（软告警）
# namespace = "spring"       # 可选：设置后包内规则 id 变为 <ns>:<id>（如 spring:J701）

[[rules]]
id = "J701"
name = "no_componentscan_basepackages"
group = "spring-convention"
description = "禁止硬编码 @ComponentScan(basePackages)"
script_path = "rules/J701_no_componentscan_basepackages.yml"   # 相对包根
severity = "warning"
enabled = false              # 团队包建议默认关闭，由各项目 overrides 按需开启
```

分发三步：

```bash
# 1) 注册到项目（先校验后写入：清单 / api_version / engine 契约 / 脚本逃逸全过闸，
#    通过后写入 java-guard.toml 并自动刷新 javaguard.lock）
java-guard rules add rules-spring --path ../team-packs/rules-spring

# 2) 按需启用（overrides 用裸 id 即可跨包匹配）
#    [[rule_packs.overrides]]
#    id = "J701"
#    enabled = true

# 3) 入库（把生效的包复制到 vendor/rules/，随仓库提交，成员与 CI 免共享盘）
java-guard rules vendor
```

包内规则、锁文件与 CI 严格校验（`rules verify --locked`）的完整说明见用户手册 3.2 节。

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
