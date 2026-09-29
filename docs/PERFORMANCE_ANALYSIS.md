# java-guard 跨平台扫描性能分析报告

- 分析对象：java-guard 0.1.5（`deploy/bin/java-guard.exe`，release）+ java-parser.jar（JDK 1.8.0_202）
- 语料：`C:\Dev\Projects\DBaaber` —— 605 个 `.java`，3.68 MiB（排除 `target/build/.git/node_modules`）
  以及其子模块 `backend/ai-core`（79 个文件，用于热缓存差分）
- 实测机：Windows 11 / 20 逻辑核 / 64 GB RAM / **Windows Defender 实时保护 = 关闭**
- 规则集：39 条定义，**16 条 enabled**（8 条 YAML + 5 条 Rhai + 3 条内置 Rust）

> ⚠️ 环境声明：本次实测在受限沙箱内执行，有两点必须写在前面的失真说明：
> 1. 沙箱 100% 阻断 Rust 版 daemon 池的管道创建（`os error 231`），导致端到端跑的是
>    **per-file JVM 回退路径**；因此「解析阶段耗时」改用 Python 以**完全相同的 jar 与协议**
>    直连测量（见 §2.2）。
> 2. 沙箱放大了进程创建成本（轻量进程基线 695 ms/次，不可信）。
>    **所有绝对时间只能当量级参考，结论以「差值 / 比值 / 每文件速率」为准。**

---

## 0. 结论速览

| # | 结论 | 证据 | 归类 |
|---|------|------|------|
| 1 | **daemon 池的「并行解析」是假的**：`DaemonPool::parse` 持全局 `Mutex<Vec<DaemonParser>>` 跨越整个 IPC 往返，4 个 daemon 与 1 个 daemon 吞吐相同，且因锁争用**比单 daemon 还慢 41%** | 300 文件等价协议实测：单 daemon 3.88 ms/文件、现实现 5.46 ms/文件、修复后 2.09 ms/文件 | **代码** |
| 2 | **池启动失败会静默跌进「每文件一个 JVM」**，实测 `os error 231`（管道忙）100% 复现，扫描从秒级变 94~111 s（**5~10×**），且无重试、无部分降级、无显眼告警 | 605 文件端到端 3 次：100.1 s / 94.8 s / 111 s | **代码 + 平台** |
| 3 | **池无条件预热**：哪怕 AST 缓存 100% 命中、一个文件都不用解析，也会串行拉起 min(核数,4,文件数) 个常驻 JVM | `src/main.rs` 解析器选择分支在 cache 判定之前 | **代码** |
| 4 | **回退路径的 JVM 没有任何内存/启动参数**：`CliParser` 裸调 `java -jar`，8 worker 并发时每个 JVM 默认预留 1/4 物理内存 → 实测 605 文件里 **201 个解析失败**（`Could not create the Java Virtual Machine`） | 单次全量扫描 `Parsed 404 files, 201 errors` | **代码** |
| 5 | 规则阶段是纯 CPU 成本、跨平台一致：16 条规则 ≈ **2.7 ms/文件**（8 条 YAML 各自全树遍历，5 条 Rhai 各自深拷贝整棵 AST + 源码行数组） | 热缓存差分：0 规则 521 ms / 全规则 730 ms（79 文件） | **代码** |
| 6 | Windows 上每文件的**文件系统操作次数**是 3~6 次（源码 open、缓存 miss open、缓存 create+write+rename），NTFS + 安全软件过滤器对「大量小文件新建」最贵 | 缓存写路径 `put()` 的 4 次调用；缓存文件 22 KiB/个（源码 6 KiB，**3.6× 放大**） | **平台放大 + 代码** |
| 7 | 平台差额的量级关系可以这样近似：**Windows 额外耗时 ≈ 文件数 × 单文件固定开销的增量**；因为解析被串行化，Windows 每次 IPC 往返的延迟直接线性放大到全量 | 单次 daemon 往返中位 4.24 ms / 均值 7.44 ms / 最大 75.8 ms，`corr(源字节, 耗时)=0.95` | **代码（串行化放大器）+ 平台** |

