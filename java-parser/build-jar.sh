#!/usr/bin/env bash
#
# 手工重建 java-parser.jar（不依赖 Maven）。
#
# 背景：本机 Maven 安装已损坏（classworlds 类加载器缺失），`mvn package` 不可用，
# 但 java-parser 的打包产物必须能随源码一起更新。本脚本用 javac + jar 复现
# maven-shade-plugin 的效果：
#
#   1. 以「现有 shaded jar」为底（其中已含 javaparser-core / gson 的全部类与资源）；
#   2. 用 JDK 8 的 javac 把 src/main/java 下的源码编译覆盖进去；
#   3. 用 jar cfe 重新打包，入口类固定为 com.javaguard.parser.Main。
#
# 之所以强制用 JDK 8 编译：产出的 class 文件版本必须是 52，
# 这样 jar 才能在 JDK 8 运行时上加载（Rust 侧也会挑最老的 java 命令做兼容验证）。
#
# 用法：
#   bash java-parser/build-jar.sh                 # 重建 target/java-parser.jar
#   bash java-parser/build-jar.sh --deploy        # 额外同步到 deploy/java-parser/
#   JAVA_HOME=/path/to/jdk8 bash java-parser/build-jar.sh
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARSER_JAR="$REPO_ROOT/java-parser/target/java-parser.jar"

JAVAPARSER_VERSION="3.28.2"
GSON_VERSION="2.11.0"

SYNC_DEPLOY=0
if [[ "${1:-}" == "--deploy" ]]; then
  SYNC_DEPLOY=1
fi

# ── 1. 定位 JDK（必须是 JDK 8）─────────────────────────────────────────────
# 产物 class 版本必须是 52，因此强制要求 JDK 8 的 javac；
# 高版本 JDK 配 -source/-target 8 虽然也能产出 52，但会隐式链接新版 API，
# 在 JDK 8 运行时上可能 NoSuchMethodError，故不采用。
jdk_major() {
  local home="$1"
  local jc=""
  if [[ -x "$home/bin/javac.exe" ]]; then jc="$home/bin/javac.exe";
  elif [[ -x "$home/bin/javac" ]]; then jc="$home/bin/javac"; fi
  [[ -n "$jc" ]] || return 1
  "$jc" -version 2>&1 | head -1
}

find_jdk_home() {
  local candidates=()
  [[ -n "${JAVA_HOME:-}" ]] && candidates+=("$JAVA_HOME")
  candidates+=(
    "/c/Program Files/Java/jdk1.8.0_202"
    "/c/Program Files/Java/jdk1.8.0_181"
    "/c/Program Files/Java/jdk-8"
    "/usr/lib/jvm/java-8-openjdk-amd64"
  )
  local c ver
  for c in "${candidates[@]}"; do
    ver="$(jdk_major "$c" 2>/dev/null || true)"
    if [[ "$ver" == *"1.8"* ]]; then
      echo "$c"
      return 0
    fi
  done
  return 1
}

if ! JDK_HOME="$(find_jdk_home)"; then
  echo "error: JDK 8 not found (set JAVA_HOME to a JDK 1.8 installation)" >&2
  exit 1
fi

exe() {
  # Git Bash 下 JDK 的裸命令名可能不在 PATH，统一补 .exe 后缀
  if [[ -x "$JDK_HOME/bin/$1.exe" ]]; then
    echo "$JDK_HOME/bin/$1.exe"
  else
    echo "$JDK_HOME/bin/$1"
  fi
}
JAVAC="$(exe javac)"
JAR="$(exe jar)"

echo "JDK   : $JDK_HOME ($(jdk_major "$JDK_HOME"))"
echo "javac : $JAVAC"

if [[ ! -f "$PARSER_JAR" ]]; then
  echo "error: base jar not found: $PARSER_JAR" >&2
  echo "       a prebuilt shaded jar is required as the base." >&2
  exit 1
fi

# ── 2. 依赖 jar ────────────────────────────────────────────────────────────
M2="${HOME:-/c/Users/win11}/.m2/repository"
JAVAPARSER_JAR="$M2/com/github/javaparser/javaparser-core/$JAVAPARSER_VERSION/javaparser-core-$JAVAPARSER_VERSION.jar"
GSON_JAR="$M2/com/google/code/gson/gson/$GSON_VERSION/gson-$GSON_VERSION.jar"

for dep in "$JAVAPARSER_JAR" "$GSON_JAR"; do
  if [[ ! -f "$dep" ]]; then
    echo "error: missing dependency $dep" >&2
    exit 1
  fi
done

# ── 3. 解包 → 编译 → 重打包 ────────────────────────────────────────────────
# 注意：所有传给 JDK 工具的路径都必须是 Windows 风格（cygpath -m 给出 C:/...），
# 直接传 /tmp/... 会被 jar.exe 解析成 <当前盘符>:\tmp\... 而找不到文件。
WORK="$(cygpath -m "$(mktemp -d)")"
trap 'rm -rf "$WORK"' EXIT
STAGE="$WORK/stage"
mkdir -p "$STAGE"

echo "unpack base jar ..."
cp "$PARSER_JAR" "$WORK/base.jar"
( cd "$STAGE" && "$JAR" xf "$WORK/base.jar" )

echo "compile sources (-source/-target 8) ..."
# -d 直接写入 stage，覆盖底包中的旧 class
CP="$(cygpath -m "$JAVAPARSER_JAR");$(cygpath -m "$GSON_JAR")"
"$JAVAC" -encoding UTF-8 -source 8 -target 8 -nowarn -cp "$CP" -d "$STAGE" \
  "$(cygpath -m "$SCRIPT_DIR/src/main/java/com/javaguard/parser/AstSerializer.java")" \
  "$(cygpath -m "$SCRIPT_DIR/src/main/java/com/javaguard/parser/Main.java")"

echo "repack ..."
OUT="$WORK/java-parser.jar"
( cd "$STAGE" && "$JAR" cfe "$OUT" com.javaguard.parser.Main . )

cp "$OUT" "$PARSER_JAR"
echo "updated: $PARSER_JAR"

# ── 4. 冒烟验证：解析一个真实文件，确认注解参数已输出 ──────────────────────
JAVA_BIN="$(exe java)"
SMOKE_SRC="$WORK/Smoke.java"
cat > "$SMOKE_SRC" <<'JAVA'
import org.springframework.context.annotation.ComponentScan;
@ComponentScan(basePackages = {"com.a", "com.b"})
public class Smoke { }
JAVA
SMOKE_JSON="$("$JAVA_BIN" -jar "$(cygpath -m "$PARSER_JAR")" --input "$SMOKE_SRC" 2>&1 || true)"
if echo "$SMOKE_JSON" | grep -q '"basePackages"'; then
  echo "smoke ok: annotation members are serialized"
else
  echo "warn: 'basePackages' not found in smoke output, please verify manually" >&2
  echo "$SMOKE_JSON" | head -5 >&2
fi

# ── 5. 可选同步到 deploy/ ──────────────────────────────────────────────────
if [[ "$SYNC_DEPLOY" == "1" ]]; then
  if [[ -d "$REPO_ROOT/deploy/java-parser" ]]; then
    cp "$PARSER_JAR" "$REPO_ROOT/deploy/java-parser/java-parser.jar"
    echo "synced: $REPO_ROOT/deploy/java-parser/java-parser.jar"
  else
    echo "warn: deploy/java-parser not found, skip sync" >&2
  fi
fi
