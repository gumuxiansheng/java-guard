# 《Java 编码实施策略》规范分类 与 JavaGuard 规则引擎覆盖方案

> **输入文档**：客户提供的《Java 编码实施策略》规范（本地文档，未纳入版本库）
> **分析对象**：java-guard 当前工作区代码（规则引擎实际实现，非设计文档）
> **结论依据**：全部结论均给出源码位置，便于复核。

---

## 〇、实施进展

| 阶段 | 状态 | 说明 |
|---|---|---|
| **P0-1 源码文本进入规则视野** | ✅ 已实施 | 新增 `CompilationUnit.source_text`（全文，保留换行符）与 1-based `source_lines`；统一在 `main.rs::parse_with_cache` 回填，覆盖**当前版本与 `--semantic-diff` 旧版本**两条路径；Rhai 侧注入 `lines` / `line_count` / `source` |
| **P0-2 注解参数序列化** | ✅ 已实施 | `AstSerializer.serializeAnnotations` 解析 `NormalAnnotationExpr` / `SingleMemberAnnotationExpr` / `MarkerAnnotationExpr`，产出 `members[{key,value}]`；YAML 侧新增 `match_members` 谓词可直接匹配注解参数 |
| **P0-3 YAML 上下文谓词 + AST 统一遍历** | ✅ 已实施 | `crates/rule-yaml/src/matcher.rs` 重写为「一次遍历 + 统一字段匹配」；新增 `within` / `not_within` / `in_type` / `in_method`；附带修复了三处既有遍历盲区（字段初始化器、静态初始化块、lambda 体） |
| 附带修复：加载期校验接入真实路径 | ✅ 已实施 | `load_rule_from_entry`（TOML 驱动的实际加载入口）此前**从不调用** `validate()`，非法规则会静默走偏并误报；现已接入 |
| P1 批量补规则 | ⏳ 待实施 | 见第五章 P1 与附录 B |
| P2 跨文件 / 非 Java 文件 | ⏳ 待实施 | `ProjectRule` + 激活死配置 `applies_to` |
| P3 数据流公共库 / 抑制机制 | ⏳ 待实施 | `guard_core::dataflow`、`// noqa` |

> 编辑器能力已在 `docs/RULE_AUTHORING.md` 中同步更新（上下文谓词、`match_members`、
> `lines`/`line_count`/`source` 三个脚本变量）。

---

## 一、文档概览

| 项 | 内容 |
|---|---|
| 标题 | Java 编码实施策略（客户规范） |
| 标准号 | 客户 IT 技术标准（编号从略） |
| 版本 / 日期 | **2.0**，2026-07-02 发布并实施（修订记录 21 条，1.0 起于 2023-05-09） |
| 章节 | 14 章：范围、规范性引用、术语、**4 编程规约（4.1 命名 / 4.2 常量定义 / 4.3 格式 / 4.4 OOP / 4.5 集合处理 / 4.6 并发处理及线程使用 / 4.7 控制语句 / 4.8 注释 / 4.9 校验 / 4.10 日志 / 4.11 正则表达式）**、5 异常、6 工程结构、7 JVM 相关、8 开源软件使用、9 数据库相关、10 国际化与本地化、11 日期、12 响应状态码、13 Spring Boot 相关、14 其他 |
| 效力标记 | 【强制】**83** 处、【推荐】**50** 处、【参考】**5** 处（合计约 138 条带标记条款，另有若干无标记的编号条款） |
| 叙事范式 | 每条 = 效力标记 + 规则正文 +（说明 / 正例 / 反例）。全文正例 45 处、反例 42 处、说明 94 处 —— **约 2/3 的条款自带可对照的正反例**，这对规则落地的准确率非常有利 |

**文档定位**：这不是一份纯风格指南，而是「编码规约 + 运行底座（JVM / 日志 / 数据库 / 框架）」的合体标准，其中约 **1/3 的条款根本无法在源码里被静态判定**（见第六章）。这一点决定了 java-guard 不可能也不应该 100% 覆盖它。

---

## 二、规范主题归类与意图

按主题归为 20 类。表中「可判定层级」沿用第三章的分级定义。