---

## 1. 7 秒耗在哪里

### 1.1 逐项实测（Windows，605 文件，Python 直连等价操作）

| 阶段 | 每文件 | 全量(605) | 备注 |
|------|-------:|----------:|------|
| 目录遍历（`scandir` 递归 + 剪枝，代理 `walkdir`） | 0.230 ms | **139 ms** | 单线程，非瓶颈 |
| 源码 `open`+`read` | 1.033 ms | **625 ms** | 大语料冷目录；热子集 0.203 ms |
| AST 缓存 miss 查询（`open` 失败） | 0.119 ms | **72 ms** | 冷启动每文件一次 |
| AST 缓存命中读取 | 0.927 ms | **561 ms** | 22 KiB/文件，共 8.95 MiB |
| daemon 单次 parse 往返 | 4.24 ms(中位) | **2.56 s** | 均值 7.44 ms → **4.50 s** |
| CLI 模式（每文件一个 JVM） | 664 ms | 401 s | 回退路径，8 线程分摊后实测 94~111 s |
| 纯 JVM 启动 | 303 ms | — | daemon TTFR = 685 ms（冷启动→首次响应） |
| 规则（16 条，热缓存差分） | 2.7 ms | **1.63 s** | 见 §1.2 |

### 1.2 规则阶段拆分（热缓存、79 文件、0 解析成本）

| 配置 | 耗时 | 相对 0 规则 |
|------|-----:|------------|
| 0 条规则（`--enable ZZ999`） | 521 / 521 ms | 基线（遍历+读源码+读缓存+反序列化） |
| 仅 J001（1 条 YAML） | 506 ms | ≈ 0 |
| 仅 8 条 YAML | 785 ms | +264 ms |
| 仅 5 条 Rhai | 602 ms | +81 ms |
| 仅 3 条内置 Rust | 526 ms | +5 ms |
| 全部 16 条 | 721 / 730 ms | **+210 ms ≈ 2.7 ms/文件** |

（个别项存在 ±60 ms 量级噪声，但「16 条规则 ≈ 2~3 ms/文件」这一量级是稳定的。）

### 1.3 线性回归：固定开销 vs 每文件开销

同一批已缓存文件按 0/1/4/16/40/79 复制成不同规模语料，热缓存、0 规则、取 3 次最快值：

| 文件数 | 0 | 1 | 4 | 16 | 40 | 79 |
|---|---:|---:|---:|---:|---:|---:|
| 最快耗时(ms) | 896 | 939 | 932 | 894 | 917 | 940 |

**结论**：曲线基本水平 → 固定启动开销 ≈ 900 ms（沙箱放大，真机应 200~400 ms），
热缓存路径的**每文件边际成本仅 ≈ 0.5 ms**。也就是说：
**「缓存命中的扫描」本身非常便宜，7 秒不在 I/O 上，而在「解析 + 规则 + 启动」三块。**

### 1.4 Linux 7 秒的分解（按 605 文件推演）

| 阶段 | 占比 | 说明 |
|------|-----:|------|
| 解析（IPC 往返，**被全局锁串行化**） | ≈ 35~40% | 605 × ≈4 ms ≈ 2.4 s，且无法用多核摊薄 |
| 规则 16 条 | ≈ 25~30% | ≈ 1.6~2.0 s，纯 CPU，跨平台同价 |
| daemon 池冷启动（4 个 JVM 串行 spawn） | ≈ 8~10% | Linux JVM 启动快，约 0.3~0.6 s |
| 缓存读写 + 源码读取 | ≈ 8% | ≈ 0.5 s |
| 目录遍历 + 固定开销 + 报告 | ≈ 10% | ≈ 0.7 s |
| **合计** | 100% | **≈ 5.5~7 s ✓ 与观测吻合** |

