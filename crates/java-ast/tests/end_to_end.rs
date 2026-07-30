//! 端到端测试：Java parser jar → Rust AST 反序列化。
//!
//! 依赖 java-parser.jar 已构建（mvn package）。

use java_ast::JavaParser;
use std::path::PathBuf;

#[test]
fn end_to_end_parse_simple_java() {
    let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap() // workspace root
        .join("java-parser/target/java-parser.jar");

    if !jar.exists() {
        eprintln!("skipping: {} not found (run mvn package first)", jar.display());
        return;
    }

    let java_cmd = std::env::var("JAVA_CMD").unwrap_or_else(|_| {
        // 尝试 GraalVM 路径
        let graal = r"C:\Program Files\Graalvm\graalvm-jdk-22.0.2+9.1\bin\java.exe";
        if PathBuf::from(graal).exists() {
            graal.to_string()
        } else {
            "java".to_string()
        }
    });

    let parser = java_ast::bridge::CliParser::new(&jar).with_java_cmd(java_cmd);

    let source = r#"
package com.example;

public class Test {
    public void hello() {
        System.out.println("hello");
    }
}
"#;

    let unit = parser.parse(source, "Test.java").expect("parse should succeed");

    assert_eq!(unit.package.as_deref(), Some("com.example"));
    assert_eq!(unit.types.len(), 1);

    let first = &unit.types[0];
    match first {
        java_ast::ast::TypeDecl::ClassDeclaration(c) => {
            assert_eq!(c.name, "Test");
            assert_eq!(c.members.len(), 1);
            match &c.members[0] {
                java_ast::ast::MemberDecl::MethodDeclaration(m) => {
                    assert_eq!(m.name, "hello");
                    assert!(m.body.is_some());
                }
                _ => panic!("expected method"),
            }
        }
        _ => panic!("expected class"),
    }
}