| # | 主题 | 出处 | 主要条款要点（提炼） | 规范意图 | 可判定层级 |
|---|---|---|---|---|---|
| 1 | **命名约定** | 4.1 | 禁 `_`/`$` 起止；禁拼音/中文；类名 UpperCamelCase；方法/参数/成员/局部变量 lowerCamelCase；常量 UPPER_SNAKE_CASE；抽象类 `Abstract`/`Base` 开头；异常类 `Exception` 结尾；测试类 `Test` 结尾；`String[] args` 而非 `String args[]`；POJO 布尔**禁 is 前缀**；包名全小写单数；禁不规范缩写；设计模式入名；接口方法/属性不加修饰符；Service/DAO 接口 + `Impl` 实现；能力型接口用 `-able`；枚举 `Enum` 后缀 + 成员全大写；父子类/同方法不同块禁同名；类型名词置词尾（`startTime`/`nameList`）；Service/DAO 方法前缀 `get/list/count/save/insert/remove/delete/update`；领域模型 `DO/DTO/BO/VO` 后缀；Java 文件名 = 类名；JSP/HTML/XML 小写文件名；Maven 坐标长度（jar 名 ≤64） | 可读性 + 可检索性 + **框架兼容性**（RPC/序列化反射依赖命名）+ 部署可靠性 | L1 / L2 |
| 2 | **常量与魔法值** | 4.2 | 禁魔法值；`long` 必须大写 `L`；常量按功能分门别类（`CacheConsts`/`ConfigConsts`）；常量五层复用（跨应用/应用内/子工程/包内/类内）；值域有限或带延伸属性必须用 `Enum` | 一处定义、消除「同名不同值」引发的隐性不一致（文中给了 `YES="yes"` vs `YES="y"` 导致线上问题的反例） | L1 / L2 / **L4**（跨类重复定义） |
| 3 | **代码格式与排版** | 4.3 | 大括号四则（左括号前不换行 / 左后换行 / 右前换行 / 右后有 else 不换行）；括号内外空格；`if/for/while/switch/do` 与括号间空格；运算符左右空格；**缩进 4 空格禁 tab**；`//` 后一个空格；强制转换无空格；单行 ≤120；参数逗号后空格；**UTF-8 无 BOM + Unix 换行**；方法 ≤80 行；禁对齐空格；语义块间空行；层级缩进；一元运算符无空格；大括号位置；**源文件仅允许 ASCII 空格 (0x20)**；发布前删调试代码 | 消除无意义的风格分歧，让 review 聚焦语义、让 diff 噪声可控 | **L1-文本**（当前引擎不可判定，见 4.5） |
| 4 | **OOP 与类型使用** | 4.4 | 用类名访问静态成员；覆写必须 `@Override`；可变参数放最后且慎用；外部接口禁改签名 + `@Deprecated`；equals 常量在前（`"x".equals(o)`）；包装类比较用 `equals`；浮点禁 `==`/`equals`；DO 属性类型对齐 DB 字段；禁 `new BigDecimal(double)`；POJO/RPC 用包装类型、局部变量用基本类型；POJO 禁属性默认值；`serialVersionUID` 不轻改；构造器禁业务逻辑；POJO 必须 `toString`；禁 `isXxx()` 与 `getXxx()` 并存；`split` 结果取下标需检查；构造器/同名方法聚集；类内方法顺序（public > private > getter/setter）；setter 命名 + 禁业务逻辑；循环内用 `StringBuilder`；`final` 五种使用场景；慎用 `clone`；访问控制从严（8 条细则）；`valueOf` vs `parseInt` | 规避 NPE / 精度丢失 / 序列化失败 / OOM 等**确定性缺陷**，同时约束封装边界 | L1 / L2 / L3 / **L4**（@Override、弃用 API） |
| 5 | **集合处理** | 4.5 | `equals`/`hashCode` 必须成对；`subList` 禁强转 `ArrayList`；`keySet/values/entrySet` 返回集合禁增删；`Collections.emptyList()` 等不可变集合禁增删；`subList` 原集合改动引发 CME；集合转数组必须 `toArray(T[])`；`addAll` 入参判空；`Arrays.asList` 禁增删；`<? extends T>` 禁 add、`<? super T>` 禁 get（PECS）；非泛型→泛型赋值需 `instanceof`；**foreach 内禁 remove/add**；`Comparator` 三条件；diamond 语法；指定集合初始容量；`entrySet` 遍历；Map 能否存 null（附对照表）；有序性/稳定性；用 Set 去重；空判断用 `StringUtils.isEmpty/isBlank` | 规避 `ClassCastException` / `UnsupportedOperationException` / `ConcurrentModificationException` 三类高频运行时异常，以及扩容等性能陷阱 | L1 / L2（Rhai 可达） |
| 6 | **并发与线程安全** | 4.6 | 单例必须线程安全；线程必须有意义命名；**必须用线程池**、禁显式 new Thread；**禁 `Executors` 建池**（必须 `ThreadPoolExecutor`）；`SimpleDateFormat` 禁 `static`（或加锁 / 用 `DateUtils`）；`ThreadLocal` 必须回收且宜 `static`；锁粒度尽可能小；多资源加锁顺序一致；**加解锁必须成对**；`lock()` 必须在 `try` **之外**；`tryLock` 前判断持有；并发更新需加锁 + `version`；定时任务用 `ScheduledExecutorService`；资金场景用悲观锁；`CountDownLatch` 必须 `countDown`；避免共享 `Random`；DCL 延迟初始化隐患；`volatile` 边界；`HashMap` resize 死链；`ArrayList/HashMap/HashSet/StringBuilder` 禁并发混合操作 | 规避死锁 / 内存泄漏 / 竞态 / CPU 飙升等**难复现**缺陷 | L2 / L3（单文件可达）；**跨方法/跨类需 L3+** |
| 7 | **控制语句** | 4.7 | `switch` 每 case 必须 break/return 或注释，且必须有 `default`；String 类型 switch 必须先判 null；`if/else/for/while/do` 必须用大括号；高并发禁「等于」作退出条件；`if-else` ≤3 层（超出用状态模式/卫语句）；复杂条件提取为布尔变量；**禁在条件表达式中赋值**；循环体内操作外提；避免取反逻辑；接口入参保护；参数校验场景清单；**常量写左边**（`if (CONST.equals(x))`） | 控制流可读 + 防竞态「击穿」+ 边界健壮 | L1 / L2（Rhai 可达） |
| 8 | **注释与文档** | 4.8 | 类/属性/方法必须 Javadoc（`/** */`，禁 `//`）；**抽象方法与接口方法必须 Javadoc**；类注释须含作者/变更人/**复核人**/日期；方法内注释位置规则；枚举字段必须注释；中文优先（专有名词例外）；注释随代码同步；注释掉的代码须说明或删除；注释质量与「避免过滥」；`TODO`/`FIXME` 必须标注入人与时间 | 可维护性 + **责任可追溯**（审计/合规硬要求，注释即证据链） | **L1-文本**（需源码或注释流；当前引擎不可判定） |
| 9 | **参数校验** | 4.9 | 必要参数校验（非空/合法性）；用断言或合适工具；避免方法内大量校验；给出「需校验」5 场景与「不需校验」3 场景 | 把校验放在正确层次，兼顾性能与健壮 | L2 部分可达（「是否应校验」属设计判断，**不可静态判定**） |
| 10 | **日志规约** | 4.10 | **禁直接用 Log4j/Logback/Log4j2 API，必须走 SLF4J**；关键日志保留 ≥15 天；必须用占位符 `{}` 而非拼接；低级别日志必须先 `isXxxEnabled()` 判断；`additivity=false` 防重复打印；异常日志须含**现场 + 堆栈**；**禁 `%C/%F/%l/%L/%M`**；推荐异步；按环境分级（生产禁 Debug）；warn vs error 的取舍；国际化产品须全英文；`RollingFile` + 双重滚动；循环内谨慎打印 | 性能可控 + 排障信息完备 + 合规留存 | L1（代码）/ **L5**（保留天数、appender 配置、异步） |
| 11 | **正则表达式** | 4.11 | 保持简洁可读；**预编译**（`Pattern` 静态常量）；合理使用非贪婪量词；避免大量回溯（原子组/负向前瞻）；用 `^ $ \b` 锚定边界 | 性能与可维护性 | L1 / L2 |
| 12 | **异常处理** | 5 | 14 条：可预检查的 RuntimeException 不应靠 catch（`IndexOutOfBounds`/NPE）；异常不做流程控制；禁大段 try-catch；**禁吞异常**（不处理就上抛）；事务 catch 后须手动回滚；**资源必须及时释放（优先 try-with-resources）**；重要异常（`FileNotFoundException`/`SocketTimeoutException`/`ConnectException`/`EOFException`/`SQLException`/`ParseException`）必须捕获 + 日志 + 报监控；**finally 禁 return**；捕获与抛出类型匹配；返回 null 必须注释说明；NPE 六大产生场景；对外用错误码 / 内部用异常 / 跨应用 RPC 用 Result；用有业务含义的自定义异常，禁抛 `RuntimeException`/`Exception`/`Throwable`；DRY 原则 | 异常语义清晰 + 资源不泄漏 + 可观测 | L1 / L2 / L3（NPE 数据流） |
| 13 | **工程结构与分层** | 6 | TBB/IBB/KBB/DBB/FBB 五类构件的依赖规则（**TBB 不可直接调 KBB**、**KBB 之间不可互调**、DBB 封装 DAL 隔离数据访问）；分层领域模型 DO/DTO/BO 定义；配套工程命名示例（`{工程前缀}-{tbb,ibb,kbb,dbb,fbb}[-adaptor\|-impl\|-dal\|-sal\|-util]`） | 把架构约束落到代码结构上 | **L4**（跨模块依赖） |
| 14 | **JVM 运行参数** | 7 | 7.1 内存构成（堆/元空间/线程栈/直接缓冲区/代码缓冲区）、内存分类与评估、堆内 vs CRedis 缓存原则；7.2 参数三档（**监控参数必配**：`-verbose:gc -Xloggc -XX:+PrintGCDetails -XX:+PrintGCDateStamps -XX:+HeapDumpOnOutOfMemoryError -XX:HeapDumpPath -XX:+UseGCLogFileRotation -XX:NumberOfGCLogFiles=8 -XX:GCLogFileSize=50M`；**性能参数必配**：`-server`、`-Xms=-Xmx` ≤ 机器内存 50%、GC 选型、`-XX:MetaspaceSize=-XX:MaxMetaspaceSize`（内存 1/16~1/8）、`ReservedCodeCacheSize`；可选调优：`-Xss`/堆空间分配/`MaxDirectMemorySize`；网络：DNS 缓存 TTL；辅助：`NativeMemoryTracking`/`TraceClassLoading` 生产禁用）；7.3 参数模板（联机/人机交互/批量/数据分析 × 低中高规格）；7.4 调优目标（Young GC 耗时、Full GC 频率等） | 统一运行底座 + 容量可评估 + 跨架构（x86/C86/ARM）差异收敛 | **L5**（部署配置，非源码） |
| 15 | **开源软件使用** | 8 | Fastjson：大对象一次性加载内存易 OOM、**版本 ≤1.2.59 遇 `\x` 结尾转义直接 OOM**、推荐 Jackson；Log4j2 异步：`AsyncLogger` 与 `AsyncAppender` **禁止对同一条日志同时使用**、全局异步用 `AsyncLoggerContextSelector`、需引 `com.lmax:disruptor`、缓冲区与刷新策略 | 规避已知 CVE / 性能坑 | L1（代码用法）/ **L4**（依赖版本） |
| 16 | **数据库相关** | 9 | 读取三方式（直接读取=小数据量、流式读取=大数据量、**游标读取禁用**）；druid 13 个参数逐一给出「含义/默认值/最佳实践/使用场景」（`initialSize`/`maxActive` 20-50/`minIdle`/`maxWait`(慎 -1)/`useUnfairLock`/`validationQuery`/`validationQueryTimeout`/`testOnBorrow`=false/`testOnReturn`=false/`testWhileIdle`=true/`timeBetweenEvictionRunsMillis`=60000/`minEvictableIdleTimeMillis`/`maxEvictableIdleTimeMillis`）；禁止全局自动提交设 false；连接超时**优先 `socketTimeout`（JDBC URL），不推荐 `StatementTimeout`**（附误杀场景表 + 需 catch 的异常类） | 数据库访问可控、可观测、可排障 | L1（代码 API）/ **L5**（连接池与 URL 配置） |
| 17 | **国际化与本地化** | 10 | 字符集：JVM 必须 `-Dfile.encoding=UTF-8`；遗留非 Unicode 文件须走 `InputStream/OutputStream` + 平台多字符集 SDK；`Character.is*` 应用 int 码点；汉字范围检查须覆盖 CJK 扩充区；语言环境：`Locale` + `ResourceBundle`（禁硬编码中文提示）；日期/数值按 Locale 格式化（`SimpleDateFormat`/`DecimalFormat`/`MessageFormat`）；时区：`-Duser.timezone=GMT+8`，**禁使用含夏令时的时区**；货币：ISO vs 行内两套 3 位编码需明确并转换 | 跨语言/跨时区/跨币种的正确性 | L1（代码）/ **L5**（启动参数） |
| 18 | **日期时间** | 11 | 5 个日期类（`Date`/`Calendar`/`SimpleDateFormat` 为 JDK8 前，`LocalDateTime`/`DateTimeFormatter` 为 JDK8 后）；`SimpleDateFormat` 注意线程安全，**推荐 `DateTimeFormatter`**；格式化模式串逐字符说明（`yyyy` vs `YYYY` 的 week-based-year 陷阱、`MM` vs `mm`、`HH` vs `hh`）；一般应使用 `"yyyy-MM-dd HH:mm:ss"`；`Calendar` 月份 0-11、星期 1-7 与 `LocalDateTime` 月份 1-12、星期 1-7 的差异 | 日期格式化/运算的高频错误点 | L1 / L2 |
| 19 | **响应状态码与错误码** | 12 | HTTP 状态码使用范围（1XX~5XX 各类含义）；**除有明确含义的状态码外，严禁使用其他自定义值**；错误码须遵守《分布式应用系统接口实施策略》《应用系统错误信息编写规范》 | 对外契约一致性 | L1（常量用法）/ **L4**（跨文档契约） |
| 20 | **Spring Boot 专项** | 13 | 13.1 自动装配：`@SpringBootApplication` 默认扫描主类包及子包，慎用 `@ComponentScan(basePackages=..)` 扩大范围；SDK/子模块应通过 `spring.factories` 或 `@Enable..` 自装配，不应要求调用方 `@ComponentScan`；**禁止创建同名 Bean**；善用 `@ConditionalOn*`；13.2 依赖注入：**建议构造器注入**（禁字段注入）、多实例用 `@Qualifier`、网络/DB 依赖由使用者自建；13.3 接口：用 Validator（`@NotNull` + `@Valid`）+ `@ControllerAdvice`/`@RestControllerAdvice` 全局异常处理；13.4 配置：复杂配置用 `@ConfigurationProperties` 而非 `@Value`；**禁止敏感信息明文写入配置文件**；13.5 启动与预热：**预热逻辑必须在服务注册前完成**（Consul/Sofa 注册时机在 `ServletWebServerInitializedEvent` 之后；`ApplicationRunner`/`CommandLineRunner` 均**晚于**注册） | 框架使用规范化 + 启动期正确性（避免有流量无预热） | L1 / L2 / **L4**（同名 Bean、扫描范围）/ **L5**（时序） |

