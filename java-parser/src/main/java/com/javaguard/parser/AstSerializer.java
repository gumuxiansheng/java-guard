package com.javaguard.parser;

import com.github.javaparser.ast.CompilationUnit;
import com.github.javaparser.ast.ImportDeclaration;
import com.github.javaparser.ast.Modifier;
import com.github.javaparser.ast.Node;
import com.github.javaparser.ast.PackageDeclaration;
import com.github.javaparser.ast.body.*;
import com.github.javaparser.ast.expr.*;
import com.github.javaparser.ast.NodeList;
import com.github.javaparser.ast.stmt.*;
import com.github.javaparser.ast.type.Type;

import java.util.*;
import java.util.stream.Collectors;

/**
 * 将 JavaParser AST 序列化为 Rust 侧可反序列化的 JSON 格式。
 *
 * 设计：只保留规则引擎需要的字段，丢弃无关的 token 范围、注释等。
 * 注意：本类必须兼容 Java 8（不使用 pattern matching instanceof / switch 表达式等
 * Java 16+ 语法），以便产物 jar 能在 JDK 8 运行时上加载。
 */
public class AstSerializer {

    public Map<String, Object> serialize(CompilationUnit cu, String filename) {
        Map<String, Object> root = new LinkedHashMap<>();

        // package
        Optional<PackageDeclaration> pkg = cu.getPackageDeclaration();
        root.put("package", pkg.map(p -> p.getNameAsString()).orElse(null));

        // imports
        List<Map<String, Object>> imports = new ArrayList<>();
        for (ImportDeclaration imp : cu.getImports()) {
            Map<String, Object> impMap = new LinkedHashMap<>();
            impMap.put("package", imp.getNameAsString());
            // 与文件内其余键名统一为 snake_case。
            // Rust 侧 ImportDecl 同时声明了驼峰 alias，因此已发布的旧 jar 仍能正常解析。
            impMap.put("is_wildcard", imp.isAsterisk());
            impMap.put("is_static", imp.isStatic());
            impMap.put("line", imp.getBegin().map(p -> p.line).orElse(0));
            imports.add(impMap);
        }
        root.put("imports", imports);

        // types
        List<Map<String, Object>> types = new ArrayList<>();
        for (TypeDeclaration<?> td : cu.getTypes()) {
            types.add(serializeType(td));
        }
        root.put("types", types);

        root.put("source_file", filename);

        return root;
    }

