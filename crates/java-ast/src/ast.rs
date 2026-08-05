//! Java AST 节点模型。与 JavaParser 序列化的 JSON 格式一一对应。

use serde::{Deserialize, Serialize};

/// Java 编译单元（一个 .java 文件的 AST 根节点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationUnit {
    pub package: Option<String>,
    pub imports: Vec<ImportDecl>,
    pub types: Vec<TypeDecl>,
    // JavaParser 侧输出的是驼峰 `sourceFile`，这里用 alias 兼容两种写法。
    #[serde(default, alias = "sourceFile")]
    pub source_file: String,
    #[serde(default)]
    pub source_lines: Vec<String>,
    /// 原始 JSON 字符串（用于 Rhai 等需要原始 JSON 的场景）。
    #[serde(skip)]
    pub raw_json: String,
}

/// import 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDecl {
    pub package: String,
    // JavaParser 侧输出驼峰 `isWildcard` / `isStatic`。
    // 缺少 alias 时会静默落到 default(false)，导致 J003（禁止通配符 import）永不触发。
    #[serde(default, alias = "isWildcard")]
    pub is_wildcard: bool,
    #[serde(default, alias = "isStatic")]
    pub is_static: bool,
    pub line: usize,
}

/// 顶层类型声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum TypeDecl {
    ClassDeclaration(ClassDecl),
    InterfaceDeclaration(InterfaceDecl),
    EnumDeclaration(EnumDecl),
    AnnotationDeclaration(AnnotationDecl),
}

/// 类声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub implements: Vec<String>,
    #[serde(default)]
    pub members: Vec<MemberDecl>,
    pub line: usize,
    pub end_line: usize,
}

/// 接口声明。

/// 枚举声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub implements: Vec<String>,
    #[serde(default)]
    pub constants: Vec<EnumConstant>,
    #[serde(default)]
    pub members: Vec<MemberDecl>,
    pub line: usize,
    pub end_line: usize,
}

/// 枚举常量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumConstant {
    pub name: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    pub line: usize,
}

/// 注解类型声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub members: Vec<MemberDecl>,
    pub line: usize,
    pub end_line: usize,
}

/// 接口声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub extends: Vec<String>,
    #[serde(default)]
    pub members: Vec<MemberDecl>,
    pub line: usize,
    pub end_line: usize,
}

/// 成员声明（字段/方法/构造器/初始化块/嵌套类型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum MemberDecl {
    FieldDeclaration(FieldDecl),
    MethodDeclaration(MethodDecl),
    ConstructorDeclaration(ConstructorDecl),
    InitializerDeclaration(InitializerDecl),
    ClassDeclaration(ClassDecl),
    InterfaceDeclaration(InterfaceDecl),
    EnumDeclaration(EnumDecl),
    AnnotationDeclaration(AnnotationDecl),
}

/// 字段声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub field_type: Option<String>,
    #[serde(default)]
    pub initializer: Option<Expr>,
    pub line: usize,
}

/// 方法声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub return_type: Option<String>,
    #[serde(default)]
    pub parameters: Vec<ParamDecl>,
    #[serde(default)]
    pub body: Option<BlockStmt>,
    pub line: usize,
    pub end_line: usize,
}

/// 构造器声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructorDecl {
    pub name: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub parameters: Vec<ParamDecl>,
    #[serde(default)]
    pub body: Option<BlockStmt>,
    pub line: usize,
    pub end_line: usize,
}

/// 初始化块（static {} 或 {}）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializerDecl {
    #[serde(default)]
    pub is_static: bool,
    pub body: BlockStmt,
    pub line: usize,
}

/// 参数声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDecl {
    #[serde(default)]
    pub param_type: Option<String>,
    pub name: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

/// 注解。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub name: String,
    #[serde(default)]
    pub members: Vec<AnnotationMember>,
    pub line: usize,
}

/// 注解成员（键值对）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationMember {
    pub key: String,
    pub value: String,
}

/// 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum Stmt {
    ExpressionStmt(ExprStmt),
    VariableDeclarationStmt(VarDeclStmt),
    IfStmt(IfStmt),
    ForStmt(ForStmt),
    ForEachStmt(ForEachStmt),
    WhileStmt(WhileStmt),
    DoStmt(DoStmt),
    TryStmt(TryStmt),
    ReturnStmt(ReturnStmt),
    ThrowStmt(ThrowStmt),
    BreakStmt(BreakStmt),
    ContinueStmt(ContinueStmt),
    BlockStmt(BlockStmt),
    SwitchStmt(SwitchStmt),
    SynchronizedStmt(SynchronizedStmt),
    EmptyStmt,
    /// 未知语句类型（Java 侧序列化器未覆盖的语句）。
    UnknownStmt {
        #[serde(default)]
        line: usize,
        #[serde(default)]
        value: String,
    },
}