---

## 三、可判定性分级

同一份规范里条款的「可自动化程度」差异极大。先定义分级，后面的覆盖方案全部基于它。

| 层级 | 含义 | 判定所需信息 | 典型条款 |
|---|---|---|---|
| **L1** | 局部语法模式：单个 AST 节点字段即可判定 | 节点自身字段 | 类名 PascalCase、`Executors.newFixedThreadPool`、`String[] args`、`isSuccess` 布尔字段、`SimpleDateFormat` + `static` |
| **L2** | 结构上下文：需要在节点组合 / 嵌套 / 顺序 / 计数上判定，但**仍在单个文件内** | 单文件完整 AST | `finally` 中 `return`；foreach 内 `remove`；`switch` 缺 `default`；`if-else` 嵌套 >3 层；构造器含业务逻辑；循环内 `String` 拼接 |
| **L3** | 单文件数据流 / 常量传播 | 单文件内的赋值与使用链 | 死循环判定（J009 已实现常量传播）；某个变量是否被并发修改；`lock()` 与 `unlock()` 是否配对 |
| **L4** | 跨文件 / 工程 / 依赖 / 配置契约 | 多文件、模块依赖、依赖清单、外部文档 | TBB 不可调 KBB；Spring 同名 Bean；常量跨类重复定义；`fastjson ≤1.2.59`；`@Override` 缺失 |
| **L5** | 运行时 / 部署 / 流程 / 人文 | JVM 启动脚本、运行期指标、治理流程 | JVM 参数模板；日志保留 15 天；预热时序；注释中的复核人；参数校验的场景取舍 |

**分布感受**（按条款粒度粗估）：

- **L1 ≈ 30%** —— 现有 YAML/Rhai 基本已能覆盖
- **L2 ≈ 30%** —— Rhai 能覆盖（因为 Rhai 拿到的是**完整 raw JSON AST**），不必改引擎
- **L3 ≈ 10%** —— 单文件数据流，Rhai 写起来吃力、Rust 内置规则更合适
- **L4 ≈ 15%** —— **引擎目前完全不具备**跨文件视角
- **L5 ≈ 15%** —— 静态代码分析的原理解释不了，必须换手段

> 这条分布是整个方案的枢纽：**java-guard 的主要缺口不在「AST 不够深」，而在「没有源码文本」「没有注解参数」「没有跨文件视角」** 这三件事上。加模式类型只能多覆盖 L1，而真正被卡住的是 L2-文本（4.3 格式、4.8 注释）和 L4。

---

## 四、JavaGuard 现有规则引擎的能力边界

### 4.1 三层规则 + 一个预留

| 层级 | 载体 | 入口 | 现有规则 |
|---|---|---|---|
| YAML 声明式 | `rules/*.yml` | `rule-yaml` crate，`PatternKind` 枚举（`crates/rule-yaml/src/rule.rs:83-97`） | J001 J003 J004 J005 J007 J010 J014 J017（8 条） |
| Rhai 脚本 | `rules/rhai/*.rhai` | `crates/rule-rhai/src/engine.rs`，注入 `ast` + `config` | J006 J011 J012 J013 J015（5 条） |
| Rust 内置 | `src/rules/*.rs` | `Rule<CompilationUnit>::check_unit` | J008 空 catch、J009 死循环（含常量传播）、J016 catch 抛异常前未记日志（3 条） |
| Java 插件 | `crates/rule-plugin` | `PluginRule::analyze` | **仅 trait 与加载器框架，`PluginLoader::load()` 直接 `Ok(vec![])`，从未启用** |

### 4.2 执行模型（决定了「能看多远」）

- **粒度 = 文件 × 规则**：`Rule<U>::check_unit(&self, unit: &U)`（`crates/guard-core/src/rule.rs:162-187`）。**规则之间不共享状态，规则拿不到第二个文件**。
- **无符号表 / 无类型解析**：只有语法结构，没有「这个变量的声明类型是什么」「这个方法覆写了谁」。
- **无父节点指针**：AST 是纯树，节点没有 `parent`。上下文必须由遍历者自己下传（YAML 侧无此能力，Rhai 侧需手写）。
- **并行粒度**：仅文件级并行，单文件内多规则串行（`src/main.rs`）。
- **增量扫描**：`git diff` + `baseline` 两层过滤，配合 `SpanPolicy::Anchor | Intersect`（`guard-core/src/rule.rs:78-92`）解决「结构类违规的行号漂移」问题——这套机制设计得不错，新增规则时应继续沿用。

### 4.3 YAML 层：精确能力边界

**能做的**：