---

## 2. 22 s − 7 s = 15 s 的归因

把差额拆成「平台系数」与「代码放大器」两层。关键在于：
**代码里的串行化与逐文件小 I/O，把 Windows 相对 Linux 的单位成本差异线性放大到了全量文件数。**

| 来源 | 平台系数（Win/Linux，经验值） | 被哪段代码放大 | 估算贡献 |
|------|------|------|---------|
| **进程创建 + JVM 冷启动** | 2~4×（CreateProcess 比 fork/exec 贵 5~10×；Windows JVM 启动 ~2×） | `DaemonPool::start` 串行 spawn `min(核数,4,文件数)` 个 JVM，且**无条件执行** | 1~3 s |
| **管道 IPC 单次往返延迟** | 2~3×（匿名管道 + 线程唤醒 + 调度粒度） | `DaemonPool::parse` 全局锁 → **每个文件都要等一次完整往返**，延迟差 × 文件数 | 3~8 s ⬅ **最大项** |
| **文件系统元数据/小文件操作** | 3~10×（NTFS 路径解析、目录索引、安全软件过滤） | 每文件 3~6 次操作：源码 open、缓存 miss open、缓存 `create_dir_all`+`create tmp`+`write`+`rename` | 1~3 s |
| **安全软件实时扫描** | 10~40%（按扫描实现的官方口径；本机已关闭，未计入实测） | 逐文件 open/新建，尤其 605 次新建缓存文件 | 1~6 s（命中时） |
| **内存分配/内存带宽** | 1.2~1.5× | 规则阶段：每条 Rhai 规则深拷贝整棵 AST；每条规则克隆全文与行数组 | 0.5~1 s |
| **常量：CPU 密集的规则执行** | ≈1× | 8 条 YAML 各自全树遍历 | 0（不贡献差额） |

> 也就是说：**差额不是「Windows 慢」这么笼统，而是「每文件一次串行 IPC 往返 + 每文件 3~6 次小文件操作」这两个模式，把平台单价的差异乘上了文件数。**

---

## 3. 代码自身的问题（按优先级）

### P0-1 · daemon 池被全局锁串行化（并行解析形同虚设）

`crates/java-ast/src/bridge.rs`

```rust
pub fn parse(&self, source: &str, filename: &str) -> Result<CompilationUnit, ParseError> {
    let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.len();
    let mut daemons = self.daemons.lock()...;      // ← 锁在整个 IPC 往返期间持有
    let result = daemons[idx].parse(source, filename);   // ← 阻塞式请求/响应
    match result { ... }
}
```

`DaemonParser` 内部的 `stdin/stdout` 已经各自有 `Mutex`，池这一层再加一把**跨 IPC 的大锁**，
使得 8 个 worker 全部排队、4 个 daemon 只有 1 个在干活。

**实测（300 文件，完全相同的 jar 与协议）：**

| 调度方式 | 耗时 | 每文件 |
|---------|-----:|------:|
| A 1 daemon / 1 线程 | 1163 ms | 3.88 ms |
| B **4 daemon + 全局锁（= 现实现）** | 1639 ms | 5.46 ms |
| C 4 daemon + 每 daemon 独立锁（修复后） | 627 ms | 2.09 ms |

现实现比「干脆只用 1 个 daemon」还慢 41%（多线程争锁 + 唤醒开销）；
修复后 **1.85×**（受 Python GIL 限制，Rust 原生多线程应接近 4×）。

**修法**：把 `daemons: Mutex<Vec<DaemonParser>>` 改为
`daemons: Vec<Mutex<DaemonParser>>`（或 `Vec<Arc<DaemonParser>>` 配合 `next` 轮询），
锁只保护「取实例 + 单个 daemon 的独占使用」，不跨越 `DaemonParser` 自身已有的内部锁。

**预期收益**：解析阶段 **2.5~4×**；605 文件从 ≈2.5~3.3 s 降到 ≈0.7~1.0 s；
**Windows 收益更大**，因为 Windows 单次往返延迟更高，串行化放大更狠。