/// 表达式语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExprStmt {
    pub expr: Expr,
    pub line: usize,
}

/// 变量声明语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarDeclStmt {
    #[serde(default)]
    pub var_type: Option<String>,
    pub declarations: Vec<VarDeclarator>,
    pub line: usize,
}

/// 变量声明符。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarDeclarator {
    pub name: String,
    #[serde(default)]
    pub initializer: Option<Expr>,
}

/// if 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IfStmt {
    pub condition: Expr,
    pub then_stmt: Box<Stmt>,
    #[serde(default)]
    pub else_stmt: Option<Box<Stmt>>,
    pub line: usize,
}

/// for 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForStmt {
    #[serde(default)]
    pub initialization: Option<Expr>,
    #[serde(default)]
    pub condition: Option<Expr>,
    #[serde(default)]
    pub update: Vec<Expr>,
    pub body: Box<Stmt>,
    pub line: usize,
}

/// enhanced for 语句（for-each）。
/// `for (Type var : iterable) body`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachStmt {
    /// 循环变量声明，JavaParser 输出为 VariableDeclarationExpr。
    pub variable: Expr,
    /// 被迭代的表达式（集合/数组）。
    pub iterable: Expr,
    pub body: Box<Stmt>,
    pub line: usize,
}

/// while 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Box<Stmt>,
    pub line: usize,
}

/// do-while 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoStmt {
    pub body: Box<Stmt>,
    pub condition: Expr,
    pub line: usize,
}

/// try 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TryStmt {
    #[serde(default)]
    pub resources: Vec<String>,
    pub try_body: BlockStmt,
    #[serde(default)]
    pub catch_clauses: Vec<CatchClause>,
    #[serde(default)]
    pub finally_body: Option<BlockStmt>,
    pub line: usize,
}

/// catch 子句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatchClause {
    #[serde(default)]
    pub exception_type: Option<String>,
    #[serde(default)]
    pub exception_name: Option<String>,
    pub body: BlockStmt,
    pub line: usize,
}

/// return 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReturnStmt {
    #[serde(default)]
    pub expr: Option<Expr>,
    pub line: usize,
}

/// throw 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrowStmt {
    pub expr: Expr,
    pub line: usize,
}

/// break 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakStmt {
    #[serde(default)]
    pub label: Option<String>,
    pub line: usize,
}

/// continue 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinueStmt {
    #[serde(default)]
    pub label: Option<String>,
    pub line: usize,
}

/// 代码块。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockStmt {
    #[serde(default)]
    pub statements: Vec<Stmt>,
    pub line: usize,
    pub end_line: usize,
}

/// switch 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchStmt {
    pub selector: Expr,
    #[serde(default)]
    pub cases: Vec<SwitchCase>,
    pub line: usize,
}

/// switch case。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchCase {
    #[serde(default)]
    pub label: Option<Expr>, // None = default
    #[serde(default)]
    pub statements: Vec<Stmt>,
    pub line: usize,
}

/// synchronized 语句。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynchronizedStmt {
    pub expr: Expr,
    pub body: BlockStmt,
    pub line: usize,
}

/// 表达式。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum Expr {
    MethodCallExpr(MethodCallExpr),
    FieldAccessExpr(FieldAccessExpr),
    NameExpr(NameExpr),
    LiteralExpr(LiteralExpr),
    BinaryExpr(BinaryExpr),
    UnaryExpr(UnaryExpr),
    AssignExpr(AssignExpr),
    CastExpr(CastExpr),
    ConditionalExpr(ConditionalExpr),
    ArrayAccessExpr(ArrayAccessExpr),
    ArrayCreationExpr(ArrayCreationExpr),
    ObjectCreationExpr(ObjectCreationExpr),
    ThisExpr(ThisExpr),
    SuperExpr(SuperExpr),
    InstanceOfExpr(InstanceOfExpr),
    LambdaExpr(LambdaExpr),
    MethodReferenceExpr(MethodReferenceExpr),
    /// for 循环的初始化声明（如 `for (int i = 0; ...)`）。
    /// 复用 `VarDeclStmt` 结构承载 `var_type`/`declarations`/`line`。
    #[serde(rename = "VariableDeclarationExpr")]
    VariableDeclarationExpr(VarDeclStmt),
    EnclosedExpr {
        inner: Box<Expr>,
        line: usize,
    },
    /// 未知表达式类型（Java 侧序列化器未覆盖的表达式）。
    /// 降级为占位节点，保证单个文件解析失败不会中断整个扫描。
    UnknownExpr {
        #[serde(default)]
        line: usize,
        #[serde(default)]
        value: String,
    },
}