| 能力 | 说明 |
|---|---|
| 6 种匹配目标 | `MethodCall` / `Import` / `Annotation` / `ClassDeclaration` / `MethodDeclaration` / `FieldDeclaration` |
| 字段白名单 | 见 `rule.rs:141-150`，**未知字段在加载期直接报错**（`rule.rs:115-134`），不会静默失效 |
| 取值语义 | 精确匹配 / glob（`*`）/ 正则（首 `^` 或尾 `$`）/ 列表 = `any_of` |
| 消息占位符 | `{callee} {method} {name} {return_type} {field_type} {package} {line}` |
| 嵌套类递归 | 类/方法/字段/注解均会递归进入嵌套类型 |
| 规则级配置 | `severity` / `category` / `enabled` / `span_policy` / `params`，以及 TOML 侧的 `group` / `applies_to` |

**做不了的**（这是关键）：

1. **布尔组合**：只有字段级的 `any_of`，没有跨字段的 `AND`（天然有）、`OR`、`NOT`。想表达「除 `org.apache.commons.lang3` 外的 `StringUtils`」在 YAML 里无解 —— 所以 J011 只能写成 Rhai。
2. **上下文约束**：无法表达「在 `for-each` 内」「不在 `try` 中」「在带 `@Controller` 的类里」。这是 4.5/4.6/4.7 大量条款的共性需求。
3. **节点关系**：无法表达「方法的参数里含有类型 X」「字段的初始化器是 `new SimpleDateFormat(...)`」。
4. **阈值/计数**：无法表达「方法 >80 行」「if-else >3 层」。J006 因此必须走 Rhai。
5. **缺节点类型**：`ObjectCreation`（`new BigDecimal(double)`）、`TryStmt`（finally 中 return）、`ForEachStmt`（foreach 内 remove）等**均无对应 PatternKind**。
6. **源码文本**：完全不可及（见 4.5）。

### 4.4 Rhai 层：精确能力边界

**核心事实**：Rhai 拿到的是 **`AstSerializer` 输出的完整 JSON**（`engine.rs:81-89`，`scope.push("ast", ...)`），所以**能力上界 = JSON AST 的遍历能力**，而不是某个受限的规则 DSL。J012（192 行）、J013（110 行）、J015（229 行）已经证明单文件内的任意结构、嵌套、顺序、计数逻辑都写得出来。

**真正的限制**（都是实战踩过的坑）：

| 限制 | 事实 | 规避写法 |
|---|---|---|
| 字符串 API 缺失 | **`trim()` 未注册，且静默返回 `()` 而非报错**；`index_of` 不可用 | 只用 `contains` / `starts_with` / `ends_with` / `len` / `sub_string` / `split`（J012/J013/J015 已验证） |
| 函数看不到顶层 `let` | 顶层 `let` 属脚本局部作用域，函数内引用报 `Variable not found` | 清单/常量作**函数参数**逐层下传，或在辅助函数内自我包含（J012/J015 已采用） |
| 深度/调用上限 | `set_max_expr_depths(256,256)` + `set_max_call_levels(512)` + 2,000,000 ops（`engine.rs:52-58`） | 复杂规则别写超深嵌套 |
| 无源码文本 | 只能看 AST，不能看原文本 | ——（见 4.5，需引擎扩展） |
| 无类型信息 | 只能看写法，看不到声明类型 | 靠类名后缀 / 参数类型字符串启发式 |
| 无跨文件 | 每次调用只处理一个文件 | ——（见 5.5） |
| 无抑制机制 | `// noqa` 未实现（`docs/DEV_PLAN.md` 已列为待办） | 暂无 |
| severity 不可控 | severity 取自规则元数据，脚本内不能设置 | 按规则拆条 |

### 4.5 AST 覆盖面的三个硬缺口

这是本节最重要的结论。

#### 缺口 1：**源码文本完全不可用** —— 直接锁死 4.3 与 4.8 两大类　→　**已在 P0-1 修复**

> 修复后：`source_text` + 1-based `source_lines` 由 `parse_with_cache` 统一回填，
> Rhai 可直接读 `lines` / `line_count` / `source`；YAML 侧文本类匹配留待 P1（新增 PatternKind）。

- `CompilationUnit.source_lines: Vec<String>` 存在于模型（`crates/java-ast/src/ast.rs:15`），但 Java 侧 `AstSerializer.serialize()` 的 root **只输出 `package` / `imports` / `types` / `source_file`**（`AstSerializer.java:26-57`），从不输出 `source_lines`；
- Rust 侧 `check_one_file` **已经**把文件按 encoding 解码成了 `source`（`src/main.rs:466-475`），但只把它喂给了解析器（`src/main.rs:483`），**没有回填 `unit.source_lines`**。

结论：**所有依赖「原始文本」的条款当前 0 覆盖**，包括：

- 4.3 全部：缩进 4 空格 / 禁 tab / 单行 ≤120 / `//` 后空格 / UTF-8 无 BOM / Unix 换行 / 大括号位置 / 运算符空格 / 仅允许 ASCII 空格
- 4.8 大部分：Javadoc 存在性、注释格式、`TODO`/`FIXME` 标注人与时间、注释掉的代码
- 4.10 部分：`log.debug("a" + b)` 拼接（需看字面量与 `+`，AST 勉强可做）vs `isDebugEnabled` 判断（AST 可做）
- `// noqa` 抑制机制

#### 缺口 2：**注解参数不序列化** —— 锁死 13 章与 4.4 的一部分　→　**已在 P0-2 修复**

> 修复后：`annotations[].members[{key,value}]` 已有值；YAML 用 `match_members`、Rhai 读
> `annotations[i].members[j]` 均可匹配参数。

`AstSerializer.java:556-566`：

```java
map.put("members", new ArrayList<>()); // MVP: 简化，不解析注解成员
```

注解的**键值参数全部丢失**。直接后果：

| 无法判定的条款 | 需要的参数 |
|---|---|
| 13.1 `@ComponentScan(basePackages={..})` 扩大扫描范围 | `basePackages` |
| 13.1 `@SpringBootApplication(scanBasePackages={..})` | `scanBasePackages` |
| 13.1 `@Bean(name={..})` 同名 Bean | `name` |
| 13.4 `@Value("${..}")` vs `@ConfigurationProperties(prefix=..)` | 注解存在性可判，参数不可判 |
| 13.3 `@NotNull`/`@Valid` | 注解存在性可判（够用） |

Rust 侧模型 `AnnotationMember { key, value }`（`ast.rs:211-215`）已经就位，**只缺 Java 侧填充**。

#### 缺口 3：**没有跨文件 / 工程视角** —— 锁死 6 章与多条 L4 条款　→　**仍待 P2**

> 注意：P0-3 已补上**单文件内**的上下文谓词（`within` / `not_within` / `in_type` / `in_method`），
> 但它解决的是「在哪个类/方法/循环里」，不是「跨文件」。本缺口保持原状。

`check_unit(&unit)` 的签名决定了规则只能看到当前文件。以下全部无法实现：

- 6 章：TBB 不可调 KBB、KBB 之间不可互调（需模块依赖图）
- 4.4：`@Override` 缺失（需父类/接口定义）、过时 API 使用
- 4.2：常量跨类重复定义（文中 `A.YES="yes"` vs `B.YES="y"` 的反例）
- 13.1：禁止同名 Bean（需扫描全工程 Bean 定义）
- 4.5：`Comparator` 三条件、集合类型语义
- 8 章：`fastjson ≤1.2.59`（需读 `pom.xml`）

#### 其他值得记录的边界

