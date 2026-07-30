//! Java AST 节点模型。与 JavaParser 序列化的 JSON 格式一一对应。

use serde::{Deserialize, Serialize};

/// Java 编译单元（一个 .java 文件的 AST 根节点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationUnit {
    pub package: Option<String>,
    pub imports: Vec<ImportDecl>,
    pub types: Vec<TypeDecl>,
    #[serde(default)]
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
    #[serde(default)]
    pub is_wildcard: bool,
    #[serde(default)]
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
    EnclosedExpr(Box<Expr>),
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