---

### P0-2 · 池启动失败 → 静默跌到「每文件一个 JVM」

`src/main.rs`：

```rust
match DaemonPool::start(&jar_path, &java_cmd, pool_size) {
    Ok(pool) => { eprintln!("Parser: daemon pool ({pool_size} resident JVM(s))"); Arc::new(pool) }
    Err(e) => {
        eprintln!("warn: daemon parser unavailable ({e}), falling back to per-file JVM (slower)");
        Arc::new(cli_parser)      // ← 每文件一次 java -jar
    }
}
```

实测本环境 100% 触发：

```
warn: daemon parser unavailable (failed to invoke java parser: 所有的管道范例都在使用中。 (os error 231)),
      falling back to per-file JVM (slower)
```
（Windows `ERROR_PIPE_BUSY`。同一台机器上用 Python 以同样方式开 4 个带管道的 JVM：20/20 成功，
说明这是**特定进程/环境的管道创建被拦截**，而非 Windows 的普遍限制——企业 EDR、
容器安全策略、CI 沙箱都可能出现同一现象。）

后果（605 文件端到端，实测 3 次）：

| 运行 | 耗时 | 解析结果 |
|------|-----:|---------|
| 1 | 100.1 s | — |
| 2 | 94.8 s | — |
| 3 | 111.5 s | `Parsed 404 files, 201 errors, 0 violations` |

即 **5~10× 的性能悬崖**，而且只在 stderr 留一行 warn，CI 日志里极易被淹没；
`verify.sh` 之类只校验违规条数的自检**不会发现性能退化**。

**修法（三选多）**：
1. `DaemonPool::start` 加**重试 + 退避**（例如 3 次、20/100/300 ms），
   并把「部分成功」视为可用（4 个起到 2 个就用 2 个，而不是整体放弃）；
2. 回退到 CLI 时**强制带 JVM 参数**（见 P0-3）并限制并发 JVM 数；
3. 把降级事件**提升为显式告警**（报告或退出码层面可感知），
   并提供 `--parser-pool-size` / `--require-daemon` 让 CI 能 fail-fast。

**预期收益**：消除 5~10× 的性能悬崖；这是「同一台机器时而 7 s 时而 90 s」这类诡异现象的根因。

---

### P0-3 · 池无条件预热 + 回退路径 JVM 无参数

**预热问题**：`DaemonPool::start` 在「扫描文件数 > 0」时无条件执行，
早于任何 cache 判定。热缓存场景（CI 二次运行）会把 4 个 JVM 拉起来又杀掉，
纯浪费 ≈0.3 s（Linux）~2 s（Windows）。

**回退路径 JVM 无参数**：`CliParser` 裸调 `java -jar`，既没有 `DAEMON_JVM_ARGS` 的
`-Xms32m -Xmx512m` 内存上限，也没有 `-XX:TieredStopAtLevel=1` 的启动加速；
而 worker 数最多 8，于是 8 个 JVM 各自默认预留 1/4 物理内存 →
实测直接造成 **201/605 个文件解析失败**（`Could not create the Java Virtual Machine`）。

**修法**：
- 池改为**惰性启动**（首次 cache miss 才起，或按实际 miss 数决定池大小），
  并**并行 spawn** 而非 for 循环串行 spawn；
- `CliParser` 复用同一套 JVM 参数（尤其加内存上限），并为其加一个 JVM 并发度信号量。

**预期收益**：热缓存场景省 0.3~2 s；CI 场景（缓存不持久时）避免 30% 的解析失败与随之而来的
重试/误报。

---

### P1-1 · AST JSON 的三重序列化与 3.6× 体积放大

一次冷解析的数据流：