| 事实 | 位置 | 影响 |
|---|---|---|
| `RuleEntry.applies_to` **声明后全仓无任何消费代码** | `src/main.rs:1273-1274, 1286` | 目前只能扫 `.java`（`src/scanner.rs:47` 硬编码 `extension == "java"`），非 Java 文件规则无入口 |
| 未覆盖的语句/表达式降级为 `UnknownStmt`/`UnknownExpr`，**带 `value` 全文** | `AstSerializer.java:361-366, 508-512` | 好消息：Rhai 能用 `value` 字段做字符串兜底匹配，不至于完全丢失 |
| `LiteralExpr.value = expr.toString()` | `AstSerializer.java:394-398` | 含引号、含数字后缀 → 恰好可以判定 4.2 的「`long` 必须大写 `L`」（`2l` vs `2L`） |
| `MethodCallExpr.callee` 由 `exprToString` 拼接 | `AstSerializer.java:375, 517-531` | `System.out` / `log` / `Executors` / `lock` 等链式描述子可直接匹配；复杂 callee 会退化为 `toString()` |
| `SwitchCase.label == None` 表示 `default` | `ast.rs:396-402` | 4.7 的「必须含 default」可判定 |
| `TryStmt.resources` 是 `Vec<String>` | `ast.rs:324-325` | 可判定是否用了 try-with-resources（4.5「优先 try-with-resources」） |
| `LambdaExpr.body` 是 `Stmt` | `ast.rs:577-584` | 打破「J009 中 lambda 内 break 不算外层退出」这类作用域判断的关键 |
| 已有 AST 磁盘缓存 `.java-guard-cache/` | CLI `--no-cache` | 跨文件规则可复用该缓存降低代价 |

### 4.6 现有 17 条规则 vs 该规范的覆盖现状

| 规范主题 | 现有规则 | 覆盖率观感 |
|---|---|---|
| 4.1 命名 | J004 类名、J005 方法名、J007 常量 | **约 4/25**（其余：包名、抽象类/异常类/测试类后缀、数组写法、布尔 is 前缀、接口 Impl、枚举、DO/DTO/VO、方法前缀、文件名、Maven 坐标…全缺） |
| 4.2 常量 | —— | 0 |
| 4.3 格式 | —— | **0（被源码文本缺口完全锁死）** |
| 4.4 OOP | —— | 0（仅 J011 涉及 StringUtils 工具类偏好） |
| 4.5 集合 | —— | 0 |
| 4.6 并发 | —— | 0 |
| 4.7 控制语句 | J009 死循环（近似覆盖「循环必须有退出」） | 约 1/13 |
| 4.8 注释 | —— | 0 |
| 4.10 日志 | J001 禁 System.out、J017 禁直连日志实现 | **约 2/15** |
| 4.11 正则 | —— | 0 |
| 5 异常 | J008 空 catch、J016 catch 抛异常前必记日志 | **约 2/14** |
| 8 开源软件 | J010/J012 禁 fastjson、J014/J015 禁非 jackson JSON | **约 2/4**（最好的一块） |
| 13 Spring Boot | J013 Controller 禁 Map 入参 | 约 1/12 |
| 6 / 7 / 9 / 10 / 11 / 12 | —— | 0 |

**总结**：现有规则是「工程实践驱动」的（fastjson、日志门面、空 catch 都来自真实痛点），而非「规范驱动」的。要做规范合规，需要**批量补规则**，但补规则前必须先补引擎能力。

---

## 五、扩展方案

按「投入产出比」排序。P0 三项是不做就无法推进 4.3/4.8 与 13 章的前置条件；P1 之后是把覆盖从 L1 推到 L2/L4。

### P0-1：让「源码文本」进入规则视野（解锁 4.3 / 4.8 / 4.11 文本类 / `// noqa`）　✅ 已实施

> **实施说明（与本文原始方案的差异）**：除 `lines` 外还新增了
> ① `source_text` 全文（保留原始换行符，`\r\n` 类判定需要它）；
> ② `line_count`。原因是 `lines` 为 1-based（含哨兵），`len(lines) == line_count + 1`，
> 若让规则用 `len(lines)` 作遍历上界必然越界 —— 实测已踩到该坑，故用一个显式变量消除它。
> 回填点选在 `parse_with_cache` 而非调用点，使 `--semantic-diff` 的旧版本解析同样受益。

**改动量极小，收益极大。**

Rust 侧（`src/main.rs:483-487`）：

```rust
match parse_with_cache(parser, cache, &source, &rel_path) {
    Ok(mut unit) => {
        if unit.source_file.is_empty() {
            unit.source_file = rel_path.clone();
        }
        // 【新增】把已解码的源码按行回填，供文本类规则使用
        if unit.source_lines.is_empty() {
            unit.source_lines = source.lines().map(|l| l.to_string()).collect();
        }
        ...
```

Rhai 侧（`crates/rule-rhai/src/engine.rs:87-89`）显式注入第二个变量，避免污染 `ast` 的语义：

```rust
scope.push("lines", lines_dynamic(&unit.source_lines));  // 1-based: lines[0] 恒为空串
```

> **建议保持 `lines` 1-based 与 `Violation.line` 对齐**，否则所有文本规则都要手工 ±1，迟早出错。

然后 4.3 的条款就可以直接写成 Rhai（示例：单行 ≤120 且禁 tab）：

```rhai
//! rule: J2xx
//! title: 单行不超过 120 字符且禁用 tab 缩进
//! severity: minor
//! params: max_len=120

fn check_line(no, text, max_len) {
    let vs = [];
    if text.contains("\t") {
        vs.push(#{ line: no, message: "第 " + no + " 行使用了 tab 缩进，应使用 4 个空格" });
    }
    if len(text) > max_len {
        vs.push(#{ line: no, message: "第 " + no + " 行长度 " + len(text) + " 超过 " + max_len });
    }
    return vs;
}

let all = [];
let ml = config["max_len"];
if ml == () { ml = 120; }
let n = len(lines);
let i = 1;
while i <= n {
    all = all + check_line(i, lines[i], ml);
    i = i + 1;
}
all
```

**注意**：`len()` 数的是字符数，该规范条款写的是「字符数」而非字节数，语义一致；但若要严格按 **UTF-16 code unit** 计（与编辑器一致），需在 Rust 侧预计算并随行下发。**建议在 Rust 侧直接下发每行长度**，一次性消除歧义：

```rust
// lines[] 与 line_lengths[] 同时注入，规则侧不用自己算
```

### P0-2：注解参数回填（解锁 13 章与 4.4 的 `@Deprecated` 等）　✅ 已实施

> **实施说明**：只把参数塞进 AST 还不够 —— YAML 的 `Annotation` 原先只能匹配 `name`，
> 参数依然用不上。因此同时新增了 `match_members` 谓词，使 13.1 的
> `@ComponentScan(basePackages=..)` 这类规则可以**纯 YAML** 表达。

Java 侧（`AstSerializer.java:556-566`）把 MVP 占位换成真实解析：

```java
private List<Map<String, Object>> serializeAnnotations(NodeList<AnnotationExpr> annotations) {
    List<Map<String, Object>> result = new ArrayList<>();
    for (AnnotationExpr ann : annotations) {
        Map<String, Object> map = new LinkedHashMap<>();
        map.put("name", ann.getNameAsString());
        map.put("line", ann.getBegin().map(p -> p.line).orElse(0));
        List<Map<String, Object>> members = new ArrayList<>();
        if (ann instanceof NormalAnnotationExpr) {
            for (MemberValuePair pair : ((NormalAnnotationExpr) ann).getPairs()) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("key", pair.getNameAsString());
                m.put("value", pair.getValue().toString());   // 数组/字符串统一取字面量文本
                members.add(m);
            }
        } else if (ann instanceof SingleMemberAnnotationExpr) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("key", "value");
            m.put("value", ((SingleMemberAnnotationExpr) ann).getMemberValue().toString());
            members.add(m);
        }
        map.put("members", members);
        result.add(map);
    }
    return result;
}
```

Rust 侧 `AnnotationMember`（`ast.rs:211-215`）**无需改动**。之后 13.1 就可以用 YAML 直接写：

```yaml
id: J7xx
title: 禁用 @ComponentScan 扩大组件扫描范围
severity: major
category: convention
pattern:
  type: Annotation
  match_fields:
    name: "ComponentScan"
message: "@ComponentScan 会扩大扫描范围（{name}），请改用自动装配（spring.factories / @Enable*）"
```

> 若嫌 `@ComponentScan` 误报（也有正当用法），可退一步只查「同时出现 `@SpringBootApplication` 与 `@ComponentScan`」（需 P1 的 `within` 谓词或 Rhai）。

