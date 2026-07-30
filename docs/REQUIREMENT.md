# JavaGuard 需求说明

## 1. 背景与目标

### 1.1 痛点

团队现有 Java 代码静态扫描依赖 SonarQube，存在以下问题：

- **部署重**：需要 SonarQube Server + PostgreSQL + Scanner CLI，CI 集成链路长
- **规则定制成本高**：自定义规则需写 Java 插件 → 编译 jar → 部署到 Server，流程繁琐
- **扫描速度慢**：全量扫描中大型项目（10w+ 行）需要数分钟，增量扫描依赖 SonarQube 的 SCM 集成
- **反馈链路长**：扫描结果需登录 Web UI 查看，不直接对接 CI/CD gate

### 1.2 目标

构建一个**轻量级 Java 代码静态扫描工具**，核心特征：

- **单二进制交付**：Rust 编译为单一可执行文件，零运行时依赖
- **规则可扩展**：YAML 声明式规则（秒级编写）+ Rhai 脚本规则（复杂逻辑）+ Java 插件（数据流分析）
- **增量扫描**：基于 git diff，只扫描变更文件，秒级完成
- **多格式报告**：JSON / CSV / SARIF / 控制台，可直接对接 CI gate
- **跨平台**：Windows / Linux / macOS，支持 ARM64

### 1.3 非目标

- 不做代码质量度量（圈复杂度、重复率等 SonarQube Metrics 功能）
- 不做安全漏洞扫描（SAST），聚焦代码规范和潜在缺陷
- 不做 IDE 插件（MVP 阶段）
- 不替代 SonarQube 的全流程，聚焦 CI/CD 中的 fast-fail gate

## 2. 用户角色

| 角色 | 场景 |
|------|------|
| **开发者** | 本地提交前快速扫描变更代码，即时反馈 |
| **CI/CD 集成者** | 在 pipeline 中配置 scan 命令，解析报告做 gate |
| **规则开发者** | 编写团队自定义规则（YAML / Rhai / Java 插件） |

## 3. 功能需求

### 3.1 核心扫描

#### F-01: 全量扫描
- 输入：项目根目录或指定 `.java` 文件列表
- 行为：递归扫描所有 `.java` 文件，解析 AST，执行所有启用的规则
- 输出：Violation 列表 + 统计摘要

#### F-02: 增量扫描
- 输入：git ref（`--diff HEAD~1` 或 `--diff main...feature`）
- 行为：通过 `git diff` 获取变更文件列表，只扫描变更文件
- 精度：行级——规则只在变更行范围内报 Violation（可配置放宽到方法级）
- 输出：同 F-01

#### F-03: 规则过滤
- 按规则 ID 启用/禁用（`--enable S001 --disable S002`）
- 按文件路径白名单/黑名单（`--include src/main/** --exclude **/generated/**`）
- 按严重级别过滤（`--min-severity major`）

### 3.2 规则体系

#### F-04: YAML 声明式规则
适用于 pattern matching 类规则，无需写代码：

```yaml
id: J001
title: 禁止使用 System.out.println
severity: minor
category: code-smell
pattern:
  type: MethodCall
  callee: System.out
  method: println
message: "不要使用 System.out.println，请使用日志框架（SLF4J）"
```

支持的 pattern 类型（MVP）：
- `MethodCall`：方法调用匹配（callee + method + 参数数量）
- `Import`：import 语句匹配（包名通配符）
- `Annotation`：注解匹配（类型 + 属性值）
- `ClassDeclaration`：类声明匹配（修饰符 + 命名规范）
- `MethodDeclaration`：方法声明匹配（返回类型 + 参数 + 命名规范）

#### F-05: Rhai 脚本规则
适用于需要 AST 遍历逻辑的规则：

```rhai
// rule_method_too_long.rhai
fn check(node, ctx) {
    if node.kind() == "MethodDeclaration" {
        let body = node.get("body");
        if body != () {
            let lines = body.end_line() - body.line();
            let max = ctx.config("max_lines", 50);
            if lines > max {
                ctx.report(
                    node.name(),
                    "方法超过 " + max + " 行（实际 " + lines + " 行）",
                    "major"
                );
            }
        }
    }
}
```

规则脚本能力：
- 遍历 AST 节点树
- 读取节点属性（名称、类型、修饰符、行号范围）
- 访问父节点 / 兄弟节点
- 读取规则配置参数
- 上报 Violation

#### F-06: Java 插件规则（预留接口，MVP 不实现）
适用于需要数据流分析的复杂规则：

```java
public class ResourceLeakRule implements Rule {
    @Override
    public void analyze(CompilationUnit cu, RuleContext ctx) {
        // 使用 Spoon ControlFlowAnalyzer 追踪资源生命周期
        // ...
    }
}
```

引擎通过反射加载 jar 包中的 `Rule` 实现类，调用 `analyze` 方法。MVP 阶段只定义接口和加载机制，不提供内置插件。

#### F-07: 规则配置文件
项目根目录 `java-guard.yml`：

```yaml
rules:
  # 启用规则
  enable:
    - J001        # 引用 YAML 规则库中的 ID
    - J002
    - custom/my_rule.rhai

  # 禁用规则（覆盖默认启用的内置规则）
  disable:
    - J099

  # 规则参数
  params:
    J002:
      max_lines: 80

  # 路径过滤
  include:
    - "src/main/**/*.java"
  exclude:
    - "src/test/**"
    - "**/generated/**"

# 扫描配置
scan:
  dialect: java          # 保留扩展位
  encoding: UTF-8
  batch_size: 50         # 每批解析文件数
```

### 3.3 报告输出