```
Java 侧  JavaParser.parse → astToJson(gson, cu) → gson.toJsonTree() → gson.toJson()  (字符串)
   ↓ 管道（605 个请求，上行 616 KiB / 下行 2233 KiB，AST 相对源码放大 3.6×）
Rust 侧  read_line → serde_json::from_str::<Value>(整行)          ← 第 1 次全量反序列化
        → serde_json::from_value::<CompilationUnit>(ast)          ← 第 2 次（typed AST）
        → serde_json::to_string(&value["ast"]) 写入 raw_json      ← 第 3 次（重新序列化）
        → cache.put(raw_json) 落盘                                 ← 22 KiB/文件
Rhai 侧  serde_json::from_str::<Value>(raw_json) → json_to_rhai()  ← 第 4 次（仅冷启动/Rhai）
```

**修法**：
- 请求/响应改为「长度前缀 + 原始 JSON 字节」，Rust 侧只做一次 `from_str`；
- `raw_json` 直接**借用响应行的子串**（`Arc<str>` 切片）而不是 `to_string` 重新序列化；
- 缓存改为存 **CompilationUnit 的紧凑二进制**（如 `bincode`/`postcard`）或压缩 JSON，
  体积可从 22 KiB 降到 ≈6~8 KiB，直接削减后续所有缓存读写成本。

**预期收益**：解析 + 缓存两侧合计 **25~40%**；Windows 上因内存带宽/分配更慢，收益更大。

---

### P1-2 · Rhai 每条规则深拷贝整棵 AST + 全文 + 行数组

`crates/rule-rhai/src/engine.rs`

```rust
let hit = slot.as_ref().is_some_and(|(key, _)| *key == raw_json);   // 22 KiB 字符串比较
Ok(slot.as_ref().expect("hit implies some").1.clone())              // ← 深拷贝整个 AST Dynamic
...
scope.push("lines", arr);                    // 逐行 String clone
scope.push("source", unit.source_text.clone());  // 全文 clone
```

热缓存的**每一条 Rhai 规则、每一个文件**都要重做一次深拷贝 + 全文/行数组克隆（当前 5 条 Rhai 规则）。
另外 `CompilationUnit::attach_source` 在每次（含缓存命中）都会 `source.to_string()` + 按行切分成 `Vec<String>`。

**修法**：把 AST/源码改成 Rhai 的共享值（`Dynamic::from(Arc<...>)`，clone 只增引用计数），
行数组改为按需构造（仅当规则声明需要 `lines`/`source` 时才注入，
可在规则元数据上加 `needs_source: true` 声明）。

**预期收益**：规则阶段 **20~35%**（实测 Rhai 5 条 = 81 ms/79 文件，其中相当部分是拷贝）。

---

### P1-3 · 8 条 YAML 规则各自全树遍历

`crates/rule-yaml/src/adapter.rs::check_unit` → `match_pattern` 每条规则独立走一遍
`walk_type/walk_member/walk_stmt/walk_expr`。8 条 YAML 规则 = 8 次全 AST 遍历（实测 264 ms/79 文件）。

**修法**：把启用中的 YAML pattern 合并为**单次遍历 + 按节点 kind 索引求值**
（`kind → Vec<rule_idx>`，遍历到某节点时只跑该 kind 下的规则）。
`matcher.rs` 已经是「单次遍历 + 统一匹配」的结构，合并的技术前提已具备。

**预期收益**：YAML 规则阶段 **≈5~8×**（8 次遍历 → 1 次），全量规则阶段 **40~55%**。

---

### P1-4 · 缓存落盘：每文件 4 次文件系统操作 + 内容寻址小文件

`crates/java-ast/src/cache.rs::put`

```rust
if path.exists() { return; }                    // 1 次 stat
create_dir_all(parent)                          // 1 次 mkdir（必然失败在 EEXIST 上）
let tmp = path.with_extension("tmp");
std::fs::write(&tmp, raw_json)                  // create + write(22 KiB)
std::fs::rename(&tmp, &path)                    // rename
```