### P0-3：给 YAML 补上下文谓词（把 L2 条款从「必须写 Rhai」降为「YAML 可写」）　✅ 已实施

> **实施说明（两处与原始方案的差异）**：
> ① **`within` 采用 any_of 而非 all_of**。原文此处表述自相矛盾；按 all_of 理解时
> `within: [ForStmt, ForEachStmt, WhileStmt, DoStmt]`（「在任意循环内」）将永不成立，
> 是个陷阱。最终与 `match_fields` 列表语义对齐，统一为 any_of。
> ② 实施时未按「先摊平成 `Vec<NodeRef>` 再匹配」的字面方案，而是「**单次深度优先遍历 +
> 随遍历维护祖先栈与宿主**」——因为摊平方案要给每个节点存一份祖先链，百万级节点时
> 分配开销可观；随遍历维护上下文是 O(1)/节点，同样只改一处遍历器。
> 两种方案在「新增节点类型只改遍历」这一目标上等价。
> ③ 顺带修复了三处既有遍历盲区（字段初始化器、静态初始化块、lambda 体），
> 并用 3 个回归用例锁住。

当前 YAML 做不了上下文，导致 4.5/4.6/4.7 的 L2 条款全部被迫写 Rhai。建议给 `Pattern` 增加**祖先/宿主约束**：

```yaml
# 建议新增：within / not_within / in_type / in_method 四组谓词
pattern:
  type: MethodCall
  match_fields:
    callee: ".*"
    method: [remove, add, clear, removeAll, retainAll]
  within:                       # 命中节点必须位于以下任一祖先之内（all_of 语义，逐条满足）
    - kind: ForEachStmt
  not_within:                   # 命中节点不得位于以下任一祖先之内
    - kind: SynchronizedStmt
  in_type:
    annotations: ["Controller", "RestController"]   # 复用现有 annotation 匹配语义
  in_method:
    name: "^(get|set)"
```

**实现要点**（`crates/rule-yaml/src/matcher.rs`）：

- 现有 `walk_*` 系列函数**已经是递归下发**（`walk_stmt_for_method_call` / `walk_expr_for_method_call`），只需在签名上追加一个 `ancestors: &mut Vec<&'static str>`（或用小型 arena / 位图），进入子节点前 `push`、离开后 `pop`；
- `within` / `not_within` 判定即对 `ancestors` 做集合运算，**成本极低**；
- matcher.rs 里 `match_type_for_method_call` 等 **12 处重新递归** 的地方要一并改（工作量大但机械）；
- 现有 17 条规则**零破坏**（新字段 `#[serde(default)]` 缺省不启用）。

预估：`matcher.rs` 约 +250~350 行，配套单测约 +200 行。**这是把 YAML 表达力从 L1 推进到 L2 的关键一步**，做完之后 4.5/4.7 中大量条款可以用 5~10 行的 YAML 表达，而不是 100~200 行的 Rhai。

### P1-1：补齐语句/表达式级 `PatternKind`

现缺的节点类型与对应条款：

| 建议新增 PatternKind | 解锁条款 |
|---|---|
| `ObjectCreation`（`class_name` / `arguments`） | 4.4 禁 `new BigDecimal(double)`；4.6 禁直接 `new Thread(...)`；4.6 禁 `new SimpleDateFormat()` 作 static |
| `TryStmt`（`has_finally` / `has_resources` / `catch_count`） | 5 章：finally 中 return、优先 try-with-resources |
| `CatchClause`（`exception_type` / `body_is_empty`） | 5 章：捕获后吞异常；J008 可由 Rust 内置迁到通用层 |
| `ForStmt` / `ForEachStmt` / `WhileStmt` / `DoStmt` | 4.4/4.7/4.10：循环内字符串拼接、循环内日志、循环内建对象/取连接 |
| `ReturnStmt` / `ThrowStmt` | 5 章：finally 中 return、禁用 RuntimeException |
| `FieldAccessExpr`（`target` / `field`） | 4.4：静态成员通过实例访问 |
| `BinaryExpr`（`op` / `left` / `right`） | 4.4 浮点 `==`；4.7 禁条件中赋值（`or` 与 `AssignExpr` 谓词组合） |
| `LiteralExpr`（`value`） | 4.2 `long` 小写 `l`；4.11 正则字面量 |
| `ElementKind`（按 `kind` 通配） | 兜底：`UnknownStmt`/`UnknownExpr` 也能被匹配 |

> 实现上**不必为每种节点写新的匹配函数**。当前 matcher 的问题在于「按 PatternKind 走不同递归路径」。更干净的做法是：**先把 AST 摊平成 `Vec<NodeRef { kind, fields, line, ancestors }>`（一次遍历），再对摊平结果做统一字段匹配**。这样新增节点类型只是往摊平器加一个分支，matcher 完全不用动，且 `within` 谓词天然可用。**建议与 P0-3 合并实施**，总工作量反而比分开做更小。

### P1-2：项目级规则（跨文件）—— 解锁 6 章与多条 L4 条款

新增一条平行于 `Rule<U>` 的 trait：

```rust
// crates/guard-core/src/rule.rs
pub trait ProjectRule: Send + Sync {
    fn id(&self) -> &RuleId;
    fn severity(&self) -> Severity;
    fn description(&self) -> &str;
    /// 两阶段：file 阶段收摘要，project 阶段汇总判定
    fn summarize(&self, unit: &CompilationUnit) -> Summary;   // 可为 no-op
    fn check_project(&self, summaries: &[(String, Summary)]) -> Vec<Violation>;
}
```

配套在 `javaguard.rules.toml` 增 `scope = "file" | "project"`，在 `run_scan` 里做**两阶段调度**。

**但必须注意内存与性能**：全量 AST 常驻内存不可接受（大工程轻松上 GB）。推荐**摘要模式**：file 阶段各规则只吐出自己关心的「小切片」，project 阶段只吃切片。例如：

- 6 章分层规则 → 摘要 = `[(工构件名, 依赖构件名, 行号)]`，与 `pom.xml` 的 `<artifactId>` 对齐后判依赖方向；
- 13.1 同名 Bean 规则 → 摘要 = `[(beanName, 类名, 行号)]`；
- 4.2 常量重复定义 → 摘要 = `[(常量名, 值, 类名, 行号)]`。

这三条规则的摘要体量都很小（KB 级），完全可行。

Rhai 侧同步扩展：为 project 规则提供**输入不是 `ast` 而是 `files`**（摘要数组）的第二种脚本契约。⚠️ **不要**把「全部文件的完整 AST」注入 Rhai —— 既有内存问题，也会撞上 `set_max_array_size(10_000)`（`engine.rs:58`）。

### P2-1：启用非 Java 文件规则（激活死配置 `applies_to`）

现状：`applies_to`（`src/main.rs:1273`）无人消费，`scanner.rs:47` 硬编码只收 `.java`。改动：

1. `scan_java_files` 增加扩展名白名单参数；
2. 对非 `.java` 文件走「文本规则」通道（复用 P0-1 的 `lines`，**不需要** Java 解析器）；
3. `RuleEntry.applies_to` 生效，如 `applies_to = ["pom.xml"]`、`["*.yml"]`。

解锁条款：8 章 `fastjson ≤1.2.59`（`pom.xml`）、9.2 druid 参数（`application.yml`）、4.10 `additivity=false`（`log4j2.xml`）、10 章 `-Dfile.encoding`（启动脚本）、12 章状态码常量。

### P2-2：抑制机制 `// noqa`

`docs/DEV_PLAN.md` 已列为待办。依赖 P0-1（需要源码文本才能看到注释）。建议语法：

```java
System.out.println(x);   // noqa: J001  说明理由
```

### P3-1：把 J009 的数据流能力沉淀为公共库

`src/rules/j009_infinite_loop.rs`（**2250 行**）已经实现了相当完整的**常量传播 + 循环变量修改检测 + 退出路径可达性分析**。这是一笔被埋没的资产 —— 4.4 的 NPE 场景、4.6 的「锁是否成对」、4.5 的「泛型集合赋值」都需要同类能力。