    private Map<String, Object> serializeType(TypeDeclaration<?> td) {
        if (td instanceof ClassOrInterfaceDeclaration) {
            ClassOrInterfaceDeclaration coid = (ClassOrInterfaceDeclaration) td;
            Map<String, Object> map = serializeBodyDeclaration(td);
            map.put("kind", coid.isInterface() ? "InterfaceDeclaration" : "ClassDeclaration");
            map.put("name", coid.getNameAsString());
            map.put("modifiers", serializeModifiers(coid.getModifiers()));
            map.put("annotations", serializeAnnotations(coid.getAnnotations()));
            if (coid.isInterface()) {
                map.put("extends", coid.getExtendedTypes().stream()
                    .map(Type::asString).collect(Collectors.toList()));
            } else {
                map.put("extends", coid.getExtendedTypes().stream()
                    .map(Type::asString).findFirst().orElse(null));
            }
            map.put("implements", coid.getImplementedTypes().stream()
                .map(Type::asString).collect(Collectors.toList()));
            map.put("members", serializeMembers(coid.getMembers()));
            map.put("line", td.getBegin().map(p -> p.line).orElse(0));
            map.put("end_line", td.getEnd().map(p -> p.line).orElse(0));
            return map;
        } else if (td instanceof EnumDeclaration) {
            EnumDeclaration ed = (EnumDeclaration) td;
            Map<String, Object> map = serializeBodyDeclaration(td);
            map.put("kind", "EnumDeclaration");
            map.put("name", ed.getNameAsString());
            map.put("modifiers", serializeModifiers(ed.getModifiers()));
            map.put("annotations", serializeAnnotations(ed.getAnnotations()));
            map.put("implements", ed.getImplementedTypes().stream()
                .map(Type::asString).collect(Collectors.toList()));
            List<Map<String, Object>> constants = new ArrayList<>();
            for (EnumConstantDeclaration ecd : ed.getEntries()) {
                Map<String, Object> c = new LinkedHashMap<>();
                c.put("name", ecd.getNameAsString());
                c.put("annotations", serializeAnnotations(ecd.getAnnotations()));
                c.put("line", ecd.getBegin().map(p -> p.line).orElse(0));
                constants.add(c);
            }
            map.put("constants", constants);
            map.put("members", serializeMembers(ed.getMembers()));
            map.put("line", ed.getBegin().map(p -> p.line).orElse(0));
            map.put("end_line", ed.getEnd().map(p -> p.line).orElse(0));
            return map;
        } else if (td instanceof AnnotationDeclaration) {
            AnnotationDeclaration ad = (AnnotationDeclaration) td;
            Map<String, Object> map = serializeBodyDeclaration(td);
            map.put("kind", "AnnotationDeclaration");
            map.put("name", ad.getNameAsString());
            map.put("modifiers", serializeModifiers(ad.getModifiers()));
            map.put("members", serializeMembers(ad.getMembers()));
            map.put("line", ad.getBegin().map(p -> p.line).orElse(0));
            map.put("end_line", ad.getEnd().map(p -> p.line).orElse(0));
            return map;
        } else {
            // fallback
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "Unknown");
            map.put("name", td.getNameAsString());
            return map;
        }
    }

    private Map<String, Object> serializeBodyDeclaration(BodyDeclaration<?> bd) {
        return new LinkedHashMap<>();
    }

    private List<Map<String, Object>> serializeMembers(List<BodyDeclaration<?>> members) {
        List<Map<String, Object>> result = new ArrayList<>();
        for (BodyDeclaration<?> member : members) {
            if (member instanceof FieldDeclaration) {
                FieldDeclaration fd = (FieldDeclaration) member;
                for (VariableDeclarator vd : fd.getVariables()) {
                    Map<String, Object> map = new LinkedHashMap<>();
                    map.put("kind", "FieldDeclaration");
                    map.put("name", vd.getNameAsString());
                    map.put("modifiers", serializeModifiers(fd.getModifiers()));
                    map.put("annotations", serializeAnnotations(fd.getAnnotations()));
                    map.put("field_type", vd.getType().asString());
                    map.put("initializer", vd.getInitializer().map(this::serializeExpr).orElse(null));
                    map.put("line", vd.getBegin().map(p -> p.line).orElse(0));
                    result.add(map);
                }
            } else if (member instanceof MethodDeclaration) {
                MethodDeclaration md = (MethodDeclaration) member;
                Map<String, Object> map = new LinkedHashMap<>();
                map.put("kind", "MethodDeclaration");
                map.put("name", md.getNameAsString());
                map.put("modifiers", serializeModifiers(md.getModifiers()));
                map.put("annotations", serializeAnnotations(md.getAnnotations()));
                map.put("return_type", md.getType().asString());
                map.put("parameters", serializeParameters(md.getParameters()));
                map.put("body", md.getBody().map(this::serializeBlock).orElse(null));
                map.put("line", md.getBegin().map(p -> p.line).orElse(0));
                map.put("end_line", md.getEnd().map(p -> p.line).orElse(0));
                result.add(map);
            } else if (member instanceof ConstructorDeclaration) {
                ConstructorDeclaration cd = (ConstructorDeclaration) member;
                Map<String, Object> map = new LinkedHashMap<>();
                map.put("kind", "ConstructorDeclaration");
                map.put("name", cd.getNameAsString());
                map.put("modifiers", serializeModifiers(cd.getModifiers()));
                map.put("annotations", serializeAnnotations(cd.getAnnotations()));
                map.put("parameters", serializeParameters(cd.getParameters()));
                map.put("body", serializeBlock(cd.getBody()));
                map.put("line", cd.getBegin().map(p -> p.line).orElse(0));
                map.put("end_line", cd.getEnd().map(p -> p.line).orElse(0));
                result.add(map);
            } else if (member instanceof InitializerDeclaration) {
                InitializerDeclaration id = (InitializerDeclaration) member;
                Map<String, Object> map = new LinkedHashMap<>();
                map.put("kind", "InitializerDeclaration");
                map.put("is_static", id.isStatic());
                map.put("body", serializeBlock(id.getBody()));
                map.put("line", id.getBegin().map(p -> p.line).orElse(0));
                result.add(map);
            } else if (member instanceof TypeDeclaration) {
                TypeDeclaration<?> td = (TypeDeclaration<?>) member;
                Map<String, Object> nested = serializeType(td);
                Map<String, Object> wrapper = new LinkedHashMap<>();
                wrapper.putAll(nested);
                result.add(wrapper);
            }
        }
        return result;
    }

    private List<Map<String, Object>> serializeParameters(List<Parameter> params) {
        List<Map<String, Object>> result = new ArrayList<>();
        for (Parameter p : params) {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("param_type", p.getType().asString());
            map.put("name", p.getNameAsString());
            map.put("annotations", serializeAnnotations(p.getAnnotations()));
            result.add(map);
        }
        return result;
    }

    private Map<String, Object> serializeBlock(BlockStmt block) {
        Map<String, Object> map = new LinkedHashMap<>();
        map.put("kind", "BlockStmt");
        map.put("line", block.getBegin().map(p -> p.line).orElse(0));
        map.put("end_line", block.getEnd().map(p -> p.line).orElse(0));
        List<Map<String, Object>> statements = new ArrayList<>();
        for (Statement stmt : block.getStatements()) {
            statements.add(serializeStmt(stmt));
        }
        map.put("statements", statements);
        return map;
    }

    private Map<String, Object> serializeStmt(Statement stmt) {
        if (stmt instanceof ExpressionStmt) {
            ExpressionStmt es = (ExpressionStmt) stmt;
            if (es.getExpression() instanceof VariableDeclarationExpr) {
                VariableDeclarationExpr vde = (VariableDeclarationExpr) es.getExpression();
                return serializeVarDecl(vde, stmt);
            }
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ExpressionStmt");
            map.put("expr", serializeExpr(es.getExpression()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof IfStmt) {
            IfStmt is = (IfStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "IfStmt");
            map.put("condition", serializeExpr(is.getCondition()));
            map.put("then_stmt", serializeStmt(is.getThenStmt()));
            map.put("else_stmt", is.getElseStmt().map(this::serializeStmt).orElse(null));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof ForStmt) {
            ForStmt fs = (ForStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ForStmt");
            // 序列化初始化 / 条件 / 更新子句，Rust 端才能做循环分析。
            // 多个初始化表达式时仅取第一个（足以覆盖 `for (int i = 0; ...)` 这类主循环变量）。
            if (fs.getInitialization().isEmpty()) {
                map.put("initialization", null);
            } else {
                map.put("initialization", serializeExpr(fs.getInitialization().get(0)));
            }
            map.put("condition", fs.getCompare().map(this::serializeExpr).orElse(null));
            List<Object> update = new ArrayList<>();
            for (Expression e : fs.getUpdate()) {
                update.add(serializeExpr(e));
            }
            map.put("update", update);
            map.put("body", serializeStmt(fs.getBody()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof ForEachStmt) {
            ForEachStmt fes = (ForEachStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ForEachStmt");
            // 序列化循环变量声明、被迭代表达式、循环体。
            // JavaParser 的 ForEachStmt.getVariable() 返回 VariableDeclarationExpr，
            // 复用 serializeExpr 序列化（Rust 端以 VariableDeclarationExpr 变体接收）。
            map.put("variable", serializeExpr(fes.getVariable()));
            map.put("iterable", serializeExpr(fes.getIterable()));
            map.put("body", serializeStmt(fes.getBody()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof WhileStmt) {
            WhileStmt ws = (WhileStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "WhileStmt");
            map.put("condition", serializeExpr(ws.getCondition()));
            map.put("body", serializeStmt(ws.getBody()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof DoStmt) {
            DoStmt ds = (DoStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "DoStmt");
            map.put("body", serializeStmt(ds.getBody()));
            map.put("condition", serializeExpr(ds.getCondition()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof TryStmt) {
            TryStmt ts = (TryStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "TryStmt");
            map.put("try_body", serializeBlock(ts.getTryBlock()));
            List<Map<String, Object>> catches = new ArrayList<>();
            for (CatchClause cc : ts.getCatchClauses()) {
                Map<String, Object> c = new LinkedHashMap<>();
                c.put("exception_type", cc.getParameter().getType().asString());
                c.put("exception_name", cc.getParameter().getNameAsString());
                c.put("body", serializeBlock(cc.getBody()));
                c.put("line", cc.getBegin().map(p -> p.line).orElse(0));
                catches.add(c);
            }
            map.put("catch_clauses", catches);
            map.put("finally_body", ts.getFinallyBlock().map(this::serializeBlock).orElse(null));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof ReturnStmt) {
            ReturnStmt rs = (ReturnStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ReturnStmt");
            map.put("expr", rs.getExpression().map(this::serializeExpr).orElse(null));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof ThrowStmt) {
            ThrowStmt ts = (ThrowStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ThrowStmt");
            map.put("expr", serializeExpr(ts.getExpression()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof BreakStmt) {
            BreakStmt bs = (BreakStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "BreakStmt");
            map.put("label", bs.getLabel().map(l -> l.asString()).orElse(null));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof ContinueStmt) {
            ContinueStmt cs = (ContinueStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "ContinueStmt");
            map.put("label", cs.getLabel().map(l -> l.asString()).orElse(null));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof BlockStmt) {
            BlockStmt bs = (BlockStmt) stmt;
            return serializeBlock(bs);
        } else if (stmt instanceof EmptyStmt) {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "EmptyStmt");
            return map;
        } else if (stmt instanceof SwitchStmt) {
            SwitchStmt ss = (SwitchStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "SwitchStmt");
            map.put("selector", serializeExpr(ss.getSelector()));
            List<Map<String, Object>> cases = new ArrayList<>();
            for (SwitchEntry se : ss.getEntries()) {
                Map<String, Object> c = new LinkedHashMap<>();
                c.put("label", se.getLabels().isEmpty() ? null
                    : serializeExpr(se.getLabels().get(0)));
                List<Map<String, Object>> caseStmts = new ArrayList<>();
                for (Statement s : se.getStatements()) {
                    caseStmts.add(serializeStmt(s));
                }
                c.put("statements", caseStmts);
                c.put("line", se.getBegin().map(p -> p.line).orElse(0));
                cases.add(c);
            }
            map.put("cases", cases);
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else if (stmt instanceof SynchronizedStmt) {
            SynchronizedStmt ss = (SynchronizedStmt) stmt;
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "SynchronizedStmt");
            map.put("expr", serializeExpr(ss.getExpression()));
            map.put("body", serializeBlock(ss.getBody()));
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        } else {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("kind", "UnknownStmt");
            map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
            return map;
        }
    }

    private Map<String, Object> serializeExpr(Expression expr) {
        Map<String, Object> map = new LinkedHashMap<>();

        if (expr instanceof MethodCallExpr) {
            MethodCallExpr mce = (MethodCallExpr) expr;
            map.put("kind", "MethodCallExpr");
            map.put("callee", mce.getScope().map(this::exprToString).orElse(null));
            map.put("method_name", mce.getNameAsString());
            List<Map<String, Object>> args = new ArrayList<>();
            for (Expression arg : mce.getArguments()) {
                args.add(serializeExpr(arg));
            }
            map.put("arguments", args);
            map.put("line", mce.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof FieldAccessExpr) {
            FieldAccessExpr fae = (FieldAccessExpr) expr;
            map.put("kind", "FieldAccessExpr");
            map.put("target", serializeExpr(fae.getScope()));
            map.put("field", fae.getNameAsString());
            map.put("line", fae.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof NameExpr) {
            NameExpr ne = (NameExpr) expr;
            map.put("kind", "NameExpr");
            map.put("name", ne.getNameAsString());
            map.put("line", ne.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof LiteralExpr) {
            LiteralExpr le = (LiteralExpr) expr;
            map.put("kind", "LiteralExpr");
            map.put("value", le.toString());
            map.put("line", le.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof BinaryExpr) {
            BinaryExpr be = (BinaryExpr) expr;
            map.put("kind", "BinaryExpr");
            map.put("left", serializeExpr(be.getLeft()));
            map.put("op", be.getOperator().asString());
            map.put("right", serializeExpr(be.getRight()));
            map.put("line", be.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof UnaryExpr) {
            UnaryExpr ue = (UnaryExpr) expr;
            map.put("kind", "UnaryExpr");
            map.put("expr", serializeExpr(ue.getExpression()));
            map.put("op", ue.getOperator().asString());
            map.put("prefix", ue.getOperator().isPrefix());
            map.put("line", ue.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof AssignExpr) {
            AssignExpr ae = (AssignExpr) expr;
            map.put("kind", "AssignExpr");
            map.put("target", serializeExpr(ae.getTarget()));
            map.put("op", ae.getOperator().asString());
            map.put("value", serializeExpr(ae.getValue()));
            map.put("line", ae.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof CastExpr) {
            CastExpr ce = (CastExpr) expr;
            map.put("kind", "CastExpr");
            map.put("cast_type", ce.getType().asString());
            map.put("expr", serializeExpr(ce.getExpression()));
            map.put("line", ce.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof ConditionalExpr) {
            ConditionalExpr ce = (ConditionalExpr) expr;
            map.put("kind", "ConditionalExpr");
            map.put("condition", serializeExpr(ce.getCondition()));
            map.put("then_expr", serializeExpr(ce.getThenExpr()));
            map.put("else_expr", serializeExpr(ce.getElseExpr()));
            map.put("line", ce.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof ArrayAccessExpr) {
            ArrayAccessExpr aae = (ArrayAccessExpr) expr;
            map.put("kind", "ArrayAccessExpr");
            map.put("array", serializeExpr(aae.getName()));
            map.put("index", serializeExpr(aae.getIndex()));
            map.put("line", aae.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof ObjectCreationExpr) {
            ObjectCreationExpr oce = (ObjectCreationExpr) expr;
            map.put("kind", "ObjectCreationExpr");
            map.put("class_name", oce.getType().asString());
            List<Map<String, Object>> args = new ArrayList<>();
            for (Expression arg : oce.getArguments()) {
                args.add(serializeExpr(arg));
            }
            map.put("arguments", args);
            map.put("line", oce.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof ThisExpr) {
            map.put("kind", "ThisExpr");
            map.put("line", expr.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof SuperExpr) {
            map.put("kind", "SuperExpr");
            map.put("line", expr.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof InstanceOfExpr) {
            InstanceOfExpr ioe = (InstanceOfExpr) expr;
            map.put("kind", "InstanceOfExpr");
            map.put("expr", serializeExpr(ioe.getExpression()));
            map.put("check_type", ioe.getType().asString());
            map.put("line", ioe.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof LambdaExpr) {
            LambdaExpr le = (LambdaExpr) expr;
            map.put("kind", "LambdaExpr");
            List<String> params = le.getParameters().stream()
                .map(p -> p.getNameAsString()).collect(Collectors.toList());
            map.put("parameters", params);
            map.put("body", serializeStmt(le.getBody()));
            map.put("line", le.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof EnclosedExpr) {
            EnclosedExpr ee = (EnclosedExpr) expr;
            map.put("kind", "EnclosedExpr");
            map.put("inner", serializeExpr(ee.getInner()));
            map.put("line", ee.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof VariableDeclarationExpr) {
            // for 循环的初始化声明（也可作为表达式出现），如 `for (int i = 0; ...)`。
            VariableDeclarationExpr vde = (VariableDeclarationExpr) expr;
            map.put("kind", "VariableDeclarationExpr");
            map.put("var_type", vde.getVariables().isEmpty() ? null
                : vde.getVariables().get(0).getType().asString());
            List<Map<String, Object>> decls = new ArrayList<>();
            for (VariableDeclarator vd : vde.getVariables()) {
                Map<String, Object> d = new LinkedHashMap<>();
                d.put("name", vd.getNameAsString());
                d.put("initializer", vd.getInitializer().map(this::serializeExpr).orElse(null));
                decls.add(d);
            }
            map.put("declarations", decls);
            map.put("line", vde.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof ArrayCreationExpr) {
            ArrayCreationExpr ace = (ArrayCreationExpr) expr;
            map.put("kind", "ArrayCreationExpr");
            map.put("element_type", ace.getElementType().asString());
            List<Map<String, Object>> init = new ArrayList<>();
            if (ace.getInitializer().isPresent()) {
                for (Expression v : ace.getInitializer().get().getValues()) {
                    init.add(serializeExpr(v));
                }
            }
            map.put("initializer", init);
            map.put("line", ace.getBegin().map(p -> p.line).orElse(0));
        } else if (expr instanceof MethodReferenceExpr) {
            MethodReferenceExpr mre = (MethodReferenceExpr) expr;
            map.put("kind", "MethodReferenceExpr");
            Expression scope = mre.getScope();
            map.put("target", scope == null ? null : exprToString(scope));
            map.put("method", mre.getIdentifier());
            map.put("line", mre.getBegin().map(p -> p.line).orElse(0));
        } else {
            map.put("kind", "UnknownExpr");
            map.put("value", expr.toString());
            map.put("line", expr.getBegin().map(p -> p.line).orElse(0));
        }

        return map;
    }

    private String exprToString(Expression expr) {
        if (expr instanceof NameExpr) {
            NameExpr ne = (NameExpr) expr;
            return ne.getNameAsString();
        } else if (expr instanceof FieldAccessExpr) {
            FieldAccessExpr fae = (FieldAccessExpr) expr;
            return exprToString(fae.getScope()) + "." + fae.getNameAsString();
        } else if (expr instanceof ThisExpr) {
            return "this";
        } else if (expr instanceof SuperExpr) {
            return "super";
        } else {
            return expr.toString();
        }
    }

    private Map<String, Object> serializeVarDecl(VariableDeclarationExpr vde, Statement stmt) {
        Map<String, Object> map = new LinkedHashMap<>();
        map.put("kind", "VariableDeclarationStmt");
        map.put("var_type", vde.getVariables().isEmpty() ? null
            : vde.getVariables().get(0).getType().asString());
        List<Map<String, Object>> decls = new ArrayList<>();
        for (VariableDeclarator vd : vde.getVariables()) {
            Map<String, Object> d = new LinkedHashMap<>();
            d.put("name", vd.getNameAsString());
            d.put("initializer", vd.getInitializer().map(this::serializeExpr).orElse(null));
            decls.add(d);
        }
        map.put("declarations", decls);
        map.put("line", stmt.getBegin().map(p -> p.line).orElse(0));
        return map;
    }

    private List<String> serializeModifiers(NodeList<Modifier> modifiers) {
        return modifiers.stream()
            .map(m -> m.toString().trim())
            .collect(Collectors.toList());
    }

    private List<Map<String, Object>> serializeAnnotations(NodeList<AnnotationExpr> annotations) {
        List<Map<String, Object>> result = new ArrayList<>();
        for (AnnotationExpr ann : annotations) {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("name", ann.getNameAsString());
            map.put("line", ann.getBegin().map(p -> p.line).orElse(0));
            map.put("members", new ArrayList<>()); // MVP: 简化，不解析注解成员
            result.add(map);
        }
        return result;
    }
}
