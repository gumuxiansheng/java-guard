//! 规则包（Rule Pack）多包加载、合并与项目覆盖。
//!
//! 规则与引擎分开发版后，一个项目往往需要同时用多个来源的规则：官方默认包、
//! 业务线/项目组自建包、以及项目本地规则。本模块负责：
//!
//! - **定位**规则包（`[rule_packs].packs` 的 `path`，或 `search_paths/<name>`）；
//! - **校验**包清单的版本契约（`api_version` / `engine`，见 [`guard_core::RulePackMeta`]）；
//! - **命名空间**限定 id（`<namespace>:<id>`），使多个包可各自拥有同名短 id；
//! - **分层合并**（低 → 高）：包层（按声明顺序，靠后覆盖靠前）< 项目本地规则
//!   < `[[rule_packs.overrides]]`。
//!
//! 未声明 `[rule_packs]` 且本地 `[pack]` 无 `namespace` 时，合并结果与引入本模块前
//! 逐条一致（向后兼容）。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use guard_core::RulePackMeta;
use serde::Deserialize;

use crate::{load_rules_file_raw, RuleEntry};

/// 规则包清单文件名（位于包根）。
pub const PACK_MANIFEST_FILE: &str = "rules-pack.toml";

/// 追加规则包搜索路径的环境变量（`;` 分隔），便于 CI 注入共享包。
pub const PACK_PATH_ENV: &str = "JAVAGUARD_RULE_PATH";

/// `[rule_packs]` 配置段。
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct RulePacksConfig {
    /// 包搜索路径（相对配置文件目录或绝对路径；按顺序查找同名包）。
    pub search_paths: Vec<String>,
    /// 生效的包（按声明顺序决定优先级，后者覆盖前者）。
    pub packs: Vec<PackRef>,
    /// 项目级覆盖（最高优先级，只改 enabled / severity / params）。
    pub overrides: Vec<RuleOverride>,
    /// 是否允许远程 source。本期仅支持本地 `path`，置 true 时报错。
    pub allow_remote: bool,
}

