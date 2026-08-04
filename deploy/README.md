# JavaGuard Deploy Package

版本：0.1.0
日期：2026-08-05

## 目录结构

```
deploy/
├── bin/
│   └── java-guard.exe          # Rust 编译的原生二进制（Windows x64, ~7.5MB）
├── java-parser/
│   └── java-parser.jar         # Java AST 解析器（JavaParser 3.28.2, ~1.8MB）
├── rules/                      # 内置规则
│   ├── J001_no_system_out.yml
│   ├── J003_no_wildcard_import.yml
│   ├── J004_class_naming.yml
│   ├── J005_method_naming.yml
│   ├── J007_constant_naming.yml
│   └── rhai/
│       └── J006_long_method.yml
├── docs/
│   └── user-manual.md          # 完整用户手册
├── java-guard.yml.example      # 配置文件示例
└── gate-config.yml.example     # CI Gate 配置示例
```

## 快速使用

1. 设置环境变量指向 java-parser.jar：

   ```powershell
   $env:JAVAGUARD_PARSER_JAR = ".\java-parser\java-parser.jar"
   ```

2. 扫描 Java 项目：

   ```bash
   .\bin\java-guard.exe scan .\your-java-project
   ```

3. 查看可用规则：

   ```bash
   .\bin\java-guard.exe rules
   ```

详见 `docs/user-manual.md`。

## 系统要求

- Windows 10+ x64
- JDK 8+（`java` 命令在 PATH 中）