冷启动 = 每文件「1 次 stat + 1 次 mkdir + 1 次新建写 + 1 次 rename」，
外加 `get()` 的 1 次失败 open。**605 个文件 ≈ 3600 次文件系统操作**，全部落在
Windows 最不擅长的「大目录 × 小文件 × 频繁新建」路径上，也正好是安全软件最敏感的模式。

**修法**：
- `parent` 目录只在首次创建时 `create_dir_all`（用一个 `OnceLock`/`Once` 记住）；
- 缓存目录**分片**（`ab/cd/xxx.json`，256 个子目录）降低单目录条目数；
- 更彻底：改为**单一 append-only 数据文件 + 内存索引**（SSTable/LMDB 风格），
  把 605 次独立 create/rename 变成 1 次顺序追加；
- CI 若缓存目录不持久化，直接默认 `--no-cache`（写缓存纯亏）。

**预期收益**：文件 I/O 阶段 **30~60%**（Windows 侧收益远大于 Linux）。

---

## 4. 平台因素：如何在你的机器上自行确认

以下每项都给出「一条命令 + 判读方法」，避免把代码问题误判成平台问题。

| 因素 | 验证命令 | 判读 |
|------|---------|------|
| **daemon 池是否真的起来了**（最关键） | `java-guard scan . 2>&1 \| head -3` | 看到 `Parser: daemon pool (N resident JVM(s))` = 正常；看到 `falling back to per-file JVM` = **命中了 5~10× 悬崖**，先修 P0-2，其它优化都白谈 |
| **安全软件实时防护** | `powershell -c "Get-MpComputerStatus \| fl RealTimeProtectionEnabled"` | `True` → 用 `Get-MpPreference`/`Add-MpPreference -ExclusionPath` 对项目目录 + `.java-guard-cache` 加白名单，实测对比前后差异（本机为 `False`，故本次未计入） |
| **JVM 冷启动成本** | `Measure-Command { java -version }` | Windows 通常 200~600 ms，Linux 50~150 ms；× daemon 数即池启动税 |
| **管道往返延迟** | 运行 `.perf-tmp/micro.py`（见 §5） | 看 `[4] 稳态单次往返 中位/均值/最大`；中位 × 文件数即「解析阶段下限」 |
| **文件系统小操作** | 运行 `.perf-tmp/micro.py` 的 `[1][2][3][3b]` | 每文件 open/read 在 Linux 上通常 0.03~0.1 ms，Windows 0.2~1.0 ms |
| **缓存是否真的在帮忙** | 连续跑两次，比较耗时；或 `ls .java-guard-cache \| wc -l` | 第二次应显著更快；若 CI 每次都是干净检出，缓存只在「写」上花钱，收益为负 |

---

## 5. 优化方案与预期收益

### 阶段一（1~2 天，收益最大、改动最小）

| 项 | 改动 | 预期收益 |
|----|------|---------|
| 1 | `DaemonPool` 去掉跨 IPC 的全局锁（P0-1） | 解析阶段 **2.5~4×**；605 文件省 ≈1.5~2.5 s；Windows 更多 |
| 2 | 池启动加重试/退避/部分可用 + 降级告警（P0-2） | 消除 **5~10×** 悬崖；把「偶发 90 s」变成不可能 |
| 3 | `CliParser` 补 JVM 参数与并发上限（P0-3） | 消除回退路径的 30% 解析失败 |
| 4 | 增加 `--timings`（各阶段耗时 + 缓存命中率 + 池状态）| 让 Linux/Windows/CI 的差异**可测量**，后续优化才能验收 |

### 阶段二（3~5 天）

| 项 | 改动 | 预期收益 |
|----|------|---------|
| 5 | YAML 规则合并为单次遍历（P1-3） | 规则阶段 **40~55%**，605 文件省 ≈0.8~1.1 s（**双平台同收益**） |
| 6 | 池惰性启动 + 并行 spawn（P0-3） | Windows 冷启动省 1~2 s；热缓存场景省 100% |
| 7 | 缓存目录分片 + `create_dir_all` 只做一次（P1-4） | 文件 I/O 阶段 **20~40%**，Windows 收益为主 |
| 8 | `raw_json` 零拷贝 + 缓存改紧凑/压缩格式（P1-1） | 解析+缓存 **25~40%**；缓存体积从 22 KiB/文件 降到 ≈6~8 KiB |