/// `[rule_packs].packs` 的一项：引用一个规则包。
#[derive(Debug, Clone, Deserialize)]
pub struct PackRef {
    /// 包名（必须与包清单 `[pack].name` 一致）。
    pub name: String,
    /// 要求的具体版本（可选；实际版本不一致时报错）。
    #[serde(default)]
    pub version: Option<String>,
    /// 直接指向包根（跳过 `search_paths` 查找）。
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// `[[rule_packs.overrides]]` 的一项：按 id 覆盖规则属性。
///
/// 只允许改 `enabled` / `severity` / `params`，不允许改 `script_path`
/// （要换脚本请用项目本地规则）。
#[derive(Debug, Clone, Deserialize)]
pub struct RuleOverride {
    /// 规则 id：规范 id（`ns:J001`）或裸 id（`J001`）均可。
    pub id: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub params: Option<toml::Value>,
}

/// 成功解析并生效的一个规则包（供 `rules` 子命令展示来源）。
#[derive(Debug, Clone)]
pub struct ResolvedPack {
    pub name: String,
    pub version: String,
    pub namespace: Option<String>,
    pub api_version: u32,
    pub root: PathBuf,
    pub rule_count: usize,
}

/// 多包解析与合并的结果。
#[derive(Debug, Default)]
pub struct MergedRules {
    /// 合并后的规则条目（包内脚本路径已解析为绝对路径）。
    pub rules: Vec<RuleEntry>,
    /// 每条规则的来源（与 `rules` 一一对应）：`pack '<name>'` 或 `project rules`。
    pub sources: Vec<String>,
    /// 生效的规则包（按声明顺序）。
    pub packs: Vec<ResolvedPack>,
    /// 非致命提示（如 id 被后层覆盖）。
    pub notes: Vec<String>,
    /// 非致命告警（如项目本地规则文件加载失败）。
    pub warnings: Vec<String>,
}

/// 解析 `[rule_packs].packs`、合并各层规则并应用 overrides。
///
/// 合并优先级（低 → 高）：`packs`（按声明顺序）< 项目本地规则
/// （`local_rules_file`）< `[[rule_packs.overrides]]`。
///
/// 包层的一切错误（找不到包、清单非法、版本契约不满足、脚本逃逸包根等）
/// 均为**硬错误**；项目本地规则文件加载失败只产生 warning（保持既有回退行为）。
pub fn resolve_and_merge(
    cfg: &RulePacksConfig,
    local_rules_file: &Path,
    config_dir: &Path,
) -> anyhow::Result<MergedRules> {
    if cfg.allow_remote {
        return Err(anyhow::anyhow!(
            "[rule_packs].allow_remote = true is not supported yet; remote rule packs are \
             planned for a later milestone. Use local `path` entries instead."
        ));
    }

    let mut merged = MergedRules::default();
    let mut layer = Layered::default();
    let search_paths = collect_search_paths(cfg, config_dir);

    // 1) 包层（低优先级，按声明顺序：靠后覆盖靠前）
    for pref in &cfg.packs {
        let root = locate_pack(pref, &search_paths, config_dir)?;
        let manifest_path = root.join(PACK_MANIFEST_FILE);
        let content = fs::read_to_string(&manifest_path).map_err(|e| {
            anyhow::anyhow!("failed to read pack manifest '{}': {e}", manifest_path.display())
        })?;
        let manifest: crate::RulesFile = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse pack manifest '{}': {e}",
                manifest_path.display()
            )
        })?;

        let meta: RulePackMeta = manifest.pack.ok_or_else(|| {
            anyhow::anyhow!(
                "pack manifest '{}' must declare a [pack] section",
                manifest_path.display()
            )
        })?;
        meta.validate().map_err(anyhow::Error::msg)?;
        let name = meta.name.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "pack manifest '{}' must declare [pack].name",
                manifest_path.display()
            )
        })?;
        // 引用名与清单声明必须一致，否则锁文件 / `rules` 列表里的名字会与实际包不符。
        if name != pref.name {
            return Err(anyhow::anyhow!(
                "rule pack reference '{}' points at a pack whose manifest declares name '{}' \
                 ({}). Use name = \"{}\", or fix [pack].name.",
                pref.name,
                name,
                manifest_path.display(),
                name
            ));
        }
        let version = meta.version.clone().ok_or_else(|| {
            anyhow::anyhow!("rule pack '{name}' must declare [pack].version")
        })?;
        if let Some(want) = &pref.version {
            if want != &version {
                return Err(anyhow::anyhow!(
                    "rule pack '{name}' resolved to version {version}, but version {want} was \
                     required. Update [rule_packs].packs to {version}, or point `path` at the \
                     intended pack."
                ));
            }
        }
        if let Some(e) = meta.engine_compat_error(env!("CARGO_PKG_VERSION")) {
            return Err(anyhow::anyhow!("{e}"));
        }

        let namespace = meta.namespace.clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut rule_count = 0usize;
        for mut rule in manifest.rules {
            if !seen.insert(rule.id.clone()) {
                return Err(anyhow::anyhow!(
                    "rule pack '{name}' declares duplicate rule id '{}'",
                    rule.id
                ));
            }
            qualify_rule_id(&mut rule, namespace.as_deref(), &format!("pack '{name}'"))?;
            rule.script_path = resolve_pack_script(&root, &rule.script_path, &name, &rule.id)?;
            layer.insert(rule, &format!("pack '{name}'"), &mut merged.notes);
            rule_count += 1;
        }

        merged.packs.push(ResolvedPack {
            name,
            version,
            namespace,
            api_version: meta.declared_api_version(),
            root,
            rule_count,
        });
    }

    // 2) 项目本地层（中优先级）：rules_file / 同级 javaguard.rules.toml
    let local = match load_rules_file_raw(local_rules_file.to_str().unwrap_or("")) {
        Ok(f) => f,
        Err(e) => {
            // 保持既有行为：本地规则文件出错只告警，由调用方回退到内置规则。
            merged.warnings.push(format!("{e}"));
            None
        }
    };
    let local_namespace = local
        .as_ref()
        .and_then(|f| f.pack.as_ref())
        .and_then(|p| p.namespace.clone());
    if let Some(file) = local {
        for mut rule in file.rules {
            // 本地规则的 script_path 保持相对，交由 load_rule_from_entry 按规则文件目录解析。
            qualify_rule_id(&mut rule, local_namespace.as_deref(), "project rules")?;
            layer.insert(rule, "project rules", &mut merged.notes);
        }
    }

    // 3) overrides（最高优先级）
    apply_overrides(&mut layer.rules, &cfg.overrides)?;

    merged.rules = layer.rules;
    merged.sources = layer.sources;
    Ok(merged)
}