建议抽出 `guard_core::dataflow`（常量表 + 使用链 + 简单可达性），让 Rust 内置规则和 Rhai 规则（通过注册函数）都能复用。**注意**：不要试图在 Rhai 里手写数据流 —— 已踩过的「`trim` 静默返回 unit」这类坑说明 Rhai 边界比看上去窄。

### P3-2：类型解析（SymbolSolver）—— 建议**不做**

引入 JavaParser 的 `SymbolSolver` 需要完整 classpath，工程成本极高，收益与 SpotBugs/ErrorProne 重叠。**建议用替代工具而非自研**（见第六章）。

### P3-3：规则与效力等级对齐

文档用【强制】/【推荐】/【参考】三档，java-guard 用 `info/minor/major/critical`。建议建立映射并在规则元数据里显式标注，让 `--min-severity` 直接对应合规档位：

| 规范效力 | severity | 说明 |
|---|---|---|
| 【强制】 | `major` | 违反即不合规 |
| 【强制】（安全/资金/并发） | `critical` | 叠加领域标签 |
| 【推荐】 | `minor` | 建议改进 |
| 【参考】 | `info` | 提示 |

---

## 六、不适合 YAML / Rhai 的规范：原因与替代实现

这是本次分析中**最需要提前对齐**的部分。以下条款不要试图用 YAML/Rhai 硬做。

### 6.1 需要换实现手段的（java-guard 扩展后可做）

| 规范 | 为什么 YAML/Rhai 做不了 | 可行替代 |
|---|---|---|
| **6 工程结构与分层**（TBB/IBB/KBB/DBB/FBB 依赖规则） | 需要**模块级依赖图**：要读 `pom.xml` 的 `<artifactId>`、解析 `import` 后的包名归属、构建模块间依赖关系。单文件视角永远看不到「TBB 调了 KBB」 | **首推 ArchUnit**（JVM 单测内以 Java 代码声明架构规则，生态成熟、报错清晰）；或按 5.5 的 **ProjectRule + 摘要模式**自研（需同时解决 `pom.xml` 解析） |
| **13.5 启动与预热时序** | 要求「预热逻辑在服务注册前完成」，本质是**运行期事件时序**，静态调用图上无法证明 | Spring Boot 的 `ApplicationListener` 顺序需运行期验证 → 用 **集成测试**（`ApplicationContextRunner` + 断言预热先于注册）+ 各产品自测用例；静态侧最多做「禁止在 `ApplicationRunner`/`CommandLineRunner` 中写预热逻辑」这一条**反例模式**（Rhai 可做） |
| **4.4 `@Override` 缺失** | 需要父类/接口的方法签名，属跨文件类型解析 | **javac 自带** `-Xlint:overrides`（还有 `-Xlint:deprecation` 对应「禁用过时 API」）。**零成本、准确率 100%**，强烈建议直接在 CI 里开 lint，而不是自研 |
| **4.4 浮点 `==`、4.5 泛型 PECS、5 章 NPE 场景** | 需要类型推断与跨方法数据流，规则写出来误报率会很高 | **SpotBugs**（`FE_FLOATING_POINT_EQUALITY`、`NP_*` 系列）、**Error Prone**（编译期，误报低） |
| **9.1 游标读取禁用** | 需要识别 MyBatis/JDBC 的游标 API（`ResultSet.TYPE_SCROLL_*`、MyBatis `Cursor`），且要区分业务语义 | 可用 Rhai 做**局部 API 白名单**（`Cursor`/`TYPE_SCROLL` 出现即提示），但「是否真的用于游标读取」需人工确认 → 标记为 `info` 级别提示而非阻断 |

### 6.2 静态代码分析原理上不可覆盖的（必须换渠道）

| 规范 | 原因 | 替代渠道 |
|---|---|---|
| **7 章 JVM 参数**（监控参数、性能参数、参数模板、调优） | 参数在**启动脚本 / 应用平台配置**里，不在源码里 | ① 对 `Dockerfile`/`start.sh`/`*.yml` 做**启动脚本规则**（P2-1 的非 Java 文件通道，可查 `-Xms == -Xmx`、必配参数是否齐全）；② 平台侧配置基线巡检；③ 运行期用 `jcmd`/`jstat` 比对参数生效值 |
| **7.1 内存评估 / 7.4 调优目标**（Young GC 耗时、Full GC 频率） | 属**运行期指标** | APM/应用平台监控 + 压测报告，人工评审 |
| **4.10 日志保留 15 天、异步日志、缓冲区策略** | 在 `log4j2.xml`/平台配置里 | 配置文件规则（P2-1）+ 运维巡检 |
| **8 章 `fastjson ≤1.2.59` 的 OOM 风险** | 需**依赖版本清单**，不是源码 | ① `pom.xml` 文本规则（P2-1，可做版本比较）；② 更根本的是 **SCA 工具**（OWASP Dependency-Check / Trivy / 平台制品库扫描） |
| **8 章 Log4j2 异步需引 `com.lmax:disruptor`** | 需依赖树 | SCA + `mvn dependency:tree` 规则 |
| **9.2 druid 13 个参数** | 在 `application.yml` 里 | 配置文件规则（P2-1）可覆盖「参数是否存在 / 是否在建议区间」，但**「最佳实践」是运行环境相关判断**，需性能测试佐证 |
| **10 章 `-Dfile.encoding` / `-Duser.timezone`、11 章日期格式** | 启动参数 + 语义判断 | 启动脚本规则 + `DateTimeFormatter` 用法规则（可做）；时区有效性需运行期 |
| **12 章响应状态码与错误码** | 需对照《接口实施策略》《错误信息编写规范》**两份外部文档** | java-guard 可做「禁自定义状态码」的白名单规则（L1）；错误码语义须以那两份文档为准，人工/评审 |
| **4.9 参数校验的场景取舍**（「调用频次低的方法需要校验」「private 方法可以不校验」） | 这是**设计判断**，需要知道调用频次与调用方 | 无法静态判定。只能做「对外接口（`@RestController`/RPC 门面）的入参必须有校验注解」这类**可操作的近似规则**（Rhai 可做）。**务必标注为近似，不要声明为规范全量覆盖** |
| **4.8 注释中的作者/变更人/复核人/日期** | 可做**格式校验**（字段是否存在），无法校验**真实性** | 文本规则可查字段存在性；真实性与时效性属**流程治理**，应在代码评审/提交钩子里约束（可考虑接入已有的 `req-guard`/`doc-guard` 体系） |
| **4.1 Maven 坐标长度、jar 名 ≤64 字符** | 在 `pom.xml` 里 | 配置文件规则（P2-1）可做，属易得项 |

### 6.3 一句话结论

> **java-guard 应当覆盖的是 L1+L2（约占 60%），L3 用 Rust 内置规则兜底，L4 通过 ProjectRule 扩展争取，L5 明确交给「启动脚本规则 + CI lint + SCA + APM + 流程治理」组合。**
> 需要特别向业务方说明：**7 章 JVM、9.2 数据库参数、10 章时区、8 章依赖版本这些「规范中篇幅最大的部分」，恰恰是静态代码分析覆盖不到的**，必须并行推进配置巡检与工具链，否则会形成「工具说合规、审计不认」的落差。

---

## 七、分阶段路线

| 阶段 | 目标 | 关键改动 | 预期收益 |
|---|---|---|---|
| **P0**（前置） | 打通三类数据缺口 | ① 源码文本回填 + `lines` 注入（`main.rs` + `engine.rs`）② 注解参数序列化（`AstSerializer.java`）③ YAML 上下文谓词 + AST 统一遍历（`matcher.rs`） | ✅ **已完成** —— 解锁 4.3 / 4.8 与 13 章大部分；YAML 表达力 L1→L2 |
| **P1** | 批量补规则（L1/L2） | 按第六章映射表逐条落地，命名类先做（收益最高、误报最低） | 覆盖 4.1 / 4.2 / 4.5 / 4.7 / 5 章主体 |
| **P2** | 跨文件与工程视角 | `ProjectRule` trait + 两阶段调度 + 摘要模式；激活 `applies_to` 与非 Java 文件通道 | 覆盖 6 章、13.1 同名 Bean、`pom.xml`/`yml` 配置类条款 |
| **P3** | 精准度与工程化 | `guard_core::dataflow` 沉淀；`// noqa` 抑制；规则与【强制】/【推荐】档位对齐；`docs/` 补规则清单与映射表 | 降低误报、可被审计追溯 |
| **并行（非 java-guard）** | 覆盖 L5 | CI 开 `javac -Xlint:overrides,deprecation`；接入 SpotBugs/ErrorProne/ArchUnit；SCA 扫依赖；启动脚本巡检；APM 看 GC 指标 | 覆盖 7 / 8 / 9.2 / 13.5 |