### 阶段三（1 周+）

| 项 | 改动 | 预期收益 |
|----|------|---------|
| 9 | Rhai 共享 AST/源码、按需注入 `lines`（P1-2） | 规则阶段 **20~35%** |
| 10 | 缓存改单文件 append-only 存储 | 新建文件数从 N 降到 1，安全软件敏感度大幅下降 |
| 11 | 请求/响应改长度前缀二进制协议，支持**流水线**（一次投递多个请求） | 把「延迟受限」变成「带宽受限」，IPC 阶段再降 30~50% |

### 综合预期

以 605 文件为基准，把阶段一 + 阶段二做完：

| 场景 | 现状（Windows 观测上限） | 优化后（估算） |
|------|------|------|
| 冷缓存全量扫描（Linux） | ≈7 s | **≈2.5~3.5 s** |
| 冷缓存全量扫描（Windows，无 AV） | ≈（本环境受沙箱限制无法直测） | **≈4~6 s** |
| 冷缓存全量扫描（Windows + AV 开启） | ≈22 s | **≈6~9 s**（AV 部分另需白名单） |
| 热缓存增量扫描 | 固定开销 ≈0.5~0.9 s + 0.5 ms/文件 | ≈0.3~0.5 s |

> 需要强调：**跨平台差距不会消失**（进程创建、管道延迟、NTFS + 安全软件是客观平台税），
> 但代码侧的串行化与逐文件小 I/O 是**放大器**，把它们消掉之后，
> Windows/Linux 的比值应从现在的 ≈3× 收敛到 **≈1.5~1.8×**——这才是"正常"的平台差。

---

## 6. 复现方法

本次所有测量脚本落在 `.perf-tmp/`（临时目录，非版本库内容）：

| 脚本 | 用途 |
|------|------|
| `micro.py <corpus> <jar> [sample]` | 遍历/源码读/缓存读/缓存 miss/daemon 往返/TTFR/载荷放大/CLI 模式/JVM 启动/进程创建基线；**Windows 与 Linux 都能直接跑**，是最重要的对比工具 |
| `pool_probe.py [N]` | 复现并量化 daemon 池全局锁的串行化（A 单实例 / B 现实现 / C 修复后） |
| `par_probe.py <corpus>` | 验证多线程读文件 / 解析 JSON 在目标平台上能否真正并行 |
| `pipe_probe.py` | 复现 `os error 231`（连续 5 轮 × 4 个带管道 JVM） |
| `rule-split.sh` / `linreg.sh` | 规则阶段差分、固定开销 vs 每文件开销 |

**建议**：把 `micro.py` 与 `pool_probe.py` 提升到 `scripts/perf/` 并在 README 里给出
「一台机器跑一次，输出五段耗时」的说明——这样 Linux 侧的 7 s 也能被同样分解，
而不是靠推算。

> Linux 侧补充（本机无法执行）：需要在 Linux 上跑一次 `micro.py`，重点取
> `[1] 目录遍历`、`[2] 源码读取`、`[4] 稳态往返中位/均值/最大`、`[6] 纯 JVM 启动` 四项，
> 与 Windows 数字逐项相除即可得到真实的**平台系数**，用来验证 §2 表格里的经验值。

---

## 7. 实施记录（2026-09-29，阶段一 + 阶段二池改造已落地）

以下改动已实现并通过 `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace`（237 个测试全绿）：

