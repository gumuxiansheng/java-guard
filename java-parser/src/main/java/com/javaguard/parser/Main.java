package com.javaguard.parser;

import com.github.javaparser.JavaParser;
import com.github.javaparser.ParseResult;
import com.github.javaparser.ast.CompilationUnit;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * CLI 入口：解析 .java 文件，输出 JSON AST 到 stdout。
 *
 * 用法: java -jar java-parser.jar --input <file> [--format json]
 */
public class Main {

    public static void main(String[] args) {
        String inputPath = null;
        String format = "json";

        for (int i = 0; i < args.length; i++) {
            switch (args[i]) {
                case "--input", "-i" -> {
                    if (i + 1 < args.length) {
                        inputPath = args[++i];
                    }
                }
                case "--format", "-f" -> {
                    if (i + 1 < args.length) {
                        format = args[++i];
                    }
                }
                case "--help", "-h" -> {
                    printHelp();
                    return;
                }
                default -> {
                    System.err.println("Unknown argument: " + args[i]);
                    System.exit(2);
                }
            }
        }

        if (inputPath == null) {
            System.err.println("Error: --input is required");
            System.exit(2);
        }

        try {
            Path file = Path.of(inputPath);
            String source = Files.readString(file);
            String filename = file.getFileName().toString();

            JavaParser parser = new JavaParser();
            ParseResult<CompilationUnit> result = parser.parse(source);

            if (!result.isSuccessful()) {
                System.err.println("Parse error:");
                result.getProblems().forEach(p ->
                    System.err.println("  " + p.getMessage())
                );
                System.exit(1);
            }

            CompilationUnit cu = result.getResult().orElseThrow();

            AstSerializer serializer = new AstSerializer();
            Object astJson = serializer.serialize(cu, filename);

            Gson gson = new GsonBuilder().disableHtmlEscaping().create();
            System.out.println(gson.toJson(astJson));

        } catch (IOException e) {
            System.err.println("IO error: " + e.getMessage());
            System.exit(2);
        }
    }

    private static void printHelp() {
        System.out.println("java-parser — Java AST serializer for JavaGuard");
        System.out.println();
        System.out.println("Usage: java -jar java-parser.jar --input <file> [--format json]");
        System.out.println();
        System.out.println("Options:");
        System.out.println("  --input, -i <file>   Input .java file");
        System.out.println("  --format, -f <fmt>   Output format (default: json)");
        System.out.println("  --help, -h           Show this help");
    }
}
