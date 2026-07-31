# JavaGuard M5-M8 交付记录

**日期**：2026-07-31
**提交**：`f9cf837`
**状态**：M5-M8 全部完成，43 tests 通过

## 交付内容

### M5: 增量扫描
- **git_diff.rs** (guard-core)：`get_diff()` 调用 `git diff --unified=0 --diff-filter=d` 获取变更
- `FileDiff` / `LineRange` / `DiffKind` 类型
- `LineFilter`：文件级 + 行级过滤，新增文件全量扫描
- `parse_diff()` 解析 hunk header `@@ -old_start,old_len +new_start,new_len @@`
- CLI `--diff HEAD~1` 参数
- Baseline `--baseline <json>` 只报告新增违规

### M6: 报告格式
- **Console**：彩色输出，按文件分组
- **JSON**：`JsonReport` 含 version/scan_info/violations/stats，带时间戳和耗时
- **SARIF 2.1.0**：完整 `runs[].tool.driver.rules` + `results[].locations`
- **CSV**：标准 CSV 转义（逗号/引号/换行）
- `ScanStats`：按 severity 和 rule 统计
- `report_to()` 统一入口，支持写入文件

### M7: CI Gate
- **gate.rs** (guard-core)：`GateConfig` 配置 max_critical/major/minor/info
- `SeverityCounts` 按级别统计
- `GateResult::Pass/Fail` + exit_code()
- CLI `--gate` + `--gate-config <yaml>` 参数
- 默认阈值：critical=0, major=0, minor=∞, info=∞

### M8: 插件接口
- **rule-plugin crate**：`PluginRule` trait + `PluginLoader`
- `PluginConfig`：jar_path + java_cmd + jvm_args
- JSON-RPC 协议预留：`RpcRequest` / `RpcResponse`
- `call_plugin()` 内部函数（MVP 不暴露）
- `PluginLoader::load()` MVP 返回空列表

## 新增 CLI 参数

```
java-guard scan \
  --diff HEAD~1              # 增量扫描
  --baseline report.json     # baseline 过滤
  --gate                     # CI gate 模式
  --gate-config gate.yml     # gate 阈值配置
  --enable J001,J004         # 启用指定规则
  --disable J008             # 禁用指定规则
  --min-severity major       # 最低严重级别
  -f sarif                   # 报告格式
  -o report.sarif            # 输出文件
```

## 测试

- guard-core: 22 tests（含 git_diff 5 个 + gate 4 个 + reporter 8 个 + rule 5 个）
- java-ast: 2 + 1 (e2e)
- rule-yaml: 8 tests
- rule-rhai: 4 tests
- rule-plugin: 3 tests
- main bin: 3 tests
- **总计 43 tests, 0 failed**

## 当前进度

| 里程碑 | 状态 |
|--------|------|
| M1 解析层 | ✅ |
| M2 规则引擎核心 | ✅ |
| M3 YAML 规则 | ✅ |
| M4 Rhai 规则 | ✅ |
| M5 增量扫描 | ✅ |
| M6 报告格式 | ✅ |
| M7 CI 集成 | ✅ |
| M8 插件接口 | ✅ |

**完成度：100%**（8/8 里程碑全部交付）