| # | 改动 | 文件 | 实测验证 |
|---|------|------|---------|
| 1 | **去全局锁**：`Mutex<Vec<DaemonParser>>` → `Vec<Mutex<DaemonParser>>`，worker 只在自己分到的 daemon 上持锁，IPC 往返期间其余 daemon 并行服务 | `bridge.rs` | 单元测试覆盖（并行 roundtrip） |
| 2 | **池启动重试 + 部分降级 + 显眼告警**：单实例 spawn 带 3 次退避重试（ERROR_PIPE_BUSY 官方建议重试）；部分失败降级为「用剩余实例」并输出 `PERF-WARN`；仅全部失败才回退 CLI | `bridge.rs` | 沙箱复现 PERF-WARN 路径正常 |
| 3 | **CliParser 补 JVM 参数**：`java -jar` 前加 `CLI_JVM_ARGS`（同 daemon：`-Xshare:auto -XX:TieredStopAtLevel=1 -Xms32m -Xmx512m`） | `bridge.rs` | 回退模式 79+4 文件 **0 解析错误**（修复前 201/605 失败） |
| 4 | **`--timings`**：新增 CLI 开关，输出 rules-load / traverse / diff / check（Σread/Σparse/Σrules）/ report 分解 | `main.rs` | 正常输出（见下） |
| 5 | **惰性池 `LazyParser`**：首次缓存 miss 才启动 daemon 池（`OnceLock` + 并行 spawn）；全缓存命中的增量扫描**一个 JVM 都不拉起**；池失败自动回退 CLI 并显眼告警 | `bridge.rs` + `main.rs` | 79 文件全缓存命中 **98 ms**、无任何 JVM 启动 |
| 6 | **池并行 spawn**：`DaemonPool::start` 用 `thread::scope` 并行拉起全部实例（串行会把池启动时间放大为 size 倍 JVM 启动耗时） | `bridge.rs` | — |

### 沙箱端到端验证（`target/release/java-guard.exe`，绝对值受沙箱放大仅看趋势）

```
# ① 池失败 → PERF-WARN + 回退 + 0 解析错误 + timings：
PERF-WARN: daemon pool unavailable (...all 4 daemon JVM(s) failed to start); falling back to per-file JVM mode...
Parsed 4 files, 0 errors, 1 violations
Timings: total 1506 ms | rules-load 31 | traverse 1 | diff 0 | check 1471 (Σ read 4 / Σ parse 5876 / Σ rules 0) | report 17

# ② 全缓存命中（79 文件）→ 零 JVM 启动：
Parsed 79 files, 0 errors, 4 violations
Timings: total 98 ms | rules-load 25 | traverse 2 | diff 0 | check 50 (Σ read 141 / Σ parse 167 / Σ rules 0) | report 3
```

### 待真机/Linux 验证项

- 去锁后的并行解析收益（沙箱管道被阻断，无法实测 Rust 池；预期对齐 `pool_probe.py` C 组 ≈1.85×，Rust 原生应近 4×）。
- Linux `micro.py` 平台系数四项（遍历/源码读/稳态往返/纯 JVM 启动）。
- 全量 605 文件端到端：预期 Linux 7 s → 2.5~3.5 s；Windows 22 s → 6~9 s。

### 顺带清理（工具链升级导致的存量 lint 漂移，与性能无关）

- `guard-core/reporter.rs`：`ReportFormat` 改派生 `Default`；`io::Error::new(ErrorKind::Other)` → `io::Error::other`。
- `rule-rhai`：线程本地缓存 `Arc<rhai::AST>` → `Rc<rhai::AST>`（线程级缓存本就无需 Send/Sync，更省原子操作）；doc 注释、`const` thread_local、`needless_borrow`、PI 常量。
- `java-ast`：空 doc 注释、`needless_return`、`clamp`、补 `is_empty()`。
- `main.rs`/`j009`/`pipeline.rs`：`map_or` → `is_some_and/is_none_or`、`print_literal`、`retain`、`contains`、`large_enum_variant` allow。
- `.gitignore` 追加 `.perf-tmp/` 与 `hs_err_pid*.log`。
