# JavaGuard 规则编写指南

## 规则类型

JavaGuard 支持三种规则类型，按复杂度递增：

| 类型 | 适用场景 | 编写速度 | 执行性能 |
|------|---------|---------|---------|
| YAML 声明式 | pattern matching（90% 规则） | 秒级 | 最快 |
| Rhai 脚本 | 需要 AST 遍历逻辑 | 分钟级 | 快 |
| Java 插件 | 需要数据流分析 | 小时级 | 较慢（预留） |

## YAML 声明式规则

### 基本结构

```yaml
id: J001                          # 全局唯一，字母+数字
title: 禁止使用 System.out.println # 简短描述
severity: minor                    # info | minor | major | critical
category: code-smell               # code-smell | bug | security | convention
tags: [logging, convention]        # 可选标签
pattern:                           # 匹配模式（见下文）
  ...
message: "不要使用 {callee}.{method}，请使用日志框架（SLF4J）"  # {占位符} 从匹配节点取值
```

### Pattern 类型

#### MethodCall — 方法调用匹配

```yaml
id: J001
title: 禁止 System.out.println
severity: minor
pattern:
  type: MethodCall
  callee:                          # 可选，不填则匹配任意 callee
    any_of:
      - System.out
      - System.err
  method:                          # 可选
    any_of:
      - println
      - printf
      - print
  args_count:                      # 可选，参数数量约束
    min: 1
    max: 2
message: "禁止使用 {callee}.{method}"
```

#### Import — import 语句匹配

```yaml
id: J003
title: 禁止通配符 import
severity: minor
pattern:
  type: Import
  is_wildcard: true                # 匹配 import xxx.*
  # package: "com.example.**"     # 可选，包名通配符
message: "禁止使用通配符 import: {package}"
```

#### ClassDeclaration — 类声明匹配

```yaml
id: J004
title: 类名必须使用 PascalCase
severity: minor
pattern:
  type: ClassDeclaration
  name_regex: "^[a-z]"            # 匹配不符合规范的名称（小写开头为违规）
  # modifiers:                    # 可选，修饰符约束
  #   any_of: [public, abstract]
message: "类名 '{name}' 应使用 PascalCase"
```

#### MethodDeclaration — 方法声明匹配

```yaml
id: J005
title: 方法名必须使用 camelCase
severity: minor
pattern:
  type: MethodDeclaration
  name_regex: "^[A-Z]"            # 大写开头为违规
message: "方法名 '{name}' 应使用 camelCase"
```

#### CatchBlock — catch 块匹配

```yaml
id: J008
title: 禁止空 catch 块
severity: major
pattern:
  type: CatchBlock
  is_empty: true
message: "空 catch 块会吞没异常，至少应记录日志"
```

### StringMatcher

所有需要字符串匹配的字段支持三种格式：

```yaml
# 精确匹配
method: println

# 多值匹配
method:
  any_of: [println, printf, print]

# 正则匹配
name_regex: "^[A-Z].*"
```

### 消息占位符

`message` 字段支持从匹配节点取值：

| 占位符 | 含义 | 示例 |
|--------|------|------|
| `{name}` | 节点名称 | 类名、方法名 |
| `{callee}` | 方法调用者 | `System.out` |
| `{method}` | 方法名 | `println` |
| `{line}` | 行号 | `42` |
| `{package}` | 包名 | `java.util` |
| `{file}` | 文件名 | `UserService.java` |

## Rhai 脚本规则

### 基本结构

```rhai
// rule_custom.rhai

fn check(node, ctx) {
    // node: AST 节点（CompilationUnit 级别）
    // ctx: RuleContext，用于上报违规和读取配置

    // 遍历所有方法
    for type_decl in node.children() {
        if type_decl.kind() == "ClassDeclaration" {
            for member in type_decl.children() {
                if member.kind() == "MethodDeclaration" {
                    check_method(member, ctx);
                }
            }
        }
    }
}

fn check_method(method, ctx) {
    let max_lines = ctx.config("max_lines", 80);
    let body = method.get("body");
    if body == () { return; }  // 抽象方法无 body

    let lines = body.end_line() - body.line();
    if lines > max_lines {
        ctx.report(
            method.name(),
            "方法超过 " + max_lines + " 行（实际 " + lines + " 行）",
            "major"
        );
    }
}
```

### Node API

每个 AST 节点提供以下方法：

| 方法 | 返回类型 | 说明 |
|------|---------|------|
| `kind()` | `String` | 节点类型（见下表） |
| `name()` | `String` | 节点名称（类名/方法名/变量名） |
| `line()` | `int` | 起始行号 |
| `end_line()` | `int` | 结束行号 |
| `children()` | `Array<Node>` | 子节点列表 |
| `parent()` | `Option<Node>` | 父节点 |
| `text()` | `String` | 原始源码文本 |
| `get(key)` | `Dynamic` | 动态属性（modifiers, return_type, body 等） |

### 节点 kind 值

```
CompilationUnit
ClassDeclaration
InterfaceDeclaration
EnumDeclaration
AnnotationDeclaration
FieldDeclaration
MethodDeclaration
ConstructorDeclaration
InitializerDeclaration
BlockStmt
IfStmt
ForStmt
WhileStmt
TryStmt
ReturnStmt
ThrowStmt
ExpressionStmt
VariableDeclarationStmt
MethodCallExpr
FieldAccessExpr
NameExpr
LiteralExpr
BinaryExpr
AnnotationExpr
```

### RuleContext API

| 方法 | 说明 |
|------|------|
| `report(location, message, severity)` | 上报违规 |
| `config(key, default)` | 读取规则参数 |
| `file_path()` | 当前文件路径 |

### 完整示例：检测缺失 @Override

```rhai
// J006-missing-override.rhai

fn check(node, ctx) {
    for type_decl in node.children() {
        if type_decl.kind() == "ClassDeclaration" {
            let super_name = type_decl.get("extends");
            if super_name == () { continue; }

            // 收集父类方法名（简化版：通过 implements/extends 接口名推断）
            // 实际实现需要符号解析，这里只检查命名约定
            for member in type_decl.children() {
                if member.kind() == "MethodDeclaration" {
                    let has_override = false;
                    for ann in member.get("annotations") {
                        if ann.name() == "Override" {
                            has_override = true;
                        }
                    }
                    // 如果方法是 public 且不以 get/set/is 开头
                    // 且类继承了某个父类，建议标注 @Override
                    // （这只是一个启发式，真正的检测需要符号表）
                }
            }
        }
    }
}
```

## 规则配置

### 项目级配置 (java-guard.yml)

```yaml
rules:
  enable:
    - J001
    - J002
    - custom/my_rule.rhai

  disable:
    - J099

  params:
    J002:
      max_lines: 100    # 覆盖默认值 80

include:
  - "src/main/**/*.java"
exclude:
  - "src/test/**"
  - "**/generated/**"
```

### 命令行覆盖

```bash
# 启用额外规则
java-guard scan --enable CUSTOM001

# 禁用规则
java-guard scan --disable J002

# 设置参数
java-guard scan --param J002.max_lines=100
```

## 规则测试

每条规则应附带测试用例：

```
rules/rhai/J002-method-too-long.rhai
tests/fixtures/J002/
  ├── should_pass.java      # 方法 30 行，不触发
  ├── should_fail.java      # 方法 100 行，触发
  └── expected.json         # 期望的 Violation
```

`expected.json`:
```json
[
  {
    "rule_id": "J002",
    "line": 1,
    "message_contains": "方法超过 80 行"
  }
]
```

运行测试：
```bash
java-guard test rules/rhai/J002-method-too-long.rhai
```