#### F-08: 控制台报告（默认）
```
[MAJOR] src/main/java/com/example/UserService.java:42
  S002: 方法超过 80 行（实际 95 行）

[MINOR] src/main/java/com/example/Util.java:15
  S001: 禁止使用 System.out.println

──────────────────────────────────
Files scanned:  127
Violations:     3 (major: 1, minor: 2)
Duration:       1.2s
```

#### F-09: JSON 报告
```json
{
  "version": "1.0",
  "scan_info": {
    "timestamp": "2026-07-30T15:03:00Z",
    "duration_ms": 1234,
    "files_scanned": 127
  },
  "violations": [
    {
      "rule_id": "S002",
      "severity": "major",
      "file": "src/main/java/com/example/UserService.java",
      "line": 42,
      "end_line": 42,
      "message": "方法超过 80 行（实际 95 行）"
    }
  ],
  "stats": {
    "total": 3,
    "by_severity": { "major": 1, "minor": 2 },
    "by_rule": { "S001": 2, "S002": 1 }
  }
}
```

#### F-10: SARIF 报告
兼容 SARIF 2.1.0 规范，可直接对接 GitHub Code Scanning / Azure DevOps。

#### F-11: CSV 报告
每行一条 Violation，适合 Excel 分析和 BI 看板。

### 3.4 CI/CD 集成

#### F-12: 退出码
- `0`：无违规或违规均在阈值内
- `1`：存在违规超过阈值（用于 CI gate）
- `2`：扫描过程出错（解析失败、配置错误等）

阈值配置：
```yaml
gate:
  max_major: 0
  max_minor: 10
  max_info: 100
```

#### F-13: baseline 机制
支持 `--baseline baseline.json`，只报告 baseline 之后的**新增**违规。用于存量项目逐步治理。

### 3.5 解析层

#### F-14: Java 解析
- 使用 JavaParser 库解析 `.java` 文件
- 解析层作为独立进程（jar），通过 CLI 调用：`java -jar java-parser.jar --input Foo.java --output ast.json`
- 引擎缓存 AST 解析结果，避免重复解析
- 支持 Java 8 ~ Java 21 语法

#### F-15: 解析层插件接口（预留）
```
Analyzer trait
  ├ JavaParserAnalyzer   (MVP 实现)
  └ SpoonAnalyzer        (预留，需要数据流分析的规则启用)
```

规则声明所需的分析级别：
```yaml
id: S100
title: 资源泄漏检测
severity: critical
requires: spoon   # 声明此规则需要 Spoon 级别分析
```

引擎根据规则需求决定使用哪个 Analyzer。MVP 阶段只有 JavaParser。

## 4. 非功能需求

### NFR-01: 性能
- 全量扫描 10w 行 Java 代码 < 10s（不含 JVM 启动）
- 增量扫描单个文件 < 200ms（含 JVM 启动，冷启动）
- 增量扫描（JVM 已预热）单个文件 < 50ms
- 内存峰值 < 512MB

### NFR-02: 交叉编译
- 目标平台：
  - `x86_64-unknown-linux-gnu`（CI Linux）
  - `aarch64-unknown-linux-gnu`（ARM CI）
  - `x86_64-pc-windows-msvc`（开发者 Windows）
  - `aarch64-apple-darwin`（开发者 Mac M 系列）
- Rust 引擎单二进制，Java 解析器 jar 随包分发

### NFR-03: 可扩展性
- 规则数量不影响启动速度（按需加载）
- 自定义规则无需重新编译引擎
- 第三方可发布规则包（规则目录 + 配置）

### NFR-04: 可维护性
- 引擎核心代码复用 SqlGuard 的 `guard-core` crate
- 解析层和规则层解耦，各自独立演进
- 规则脚本沙箱化，规则异常不影响引擎稳定性

## 5. 里程碑

| 阶段 | 交付物 | 周期 |
|------|--------|------|
| M1: 解析层 | JavaParser CLI jar + JSON AST 输出 + 基础测试 | 1 周 |
| M2: 引擎壳 | Rust CLI + 配置加载 + 文件扫描 + 控制台报告 | 1 周 |
| M3: YAML 规则 | 声明式规则引擎 + 5 条内置规则 + 规则测试 | 1.5 周 |
| M4: Rhai 规则 | Rhai 脚本集成 + AST 包装层 + 3 条内置规则 | 1.5 周 |
| M5: 增量扫描 | git diff 集成 + 行级过滤 + baseline | 1 周 |
| M6: 报告格式 | JSON + SARIF + CSV | 0.5 周 |
| M7: CI 集成 | 退出码 + gate 配置 + GitHub Actions 示例 | 0.5 周 |
| M8: 插件接口 | Java 插件加载机制 + 接口文档（不含实现） | 0.5 周 |

总计约 7.5 周。

## 6. 内置规则清单（MVP）

| ID | 规则 | 类型 | 严重级别 |
|----|------|------|----------|
| J001 | 禁止 `System.out.println` / `System.err.println` | YAML | minor |
| J002 | 方法不超过 N 行（默认 80） | Rhai | major |
| J003 | import 不使用通配符 `*` | YAML | minor |
| J004 | 类名使用 PascalCase | YAML | minor |
| J005 | 方法名使用 camelCase | YAML | minor |
| J006 | `@Override` 注解必须标注（覆写父类方法时） | Rhai | major |
| J007 | 常量使用 `UPPER_SNAKE_CASE` | YAML | minor |
| J008 | 禁止空 catch 块 | Rhai | major |

## 7. 约束与假设

- 目标项目使用 Maven 或 Gradle 构建（但不依赖构建工具运行扫描）
- Java 源码编码为 UTF-8
- 不处理注释中的规则（如 `// noqa`），MVP 后期再加
- 不支持非 Java 文件（XML、properties 等不扫描）
- MVP 不做跨文件分析（如检测未使用的 public 方法），预留接口