/// 把规则 id 规范化为 canonical id（`<namespace>:<id>`）。
///
/// `namespace` 为 `None` 时保持裸 id。规则 id 本身不允许包含 `:`
/// （否则无法与 namespace 前缀区分）。
fn qualify_rule_id(
    rule: &mut RuleEntry,
    namespace: Option<&str>,
    source: &str,
) -> anyhow::Result<()> {
    if rule.id.contains(':') {
        return Err(anyhow::anyhow!(
            "rule id '{}' from {source} must not contain ':' (it is reserved for the \
             namespace prefix, e.g. \"spring:J001\")",
            rule.id
        ));
    }
    if let Some(ns) = namespace {
        rule.id = format!("{ns}:{}", rule.id);
    }
    Ok(())
}

/// 规则 id 的裸部分（去掉 `<namespace>:` 前缀）。
pub fn short_id(canonical_id: &str) -> &str {
    canonical_id.rsplit(':').next().unwrap_or(canonical_id)
}

/// 过滤/覆盖的 id 匹配：规范 id 精确匹配，或裸 id 匹配任一同名规则。
///
/// 使 `--enable J001` / `[overrides] id = "J001"` 在规则来自带 namespace 的包时
/// 依然生效（跨包聚合）。
pub fn id_matches(pattern: &str, canonical_id: &str) -> bool {
    pattern == canonical_id || pattern == short_id(canonical_id)
}

/// 按优先级分层合并：同 canonical id 时**后者覆盖前者**（原地替换，保持声明顺序）。
///
/// `sources` 与 `rules` 平行，记录每条规则的来源标签（供 `rules` 列表展示）。
#[derive(Default)]
struct Layered {
    rules: Vec<RuleEntry>,
    sources: Vec<String>,
}

impl Layered {
    fn insert(&mut self, rule: RuleEntry, source: &str, notes: &mut Vec<String>) {
        match self.rules.iter().position(|r| r.id == rule.id) {
            Some(pos) => {
                notes.push(format!(
                    "rule id '{}' redefined by {source}; the earlier definition is replaced",
                    rule.id
                ));
                self.rules[pos] = rule;
                self.sources[pos] = source.to_string();
            }
            None => {
                self.rules.push(rule);
                self.sources.push(source.to_string());
            }
        }
    }
}

/// 搜索路径 = `[rule_packs].search_paths` + 环境变量 `JAVAGUARD_RULE_PATH`（`;` 分隔）。
/// 相对路径基于配置文件所在目录解析。
fn collect_search_paths(cfg: &RulePacksConfig, config_dir: &Path) -> Vec<PathBuf> {
    let mut raw: Vec<String> = cfg.search_paths.clone();
    if let Ok(env) = std::env::var(PACK_PATH_ENV) {
        raw.extend(env.split(';').map(str::to_string));
    }
    raw.into_iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let path = Path::new(&p);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                config_dir.join(path)
            }
        })
        .collect()
}

