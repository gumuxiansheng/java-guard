package com.javaguard.parser;

import com.github.javaparser.JavaParser;
import com.github.javaparser.ParseResult;
import com.github.javaparser.ast.CompilationUnit;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * CLI / Daemon 入口：解析 .java 文件，输出 JSON AST。
 *
 * 用法:
 *   java -jar java-parser.jar --input <file> [--format json]     # 单次解析，输出 JSON 到 stdout
 *   java -jar java-parser.jar --daemon                            # 常驻模式，stdin/stdout 管道通信
 */
public class Main {

    public static void main(String[] args) throws IOException {
        String inputPath = null;
        String format = "json";
        boolean daemon = false;

        for (int i = 0; i < args.length; i++) {
            String arg = args[i];
            if (arg.equals("--input") || arg.equals("-i")) {
                if (i + 1 < args.length) {
                    inputPath = args[++i];
                }
            } else if (arg.equals("--format") || arg.equals("-f")) {
                if (i + 1 < args.length) {
                    format = args[++i];
                }
            } else if (arg.equals("--daemon")) {
                daemon = true;
            } else if (arg.equals("--help") || arg.equals("-h")) {
                printHelp();
                return;
            } else {
                System.err.println("Unknown argument: " + arg);
                System.exit(2);
            }
        }

        if (daemon) {
            runDaemon();
        } else {
            if (inputPath == null) {
                System.err.println("Error: --input is required");
                System.exit(2);
            }
            runOnce(inputPath, format);
        }
    }

    /**
     * 单次解析模式（原 CLI 行为）：读文件、解析、输出 JSON AST 到 stdout。
     */
    private static void runOnce(String inputPath, String format) throws IOException {
        Path file = Paths.get(inputPath);
        String source = new String(Files.readAllBytes(file), StandardCharsets.UTF_8);
        String filename = file.getFileName().toString();

        JavaParser parser = new JavaParser();
        CompilationUnit cu = parseOrExit(parser, source, filename);
        emitAst(cu, filename);
    }

    /**
     * 常驻模式：逐行读取 stdin 上的 JSON 请求，每行返回一个 JSON 响应。
     *
     * 请求（每行一个 JSON）:
     *   {"action": "parse", "name": "Foo.java", "source": "..."}
     *   {"action": "exit"}
     *
     * 响应（每行一个 JSON）:
     *   {"status": "ok", "ast": {...}}
     *   {"status": "error", "message": "..."}
     *
     * JavaParser / Gson / Serializer 均只初始化一次并在请求间复用，
     * 避免每次解析重复加载类库与解析器状态（常驻 JVM 的价值所在）。
     */
    private static void runDaemon() throws IOException {
        BufferedReader reader = new BufferedReader(
                new InputStreamReader(System.in, StandardCharsets.UTF_8));
        // 显式 UTF-8 输出（Windows 上避免系统默认编码污染 stdout）
        PrintWriter out = new PrintWriter(
                new OutputStreamWriter(System.out, StandardCharsets.UTF_8), true);

        JavaParser parser = new JavaParser();
        Gson gson = new GsonBuilder().disableHtmlEscaping().create();

        String line;
        while ((line = reader.readLine()) != null) {
            if (line.trim().isEmpty()) {
                continue;
            }
            JsonObject request;
            try {
                request = gson.fromJson(line, JsonObject.class);
            } catch (Exception e) {
                respondError(out, gson, "malformed request: " + e.getMessage());
                continue;
            }
            if (request == null || !request.has("action")) {
                respondError(out, gson, "missing action");
                continue;
            }
            String action = request.get("action").getAsString();
            if ("exit".equals(action) || "quit".equals(action)) {
                return;
            }
            if (!"parse".equals(action)) {
                respondError(out, gson, "unknown action: " + action);
                continue;
            }

            String name = request.has("name") ? request.get("name").getAsString() : "Unknown.java";
            String source = request.has("source") ? request.get("source").getAsString() : "";

            ParseResult<CompilationUnit> result;
            try {
                result = parser.parse(source);
            } catch (Exception e) {
                respondError(out, gson, "parse threw: " + e.getMessage());
                continue;
            }

            if (!result.isSuccessful()) {
                StringBuilder sb = new StringBuilder("parse error");
                for (com.github.javaparser.Problem p : result.getProblems()) {
                    sb.append("\n  ").append(p.getMessage());
                }
                respondError(out, gson, sb.toString());
                continue;
            }
            CompilationUnit cu = result.getResult().orElse(null);
            if (cu == null) {
                respondError(out, gson, "parser produced no compilation unit");
                continue;
            }

            JsonObject resp = new JsonObject();
            resp.addProperty("status", "ok");
            resp.add("ast", gson.toJsonTree(astToJson(gson, cu, name)));
            out.println(gson.toJson(resp));
        }
    }

    /**
     * 解析失败时打印错误并退出（单次模式专用）。
     */
    private static CompilationUnit parseOrExit(JavaParser parser, String source, String filename) {
        ParseResult<CompilationUnit> result = parser.parse(source);
        if (!result.isSuccessful()) {
            System.err.println("Parse error:");
            result.getProblems().forEach(p ->
                System.err.println("  " + p.getMessage())
            );
            System.exit(1);
        }
        return result.getResult().orElseThrow(
            () -> new RuntimeException("parser produced no compilation unit"));
    }

    /**
     * 序列化 CompilationUnit 为 JSON（gson 树）。
     */
    private static JsonElement astToJson(Gson gson, CompilationUnit cu, String filename) {
        AstSerializer serializer = new AstSerializer();
        Object astJson = serializer.serialize(cu, filename);
        return gson.toJsonTree(astJson);
    }

    /**
     * 打印 AST JSON 到 stdout（单次模式）。
     */
    private static void emitAst(CompilationUnit cu, String filename) {
        Gson gson = new GsonBuilder().disableHtmlEscaping().create();
        System.out.println(gson.toJson(astToJson(gson, cu, filename)));
    }

    private static void respondError(PrintWriter out, Gson gson, String message) {
        JsonObject resp = new JsonObject();
        resp.addProperty("status", "error");
        resp.addProperty("message", message);
        out.println(gson.toJson(resp));
    }

    private static void printHelp() {
        System.out.println("java-parser — Java AST serializer for JavaGuard");
        System.out.println();
        System.out.println("Usage: java -jar java-parser.jar [--input <file>] [--daemon] [--format json]");
        System.out.println();
        System.out.println("Options:");
        System.out.println("  --input, -i <file>   Input .java file (single-shot mode)");
        System.out.println("  --daemon             Resident mode: JSON request/response over stdin/stdout");
        System.out.println("  --format, -f <fmt>   Output format (default: json)");
        System.out.println("  --help, -h           Show this help");
    }
}