---

## 八、规则编号规划建议

现有 `J001`–`J017` **保持不动**（避免迁移历史报告与 baseline）。后续按主题分段续编，便于用 `--enable J4xx` 按主题批量开关：

| 段 | 主题 | 段 | 主题 |
|---|---|---|---|
| J1xx | 命名约定（4.1）+ 常量与魔法值（4.2） | J6xx | 异常处理（5）+ 日志（4.10） |
| J2xx | 代码格式与排版（4.3）+ 注释（4.8） | J7xx | 工程结构与分层（6）+ Spring Boot（13） |
| J3xx | OOP 与类型使用（4.4） | J8xx | JVM 参数（7）+ 开源软件（8）+ 数据库（9） |
| J4xx | 集合处理（4.5） | J9xx | 国际化（10）+ 日期（11）+ 状态码（12）+ 正则（4.11） |
| J5xx | 并发与线程安全（4.6）+ 控制语句（4.7） | | |

每条规则元数据建议补充两个字段，便于生成合规报告：

```toml
[[rules]]
id = "J201"
name = "max_line_length"
group = "format"
description = "单行字符数不超过 120（《Java编码实施策略》4.3【强制】）"
script_path = "rules/rhai/J201_max_line_length.rhai"
severity = "major"
enabled = true
# 建议新增：
# standard_ref = "JAVA-CODING-STANDARD §4.3"   # 规范条款锚点，供审计追溯
# level        = "mandatory"              # mandatory | recommended | reference
```

---

## 附录 A：本文引用到的源码位置

| 结论 | 位置 |
|---|---|
| 规则粒度 = 单文件 | `crates/guard-core/src/rule.rs:162-187`（`Rule::check_unit(&U)`） |
| YAML 支持的 6 种 PatternKind | `crates/rule-yaml/src/rule.rs:83-97` |
| YAML 字段白名单 + 加载期校验 | `crates/rule-yaml/src/rule.rs:115-150` |
| YAML 匹配语义（精确/glob/正则/any_of） | `crates/rule-yaml/src/matcher.rs:653-659` |
| YAML 递归下发（改造为上下文谓词的着力点） | `crates/rule-yaml/src/matcher.rs:119-286` |
| Rhai 注入 `ast` + `config` | `crates/rule-rhai/src/engine.rs:81-89` |
| Rhai 引擎上限 | `crates/rule-rhai/src/engine.rs:52-58` |
| `source_lines` 模型存在但永不填充 | `crates/java-ast/src/ast.rs:15` + `AstSerializer.java:26-57` |
| 源码已解码但未回填 | `src/main.rs:466-487` |
| 注解参数被丢弃 | `AstSerializer.java:556-566` |
| `applies_to` 死配置 | `src/main.rs:1273-1274, 1286`（全仓无消费） |
| 扫描器硬编码只收 `.java` | `src/scanner.rs:47` |
| AST 缓存 | CLI `--no-cache` / `.java-guard-cache/` |
| Java 插件未启用 | `crates/rule-plugin/src/lib.rs`（`PluginLoader::load()` → `Ok(vec![])`） |
| 已有数据流能力（常量传播） | `src/rules/j009_infinite_loop.rs`（2250 行） |

## 附录 B：可直接用现有引擎实现的条款（无需任何扩展）

先做这批可以快速验证价值：

| 条款 | 实现 | 载体 |
|---|---|---|
| 4.1 `String args[]` 应写 `String[] args` | `FieldDeclaration`/`VariableDeclarationStmt` 的 `var_type` 含 `[]` 位置判定 | Rhai |
| 4.1 POJO 布尔禁 `is` 前缀 | `field_type` ∈ {`boolean`,`Boolean`} + `name` 以 `is` 开头 | **YAML**（现成） |
| 4.1 类名/方法名/常量名 | 已有 J004/J005/J007 | YAML |
| 4.1 异常类 `Exception` 结尾、抽象类 `Abstract`/`Base` 开头、测试类 `Test` 结尾 | `ClassDeclaration.name` 正则 + `modifiers` | **YAML**（现成） |
| 4.2 `long` 必须大写 `L` | `LiteralExpr.value` 尾字符为 `l` | Rhai |
| 4.2 魔法值直查 | 字面量直接作为实参 | Rhai |
| 4.4 禁 `new BigDecimal(double)` | `ObjectCreationExpr.class_name == "BigDecimal"` + 实参非字符串 | Rhai |
| 4.4 禁字段注入推荐构造器注入 | `@Autowired` 出现在 `FieldDeclaration` 上 | **YAML**（现成） |
| 4.4 禁止构造器含业务逻辑 | `ConstructorDeclaration.body.statements` 计数 | Rhai |
| 4.4 POJO 必须 `toString` | 类名以 `DO/DTO/VO/BO` 结尾且成员无 `toString` | Rhai |
| 4.4 循环内用 `StringBuilder` | `ForStmt/ForEachStmt` 体内字符串 `+` | Rhai |
| 4.6 禁 `Executors` 建线程池 | `MethodCallExpr.callee=="Executors"` | **YAML**（现成） |
| 4.6 `SimpleDateFormat` 禁 `static` | `FieldDeclaration.field_type` 含 `SimpleDateFormat` + `modifier: static` | **YAML**（现成） |
| 4.6 禁直接 `new Thread(...)` | `ObjectCreationExpr.class_name=="Thread"` | Rhai |
| 4.6 `lock()` 必须在 `try` 之外 | `TryStmt.try_body` 首条为 `lock()` | Rhai |
| 4.7 `switch` 必须有 `default` | `SwitchStmt.cases` 中存在 `label == null` | Rhai |
| 4.7 `switch` case 必须 break/return | `cases[].statements` 末条类型判定 | Rhai |
| 4.7 禁条件中赋值 | `IfStmt/WhileStmt.condition` 内出现 `AssignExpr` | Rhai |
| 4.7 禁取反逻辑 | `UnaryExpr.op == "!"` | Rhai |
| 4.7 `if/else` 必须大括号 | `then_stmt` 非 `BlockStmt` | Rhai |
| 5 章 finally 中 `return` | `TryStmt.finally_body` 含 `ReturnStmt` | Rhai |
| 5 章 优先 try-with-resources | `TryStmt.resources` 为空且 try 体内有流对象创建 | Rhai（启发式，建议 `info`） |
| 5 章 禁抛 `RuntimeException`/`Exception`/`Throwable` | `ThrowStmt.expr` 的 `ObjectCreationExpr.class_name` | Rhai |
| 4.5 foreach 内 `remove`/`add` | `ForEachStmt` 体内方法调用 | Rhai |
| 4.5 `Arrays.asList` 结果增删 | `MethodCallExpr.callee=="Arrays"` 的返回值被 `.add/.remove` 链式调用 | Rhai |
| 4.10 日志必须占位符 | `log.xxx("a" + b)` 参数为 `BinaryExpr(op="+")` | Rhai |
| 4.10 低级别日志需 `isXxxEnabled` | `log.debug(...)` 所在块无 `isDebugEnabled()` | Rhai |
| 4.10 循环内打印日志 | `ForStmt/ForEachStmt/WhileStmt` 体内 `MethodCallExpr.callee` 含 `log` | Rhai |
| 4.11 正则预编译 | `Pattern.compile` 出现在方法体内而非 `static final` 字段 | Rhai |
| 13.1 `@ComponentScan` / 13.2 `@Autowired` 字段注入 / 13.3 `@Valid` 缺失 | 注解存在性判定 | **YAML**（现成，参数类需 P0-2） |