/// 方法调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodCallExpr {
    #[serde(default)]
    pub callee: Option<String>,
    pub method_name: String,
    #[serde(default)]
    pub arguments: Vec<Expr>,
    pub line: usize,
}

/// 字段访问。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldAccessExpr {
    pub target: Box<Expr>,
    pub field: String,
    pub line: usize,
}

/// 标识符引用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameExpr {
    pub name: String,
    pub line: usize,
}

/// 字面量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteralExpr {
    pub value: String,
    #[serde(default)]
    pub literal_type: Option<String>,
    pub line: usize,
}

/// 二元运算。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryExpr {
    pub left: Box<Expr>,
    pub op: String,
    pub right: Box<Expr>,
    pub line: usize,
}

/// 一元运算。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnaryExpr {
    pub expr: Box<Expr>,
    pub op: String,
    #[serde(default)]
    pub prefix: bool,
    pub line: usize,
}

/// 赋值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignExpr {
    pub target: Box<Expr>,
    pub op: String,
    pub value: Box<Expr>,
    pub line: usize,
}

/// 类型转换。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastExpr {
    pub cast_type: String,
    pub expr: Box<Expr>,
    pub line: usize,
}

/// 三元条件表达式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalExpr {
    pub condition: Box<Expr>,
    pub then_expr: Box<Expr>,
    pub else_expr: Box<Expr>,
    pub line: usize,
}

/// 数组访问。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayAccessExpr {
    pub array: Box<Expr>,
    pub index: Box<Expr>,
    pub line: usize,
}

/// 数组创建。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayCreationExpr {
    pub element_type: String,
    #[serde(default)]
    pub initializer: Vec<Expr>,
    pub line: usize,
}

/// 对象创建（new）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectCreationExpr {
    pub class_name: String,
    #[serde(default)]
    pub arguments: Vec<Expr>,
    pub line: usize,
}

/// this 引用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThisExpr {
    pub line: usize,
}

/// super 引用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperExpr {
    pub line: usize,
}

/// instanceof。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceOfExpr {
    pub expr: Box<Expr>,
    pub check_type: String,
    pub line: usize,
}

/// Lambda 表达式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LambdaExpr {
    #[serde(default)]
    pub parameters: Vec<String>,
    pub body: Box<Stmt>,
    pub line: usize,
}