/// 定位包根：`path` 优先；否则在搜索路径下查找 `<search_path>/<name>/rules-pack.toml`。
fn locate_pack(
    pref: &PackRef,
    search_paths: &[PathBuf],
    config_dir: &Path,
) -> anyhow::Result<PathBuf> {
    if let Some(p) = &pref.path {
        let root = if p.is_absolute() {
            p.clone()
        } else {
            config_dir.join(p)
        };
        if !root.join(PACK_MANIFEST_FILE).is_file() {
            return Err(anyhow::anyhow!(
                "rule pack '{}' at '{}' has no {PACK_MANIFEST_FILE}",
                pref.name,
                root.display()
            ));
        }
        return Ok(root);
    }

    for dir in search_paths {
        let nested = dir.join(&pref.name);
        if nested.join(PACK_MANIFEST_FILE).is_file() {
            return Ok(nested);
        }
    }

    Err(anyhow::anyhow!(
        "rule pack '{}' not found. Searched: [{}].\n\
         hint: add its parent directory to [rule_packs].search_paths (packs live at \
         <search_path>/<name>/{PACK_MANIFEST_FILE}), or reference it directly with \
         {{ name = \"{}\", path = \"...\" }}.",
        pref.name,
        search_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        pref.name
    ))
}

/// 解析包内规则脚本路径（相对包根），返回可直接使用的绝对路径。
///
/// `builtin:` 前缀的条目指向引擎内置规则，不属于包内文件，原样返回。
fn resolve_pack_script(
    root: &Path,
    script_path: &str,
    pack: &str,
    rule_id: &str,
) -> anyhow::Result<String> {
    if script_path.starts_with("builtin:") {
        return Ok(script_path.to_string());
    }
    let rel = Path::new(script_path);
    if rel.is_absolute() {
        return Err(anyhow::anyhow!(
            "rule pack '{pack}', rule '{rule_id}': script path must be relative to the pack root, \
             got '{}'",
            rel.display()
        ));
    }
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined = root.join(rel);
    let canon = joined.canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "rule pack '{pack}', rule '{rule_id}': script not found: '{}' ({e})",
            joined.display()
        )
    })?;
    if !canon.starts_with(&canon_root) {
        return Err(anyhow::anyhow!(
            "rule pack '{pack}', rule '{rule_id}': script path '{}' escapes the pack root '{}'",
            rel.display(),
            root.display()
        ));
    }
    Ok(canon.to_string_lossy().into_owned())
}