/// 方法引用（::）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodReferenceExpr {
    #[serde(default)]
    pub target: Option<String>,
    pub method: String,
    pub line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_unit() -> CompilationUnit {
        CompilationUnit {
            package: Some("com.example".to_string()),
            imports: vec![ImportDecl {
                package: "java.util".to_string(),
                is_wildcard: false,
                is_static: false,
                line: 1,
            }],
            types: vec![TypeDecl::ClassDeclaration(ClassDecl {
                name: "Foo".to_string(),
                modifiers: vec!["public".to_string()],
                annotations: vec![],
                extends: None,
                implements: vec![],
                members: vec![MemberDecl::MethodDeclaration(MethodDecl {
                    name: "bar".to_string(),
                    modifiers: vec![],
                    annotations: vec![],
                    return_type: Some("void".to_string()),
                    parameters: vec![],
                    body: Some(BlockStmt {
                        statements: vec![],
                        line: 2,
                        end_line: 4,
                    }),
                    line: 2,
                    end_line: 4,
                })],
                line: 1,
                end_line: 5,
            })],
            source_file: "Foo.java".to_string(),
            source_lines: vec![],
            raw_json: String::new(),
        }
    }

    /// 手工构造的 AST 应可正常逐层导航（校验模型结构本身）。
    ///
    /// 注意：这里只做「构造 + 读取」，不做 `serde_json::to_string`。
    /// `Expr`/`Stmt` 是相互递归的 internally-tagged 枚举，对其做序列化会让
    /// serde 的 `TaggedSerializer` 在单态化阶段无限嵌套（编译期 recursion limit），
    /// 而本项目里 AST 只从 JavaParser 的 JSON **反序列化**，不反向序列化。
    #[test]
    fn sample_unit_structure_is_navigable() {
        let unit = sample_unit();
        assert_eq!(unit.package.as_deref(), Some("com.example"));
        assert_eq!(unit.imports.len(), 1);
        assert_eq!(unit.imports[0].package, "java.util");
        assert_eq!(unit.source_file, "Foo.java");
        match &unit.types[0] {
            TypeDecl::ClassDeclaration(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.modifiers, vec!["public".to_string()]);
                assert_eq!(c.line, 1);
                assert_eq!(c.end_line, 5);
                match &c.members[0] {
                    MemberDecl::MethodDeclaration(m) => {
                        assert_eq!(m.name, "bar");
                        assert_eq!(m.return_type.as_deref(), Some("void"));
                        assert_eq!(m.body.as_ref().unwrap().end_line, 4);
                    }
                    _ => panic!("expected MethodDeclaration"),
                }
            }
            _ => panic!("expected ClassDeclaration"),
        }
    }

    /// 解析器输出的完整 JSON 应能反序列化成 `CompilationUnit`（真实使用路径）。
    #[test]
    fn compilation_unit_deserializes_from_parser_json() {
        let v = json!({
            "package": "com.example",
            "imports": [
                { "package": "java.util", "is_wildcard": true, "line": 3 }
            ],
            "types": [{
                "kind": "ClassDeclaration",
                "name": "Foo",
                "modifiers": ["public"],
                "line": 5,
                "end_line": 12,
                "members": [{
                    "kind": "MethodDeclaration",
                    "name": "bar",
                    "return_type": "void",
                    "line": 6,
                    "end_line": 9,
                    "body": { "statements": [], "line": 6, "end_line": 9 }
                }]
            }],
            "source_file": "Foo.java"
        });
        let unit: CompilationUnit = serde_json::from_value(v).unwrap();
        assert_eq!(unit.package.as_deref(), Some("com.example"));
        assert!(unit.imports[0].is_wildcard);
        assert_eq!(unit.source_file, "Foo.java");
        match &unit.types[0] {
            TypeDecl::ClassDeclaration(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.end_line, 12);
                assert_eq!(c.members.len(), 1);
            }
            _ => panic!("expected ClassDeclaration"),
        }
    }

    /// `raw_json` 标了 `#[serde(skip)]`：即使 JSON 里带该字段也不会被读入。
    #[test]
    fn raw_json_field_is_skipped_by_serde() {
        let v = json!({
            "package": null,
            "imports": [],
            "types": [],
            "raw_json": "should-be-ignored"
        });
        let unit: CompilationUnit = serde_json::from_value(v).unwrap();
        assert!(unit.raw_json.is_empty(), "raw_json 应被 serde skip，不从 JSON 读取");
    }

    #[test]
    fn typedecl_tag_deserialization() {
        let v = json!({
            "kind": "ClassDeclaration",
            "name": "X",
            "line": 1,
            "end_line": 2,
            "members": []
        });
        let td: TypeDecl = serde_json::from_value(v).unwrap();
        match td {
            TypeDecl::ClassDeclaration(c) => assert_eq!(c.name, "X"),
            _ => panic!("expected ClassDeclaration"),
        }
    }

    #[test]
    fn memberdecl_tag_deserialization() {
        let v = json!({
            "kind": "FieldDeclaration",
            "name": "count",
            "field_type": "int",
            "line": 3
        });
        let m: MemberDecl = serde_json::from_value(v).unwrap();
        match m {
            MemberDecl::FieldDeclaration(f) => assert_eq!(f.name, "count"),
            _ => panic!("expected FieldDeclaration"),
        }
    }

    #[test]
    fn stmt_tag_deserialization() {
        let v = json!({
            "kind": "ExpressionStmt",
            "line": 1,
            "expr": { "kind": "NameExpr", "name": "x", "line": 1 }
        });
        let s: Stmt = serde_json::from_value(v).unwrap();
        match s {
            Stmt::ExpressionStmt(es) => match es.expr {
                Expr::NameExpr(n) => assert_eq!(n.name, "x"),
                _ => panic!("expected NameExpr"),
            },
            _ => panic!("expected ExpressionStmt"),
        }
    }

    /// 回归测试：JavaParser 输出的是驼峰 `isWildcard`/`isStatic`/`sourceFile`。
    ///
    /// 早期只声明了 snake_case 字段 + `#[serde(default)]`，驼峰键被静默忽略，
    /// 导致 `is_wildcard` 恒为 false、J003（禁止通配符 import）永远不触发。
    #[test]
    fn camel_case_keys_from_java_parser_are_accepted() {
        let v = json!({
            "package": "com.example",
            "imports": [
                { "package": "java.util.*", "isWildcard": true, "isStatic": false, "line": 3 },
                { "package": "java.util.Objects.requireNonNull", "isWildcard": false, "isStatic": true, "line": 4 }
            ],
            "types": [],
            "sourceFile": "Foo.java"
        });
        let unit: CompilationUnit = serde_json::from_value(v).unwrap();
        assert!(unit.imports[0].is_wildcard, "驼峰 isWildcard 应被识别");
        assert!(unit.imports[1].is_static, "驼峰 isStatic 应被识别");
        assert_eq!(unit.source_file, "Foo.java", "驼峰 sourceFile 应被识别");
    }

    /// 回归测试：Java 侧序列化器对未覆盖的表达式类型输出 `UnknownExpr` 兜底节点，
    /// Rust 侧必须能反序列化，避免单个文件解析失败中断整个扫描。
    #[test]
    fn unknown_expr_fallback_deserializes() {
        let v = json!({
            "kind": "UnknownExpr",
            "value": "SomeType.class",
            "line": 10
        });
        let e: Expr = serde_json::from_value(v).unwrap();
        match e {
            Expr::UnknownExpr { line, value } => {
                assert_eq!(line, 10);
                assert_eq!(value, "SomeType.class");
            }
            _ => panic!("expected UnknownExpr"),
        }
    }

    /// 回归测试：Java 侧 EnclosedExpr 输出 `{kind, inner, line}` 结构。
    /// 早期 Rust 侧声明为新类型 `EnclosedExpr(Box<Expr>)`，反序列化时
    /// 找不到 `kind` 字段而报 "missing field `kind`"，任何带括号表达式的
    /// 文件都会解析失败。
    #[test]
    fn enclosed_expr_with_inner_and_line_deserializes() {
        let v = json!({
            "kind": "EnclosedExpr",
            "inner": { "kind": "NameExpr", "name": "x", "line": 3 },
            "line": 3
        });
        let e: Expr = serde_json::from_value(v).unwrap();
        match e {
            Expr::EnclosedExpr { inner, line } => {
                assert_eq!(line, 3);
                match inner.as_ref() {
                    Expr::NameExpr(n) => assert_eq!(n.name, "x"),
                    _ => panic!("expected inner NameExpr"),
                }
            }
            _ => panic!("expected EnclosedExpr"),
        }
    }

    /// 回归测试：ArrayCreationExpr（Java 侧早期漏序列化，落入 UnknownExpr）。
    #[test]
    fn array_creation_expr_deserializes() {
        let v = json!({
            "kind": "ArrayCreationExpr",
            "element_type": "java.util.concurrent.Executor",
            "initializer": [
                { "kind": "NameExpr", "name": "taskExecutor", "line": 5 }
            ],
            "line": 5
        });
        let e: Expr = serde_json::from_value(v).unwrap();
        match e {
            Expr::ArrayCreationExpr(ac) => {
                assert_eq!(ac.element_type, "java.util.concurrent.Executor");
                assert_eq!(ac.initializer.len(), 1);
            }
            _ => panic!("expected ArrayCreationExpr"),
        }
    }

    /// 回归测试：MethodReferenceExpr（Java 侧早期漏序列化，落入 UnknownExpr）。
    #[test]
    fn method_reference_expr_deserializes() {
        let v = json!({
            "kind": "MethodReferenceExpr",
            "target": "taskExecutor",
            "method": "execute",
            "line": 7
        });
        let e: Expr = serde_json::from_value(v).unwrap();
        match e {
            Expr::MethodReferenceExpr(mr) => {
                assert_eq!(mr.target.as_deref(), Some("taskExecutor"));
                assert_eq!(mr.method, "execute");
            }
            _ => panic!("expected MethodReferenceExpr"),
        }
    }

    #[test]
    fn import_defaults_apply() {
        // 缺少 is_wildcard/is_static 时应取默认值 false
        let v = json!({ "package": "java.util", "line": 1 });
        let imp: ImportDecl = serde_json::from_value(v).unwrap();
        assert!(!imp.is_wildcard);
        assert!(!imp.is_static);
    }
}