/// 应用 `[[rule_packs.overrides]]`：只允许改 `enabled` / `severity` / `params`。
///
/// 裸 id 命中多条时按合并顺序取**最后一条**（即优先级最高者）。
/// 未命中任何规则 → 报错并给出可用 id 提示（防 typo）。
fn apply_overrides(rules: &mut [RuleEntry], overrides: &[RuleOverride]) -> anyhow::Result<()> {
    for ov in overrides {
        let Some(idx) = rules
            .iter()
            .rposition(|r| id_matches(&ov.id, &r.id))
        else {
            let want = short_id(&ov.id).to_lowercase();
            let mut similar: Vec<&str> = rules
                .iter()
                .map(|r| r.id.as_str())
                .filter(|id| id.to_lowercase().contains(&want))
                .collect();
            if similar.is_empty() {
                similar = rules.iter().map(|r| r.id.as_str()).take(10).collect();
            }
            return Err(anyhow::anyhow!(
                "[rule_packs.overrides] references unknown rule id '{}'. Known ids: {}",
                ov.id,
                similar.join(", ")
            ));
        };

        if let Some(enabled) = ov.enabled {
            rules[idx].enabled = enabled;
        }
        if let Some(severity) = &ov.severity {
            rules[idx].severity = severity.clone();
        }
        if let Some(params) = &ov.params {
            rules[idx].params = Some(params.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个干净的临时项目目录。
    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("javaguard_rulepack_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 在 `root` 下写入一个规则包（清单 + 脚本）。
    fn write_pack(root: &Path, manifest: &str, scripts: &[&str]) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join(PACK_MANIFEST_FILE), manifest).unwrap();
        for rel in scripts {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "id: X\n").unwrap();
        }
    }

    fn pack_ref(name: &str, path: Option<&Path>) -> PackRef {
        PackRef {
            name: name.to_string(),
            version: None,
            path: path.map(Path::to_path_buf),
        }
    }

    fn script_name(entry: &RuleEntry) -> String {
        Path::new(&entry.script_path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    // ===== 向后兼容 =====

    #[test]
    fn no_packs_returns_local_rules_verbatim() {
        let dir = temp_project("local_only");
        let rules_file = dir.join("javaguard.rules.toml");
        std::fs::write(
            &rules_file,
            r#"
[[rules]]
id = "J001"
name = "no_system_out"
script_path = "rules/J001.yml"
severity = "minor"
"#,
        )
        .unwrap();

        let merged =
            resolve_and_merge(&RulePacksConfig::default(), &rules_file, &dir).unwrap();
        assert_eq!(merged.rules.len(), 1);
        // 无 namespace → id 保持裸 id；script_path 保持相对（交由调用方解析）
        assert_eq!(merged.rules[0].id, "J001");
        assert_eq!(merged.rules[0].script_path, "rules/J001.yml");
        assert!(merged.packs.is_empty());
        assert!(merged.notes.is_empty());
        assert!(merged.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_pack_namespace_qualifies_local_ids() {
        let dir = temp_project("local_ns");
        let rules_file = dir.join("javaguard.rules.toml");
        std::fs::write(
            &rules_file,
            r#"
[pack]
namespace = "myteam"

[[rules]]
id = "J001"
name = "a"
script_path = "rules/J001.yml"
"#,
        )
        .unwrap();

        let merged =
            resolve_and_merge(&RulePacksConfig::default(), &rules_file, &dir).unwrap();
        assert_eq!(merged.rules[0].id, "myteam:J001");
        // 本地规则的脚本路径仍保持相对
        assert_eq!(merged.rules[0].script_path, "rules/J001.yml");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== 包定位与解析 =====

    #[test]
    fn pack_found_via_search_path_and_namespaced() {
        let dir = temp_project("search_path");
        let packs = dir.join("vendor/packs");
        write_pack(
            &packs.join("spring-rules"),
            r#"
[pack]
name = "spring-rules"
namespace = "spring"
version = "1.0.0"
api_version = 1

[[rules]]
id = "J701"
name = "no_componentscan_basepackages"
description = "禁止 ComponentScan basePackages"
script_path = "rules/J701.yml"
severity = "major"
enabled = true
"#,
            &["rules/J701.yml"],
        );

        let cfg = RulePacksConfig {
            search_paths: vec![packs.to_string_lossy().into_owned()],
            packs: vec![pack_ref("spring-rules", None)],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();

        assert_eq!(merged.rules.len(), 1);
        assert_eq!(merged.rules[0].id, "spring:J701");
        assert!(Path::new(&merged.rules[0].script_path).is_absolute());
        assert_eq!(script_name(&merged.rules[0]), "J701.yml");

        assert_eq!(merged.packs.len(), 1);
        assert_eq!(merged.packs[0].name, "spring-rules");
        assert_eq!(merged.packs[0].version, "1.0.0");
        assert_eq!(merged.packs[0].api_version, 1);
        assert_eq!(merged.packs[0].rule_count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_found_via_explicit_path() {
        let dir = temp_project("explicit_path");
        let root = dir.join("my-rules");
        write_pack(
            &root,
            r#"
[pack]
name = "my-rules"
version = "0.1.0"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );

        let cfg = RulePacksConfig {
            packs: vec![pack_ref("my-rules", Some(&root))],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();
        // 未设 namespace → 裸 id
        assert_eq!(merged.rules[0].id, "R001");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_not_found_lists_search_paths() {
        let dir = temp_project("not_found");
        let cfg = RulePacksConfig {
            search_paths: vec!["vendor/packs".into()],
            packs: vec![pack_ref("missing-pack", None)],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("missing-pack"), "msg: {msg}");
        assert!(msg.contains("search_paths"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_without_pack_section_errors() {
        let dir = temp_project("no_pack_section");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("[pack]"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_name_mismatch_errors() {
        let dir = temp_project("name_mismatch");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "actual-name"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("declared-name", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("actual-name"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_version_mismatch_errors() {
        let dir = temp_project("version_mismatch");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "2.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![PackRef {
                name: "p".into(),
                version: Some("1.0.0".into()),
                path: Some(root),
            }],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("1.0.0") && msg.contains("2.0.0"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_without_version_errors() {
        let dir = temp_project("no_version");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("version"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_future_api_version_errors() {
        let dir = temp_project("pack_api");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 999

[[rules]]
id = "R001"
name = "r001"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("rule API version"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_duplicate_rule_id_errors() {
        let dir = temp_project("pack_dup");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "a"
script_path = "J001.yml"

[[rules]]
id = "R001"
name = "b"
script_path = "J002.yml"
"#,
            &["J001.yml", "J002.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("duplicate rule id"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_script_missing_errors() {
        let dir = temp_project("script_missing");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "rules/nope.yml"
"#,
            &[],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("script not found"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_script_escaping_root_errors() {
        let dir = temp_project("script_escape");
        // 包根之外放一个真实文件，使 canonicalize 成功但逃逸检测触发
        std::fs::write(dir.join("outside.yml"), "id: X\n").unwrap();
        let root = dir.join("packs/p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "r001"
script_path = "../../outside.yml"
"#,
            &[],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("escapes the pack root"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_builtin_script_is_passed_through() {
        let dir = temp_project("builtin");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 1

[[rules]]
id = "J008"
name = "empty_catch"
script_path = "builtin:j008_empty_catch"
"#,
            &[],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();
        assert_eq!(merged.rules[0].script_path, "builtin:j008_empty_catch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn allow_remote_is_rejected() {
        let dir = temp_project("allow_remote");
        let cfg = RulePacksConfig {
            allow_remote: true,
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap_err();
        assert!(format!("{err}").contains("allow_remote"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== 合并优先级与覆盖 =====

    #[test]
    fn local_rules_override_pack_rules() {
        let dir = temp_project("override_local");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
version = "1.0.0"
api_version = 1

[[rules]]
id = "J001"
name = "from_pack"
script_path = "J001.yml"
severity = "minor"
enabled = true
"#,
            &["J001.yml"],
        );
        let rules_file = dir.join("javaguard.rules.toml");
        std::fs::write(
            &rules_file,
            r#"
[[rules]]
id = "J001"
name = "from_project"
script_path = "rules/J001.yml"
severity = "major"
"#,
        )
        .unwrap();

        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &rules_file, &dir).unwrap();
        assert_eq!(merged.rules.len(), 1);
        assert_eq!(merged.rules[0].name, "from_project");
        assert_eq!(merged.rules[0].severity, "major");
        assert_eq!(merged.notes.len(), 1);
        assert!(merged.notes[0].contains("J001"), "{}", merged.notes[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn later_pack_overrides_earlier_pack() {
        let dir = temp_project("override_pack");
        write_pack(
            &dir.join("a"),
            r#"
[pack]
name = "a"
version = "1.0.0"
api_version = 1

[[rules]]
id = "J001"
name = "from_a"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );
        write_pack(
            &dir.join("b"),
            r#"
[pack]
name = "b"
version = "1.0.0"
api_version = 1

[[rules]]
id = "J001"
name = "from_b"
script_path = "J001.yml"
"#,
            &["J001.yml"],
        );

        let cfg = RulePacksConfig {
            packs: vec![
                pack_ref("a", Some(&dir.join("a"))),
                pack_ref("b", Some(&dir.join("b"))),
            ],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();
        assert_eq!(merged.rules.len(), 1);
        assert_eq!(merged.rules[0].name, "from_b");
        assert_eq!(merged.packs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn namespaces_allow_same_short_id_in_two_packs() {
        let dir = temp_project("two_namespaces");
        write_pack(
            &dir.join("a"),
            r#"
[pack]
name = "a"
namespace = "team-a"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "from_a"
script_path = "R001.yml"
"#,
            &["R001.yml"],
        );
        write_pack(
            &dir.join("b"),
            r#"
[pack]
name = "b"
namespace = "team-b"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "from_b"
script_path = "R001.yml"
"#,
            &["R001.yml"],
        );

        let cfg = RulePacksConfig {
            packs: vec![
                pack_ref("a", Some(&dir.join("a"))),
                pack_ref("b", Some(&dir.join("b"))),
            ],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();
        assert_eq!(merged.rules.len(), 2);
        assert_eq!(merged.rules[0].id, "team-a:R001");
        assert_eq!(merged.rules[1].id, "team-b:R001");
        // 无冲突 → 无覆盖提示
        assert!(merged.notes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overrides_change_enabled_severity_params() {
        let dir = temp_project("overrides");
        let rules_file = dir.join("javaguard.rules.toml");
        std::fs::write(
            &rules_file,
            r#"
[[rules]]
id = "J001"
name = "a"
script_path = "rules/J001.yml"
severity = "minor"
enabled = true
"#,
        )
        .unwrap();

        let cfg = RulePacksConfig {
            overrides: vec![RuleOverride {
                id: "J001".into(),
                enabled: Some(false),
                severity: Some("critical".into()),
                params: Some(toml::Value::Integer(7)),
            }],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &rules_file, &dir).unwrap();
        assert!(!merged.rules[0].enabled);
        assert_eq!(merged.rules[0].severity, "critical");
        assert_eq!(merged.rules[0].params, Some(toml::Value::Integer(7)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overrides_match_namespaced_id_via_short_id() {
        let dir = temp_project("overrides_ns");
        let root = dir.join("p");
        write_pack(
            &root,
            r#"
[pack]
name = "p"
namespace = "team-a"
version = "1.0.0"
api_version = 1

[[rules]]
id = "R001"
name = "a"
script_path = "R001.yml"
enabled = true
"#,
            &["R001.yml"],
        );
        let cfg = RulePacksConfig {
            packs: vec![pack_ref("p", Some(&root))],
            overrides: vec![RuleOverride {
                id: "R001".into(),
                enabled: Some(false),
                severity: None,
                params: None,
            }],
            ..Default::default()
        };
        let merged = resolve_and_merge(&cfg, &dir.join("javaguard.rules.toml"), &dir).unwrap();
        assert_eq!(merged.rules[0].id, "team-a:R001");
        assert!(!merged.rules[0].enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overrides_unknown_id_errors() {
        let dir = temp_project("overrides_unknown");
        let rules_file = dir.join("javaguard.rules.toml");
        std::fs::write(
            &rules_file,
            r#"
[[rules]]
id = "J001"
name = "a"
script_path = "rules/J001.yml"
"#,
        )
        .unwrap();

        let cfg = RulePacksConfig {
            overrides: vec![RuleOverride {
                id: "J999".into(),
                enabled: Some(false),
                severity: None,
                params: None,
            }],
            ..Default::default()
        };
        let err = resolve_and_merge(&cfg, &rules_file, &dir).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("J999"), "msg: {msg}");
        assert!(msg.contains("J001"), "msg: {msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== id 匹配语义 =====

    #[test]
    fn id_matches_canonical_and_short() {
        assert!(id_matches("spring:J701", "spring:J701"));
        assert!(id_matches("J701", "spring:J701"));
        assert!(!id_matches("spring:J702", "spring:J701"));
        assert!(!id_matches("J701", "J702"));
        assert!(id_matches("J001", "J001"));
    }

    #[test]
    fn short_id_strips_namespace() {
        assert_eq!(short_id("spring:J701"), "J701");
        assert_eq!(short_id("J701"), "J701");
    }
}