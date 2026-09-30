//! 规则包锁文件 `javaguard.lock`（M3）。
//!
//! 目的：让「同一份配置 + 锁文件」在任意机器 / CI 上得到**完全相同的规则集**，
//! 使检查结论可复现、可审计。
//!
//! - `java-guard rules lock`：按当前解析结果生成 / 更新锁文件。
//! - `java-guard rules verify` / `scan --locked`：校验锁与实际情况是否一致。
//!
//! 校验强度分两档（见 `docs` 中的规则包设计稿）：
//! - 默认（Auto）：只比对 `name` + `version`（零文件读取）；
//! - `--locked`（Strict）：额外比对 **checksum**（读取包内全部文件），
//!   并要求锁文件存在、且不含多余条目。
//!
//! `source` 语法本期只实现 `path:`；`git+` / `registry:` 为预留（后续里程碑）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::rulepack::ResolvedPack;

/// 锁文件名（位于配置文件同目录）。
pub const LOCK_FILE_NAME: &str = "javaguard.lock";

/// 锁文件格式版本。
const LOCK_FORMAT_VERSION: u32 = 1;

/// 锁文件内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    /// 格式版本（未来结构变更时递增）。
    #[serde(default = "default_format_version")]
    pub version: u32,
    /// 被锁定的规则包（`[[pack]]`）。
    #[serde(default, rename = "pack")]
    pub packs: Vec<LockedPack>,
}

/// 单个被锁定的规则包。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPack {
    pub name: String,
    pub version: String,
    /// 来源：`path:<相对配置文件目录的路径>`（本期唯一实现）。
    pub source: String,
    /// `sha256:<hex>`，覆盖包内按路径排序后的全部文件。
    pub checksum: String,
    /// 该包声明的规则 API 版本（冗余记录，便于审计）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<u32>,
}

fn default_format_version() -> u32 {
    LOCK_FORMAT_VERSION
}

/// 锁校验强度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// 不校验（`--no-lock`）。
    Off,
    /// 默认：比对 name + version。
    Auto,
    /// `--locked`：额外比对 checksum，并要求锁完整。
    Strict,
}

/// 锁文件路径（配置文件所在目录）。
pub fn lock_path(config_dir: &Path) -> PathBuf {
    config_dir.join(LOCK_FILE_NAME)
}

/// 按当前解析结果构建锁内容。
pub fn build_lock(packs: &[ResolvedPack], config_dir: &Path) -> anyhow::Result<LockFile> {
    let cfg_dir = canonical_or_self(config_dir);
    let mut locked = Vec::with_capacity(packs.len());
    for pack in packs {
        locked.push(LockedPack {
            name: pack.name.clone(),
            version: pack.version.clone(),
            source: source_of(&pack.root, &cfg_dir),
            checksum: pack_checksum(&pack.root)?,
            api_version: Some(pack.api_version),
        });
    }
    Ok(LockFile {
        version: LOCK_FORMAT_VERSION,
        packs: locked,
    })
}

/// 写出锁文件（带说明头注释），返回写入路径。
pub fn write_lock(lock: &LockFile, config_dir: &Path) -> anyhow::Result<PathBuf> {
    let body = toml::to_string_pretty(lock)
        .map_err(|e| anyhow::anyhow!("failed to serialize {LOCK_FILE_NAME}: {e}"))?;
    let header = format!(
        "# {LOCK_FILE_NAME} — 规则包锁文件（由 `java-guard rules lock` 生成，请勿手工编辑）\n\
         # 提交到版本库以保证规则集可复现。\n\n"
    );
    let path = lock_path(config_dir);
    fs::write(&path, format!("{header}{body}"))
        .map_err(|e| anyhow::anyhow!("failed to write '{}': {e}", path.display()))?;
    Ok(path)
}

/// 读取锁文件；不存在时返回 `Ok(None)`。
pub fn read_lock(config_dir: &Path) -> anyhow::Result<Option<LockFile>> {
    let path = lock_path(config_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", path.display()))?;
    let lock: LockFile = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse '{}': {e}", path.display()))?;
    Ok(Some(lock))
}

/// 校验锁文件与当前解析结果是否一致。
///
/// `Ok(notes)` 为可展示的提示；不一致时返回 `Err`（调用方决定是否中止）。
pub fn verify_lock(
    packs: &[ResolvedPack],
    config_dir: &Path,
    mode: LockMode,
) -> anyhow::Result<Vec<String>> {
    if mode == LockMode::Off {
        return Ok(Vec::new());
    }
    let mut notes = Vec::new();
    let lock = read_lock(config_dir)?;

    let Some(lock) = lock else {
        if mode == LockMode::Strict {
            return Err(anyhow::anyhow!(
                "{LOCK_FILE_NAME} not found; run `java-guard rules lock` and commit it \
                 (CI is expected to run with --locked)."
            ));
        }
        if !packs.is_empty() {
            notes.push(format!(
                "no {LOCK_FILE_NAME}; run `java-guard rules lock` to pin rule pack versions"
            ));
        }
        return Ok(notes);
    };

    let mut problems: Vec<String> = Vec::new();

    for pack in packs {
        match lock.packs.iter().find(|p| p.name == pack.name) {
            None => problems.push(format!(
                "rule pack '{}' ({}) is not recorded in {LOCK_FILE_NAME}",
                pack.name, pack.version
            )),
            Some(locked) => {
                if locked.version != pack.version {
                    problems.push(format!(
                        "rule pack '{}' resolved to version {} but {LOCK_FILE_NAME} pins {}",
                        pack.name, pack.version, locked.version
                    ));
                }
                if mode == LockMode::Strict {
                    let actual = pack_checksum(&pack.root)?;
                    if locked.checksum != actual {
                        problems.push(format!(
                            "rule pack '{}' checksum mismatch (lock {}, actual {}) — the pack \
                             content changed without a version bump",
                            pack.name, locked.checksum, actual
                        ));
                    }
                }
            }
        }
    }

    if mode == LockMode::Strict {
        for locked in &lock.packs {
            if !packs.iter().any(|p| p.name == locked.name) {
                problems.push(format!(
                    "{LOCK_FILE_NAME} pins '{}' but the config no longer resolves it",
                    locked.name
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(notes)
    } else {
        Err(anyhow::anyhow!(
            "{} (run `java-guard rules lock` to update {LOCK_FILE_NAME})",
            problems.join("; ")
        ))
    }
}

/// 计算包内容校验和：`sha256:` + 覆盖包内**按路径排序**的全部文件的哈希。
///
/// 逐文件喂入 `相对路径\0长度\0内容`，路径统一用 `/` 分隔——保证跨平台、
/// 跨遍历顺序结果稳定。
pub fn pack_checksum(root: &Path) -> anyhow::Result<String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (rel, path) in files {
        let bytes = fs::read(&path)
            .map_err(|e| anyhow::anyhow!("failed to read '{}': {e}", path.display()))?;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("sha256:{}", to_hex(&hasher.finalize())))
}

/// 递归收集包内文件（跳过 `.git` 等版本控制目录）。
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> anyhow::Result<()> {
    let entries = fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("failed to read dir '{}': {e}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|e| anyhow::anyhow!("failed to read dir '{}': {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| anyhow::anyhow!("failed to stat '{}': {e}", path.display()))?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == ".svn" || name == ".hg" {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    Ok(())
}

/// 去掉 Windows `canonicalize()` 引入的 `\\?\` 前缀（仅影响展示/落盘文本）。
fn strip_unc(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string()
}

/// `source` 字段：包根相对配置文件目录（不可相对时用绝对路径）。
///
/// 两侧都先规范化，避免 Windows 下 `canonicalize()` 引入的 `\\?\` 前缀导致
/// 前缀匹配失败；输出统一去掉 `\\?\` 并转为 `/`：锁文件要提交进版本库，
/// 机器相关的 UNC 前缀会让它在其它机器上失去意义（也污染 diff）。
fn source_of(root: &Path, canonical_config_dir: &Path) -> String {
    let canon_root = canonical_or_self(root);
    match canon_root.strip_prefix(canonical_config_dir) {
        Ok(rel) => format!("path:{}", rel.to_string_lossy().replace('\\', "/")),
        Err(_) => format!("path:{}", strip_unc(&canon_root).replace('\\', "/")),
    }
}

/// 规范化路径；失败时原样返回。
///
/// 空路径（默认配置路径 `java-guard.toml` 的 parent 为 `""`）视为当前目录，
/// 否则 `canonicalize` 会失败，使 `source` 的相对化退化为绝对路径。
fn canonical_or_self(p: &Path) -> PathBuf {
    let p = if p.as_os_str().is_empty() {
        Path::new(".")
    } else {
        p
    };
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("javaguard_lock_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn resolved_pack(name: &str, version: &str, root: &Path) -> ResolvedPack {
        ResolvedPack {
            name: name.to_string(),
            version: version.to_string(),
            namespace: None,
            api_version: 1,
            root: root.to_path_buf(),
            rule_count: 1,
        }
    }

    // ===== checksum =====

    #[test]
    fn checksum_is_order_independent_and_content_sensitive() {
        let dir = tmp("ck_order");
        // 以不同创建顺序写入同一组文件 → 结果必须一致
        fs::write(dir.join("b.yml"), "b").unwrap();
        fs::write(dir.join("a.yml"), "a").unwrap();
        let first = pack_checksum(&dir).unwrap();

        let dir2 = tmp("ck_order2");
        fs::write(dir2.join("a.yml"), "a").unwrap();
        fs::write(dir2.join("b.yml"), "b").unwrap();
        assert_eq!(first, pack_checksum(&dir2).unwrap());

        // 内容变化 → 哈希必须变化
        fs::write(dir2.join("b.yml"), "b2").unwrap();
        assert_ne!(first, pack_checksum(&dir2).unwrap());
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn checksum_includes_nested_files_and_skips_vcs_dirs() {
        let dir = tmp("ck_nested");
        fs::create_dir_all(dir.join("rules")).unwrap();
        fs::write(dir.join("rules/J001.yml"), "x").unwrap();
        let before = pack_checksum(&dir).unwrap();

        // .git 目录内容不参与校验和（避免版本库元数据污染结果）
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        assert_eq!(before, pack_checksum(&dir).unwrap());

        // 嵌套文件内容变化必须被检测到
        fs::write(dir.join("rules/J001.yml"), "y").unwrap();
        assert_ne!(before, pack_checksum(&dir).unwrap());
    }

    // ===== 构建 / 读写 =====

    #[test]
    fn build_write_read_roundtrip() {
        let dir = tmp("roundtrip");
        let pack_root = dir.join("vendor/rules/team-a");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("rules-pack.toml"), "[pack]\nname = \"team-a\"\n").unwrap();

        let packs = vec![resolved_pack("team-a", "1.2.0", &pack_root)];
        let lock = build_lock(&packs, &dir).unwrap();
        assert_eq!(lock.version, LOCK_FORMAT_VERSION);
        assert_eq!(lock.packs.len(), 1);
        assert_eq!(lock.packs[0].name, "team-a");
        assert_eq!(lock.packs[0].version, "1.2.0");
        assert_eq!(lock.packs[0].api_version, Some(1));
        assert!(
            lock.packs[0].source.starts_with("path:"),
            "source: {}",
            lock.packs[0].source
        );
        assert!(lock.packs[0].checksum.starts_with("sha256:"));

        write_lock(&lock, &dir).unwrap();
        let read = read_lock(&dir).unwrap().unwrap();
        assert_eq!(read.packs, lock.packs);
    }

    #[test]
    fn read_lock_missing_returns_none() {
        let dir = tmp("missing");
        assert!(read_lock(&dir).unwrap().is_none());
    }

    // ===== 校验 =====

    #[test]
    fn verify_off_mode_is_noop() {
        let dir = tmp("off_mode");
        let notes = verify_lock(&[], &dir, LockMode::Off).unwrap();
        assert!(notes.is_empty());
    }

    #[test]
    fn verify_auto_without_lock_notes_when_packs_present() {
        let dir = tmp("auto_no_lock");
        let pack_root = dir.join("p");
        fs::create_dir_all(&pack_root).unwrap();
        let packs = vec![resolved_pack("p", "1.0.0", &pack_root)];

        let notes = verify_lock(&packs, &dir, LockMode::Auto).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains(LOCK_FILE_NAME), "{}", notes[0]);

        // 无包时不给噪音提示
        assert!(verify_lock(&[], &dir, LockMode::Auto).unwrap().is_empty());
    }

    #[test]
    fn verify_strict_without_lock_errors() {
        let dir = tmp("strict_no_lock");
        let err = verify_lock(&[], &dir, LockMode::Strict).unwrap_err();
        assert!(format!("{err}").contains(LOCK_FILE_NAME), "err: {err}");
    }

    #[test]
    fn verify_auto_matches_and_detects_version_drift() {
        let dir = tmp("auto_drift");
        let pack_root = dir.join("p");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("rules-pack.toml"), "x").unwrap();

        let packs = vec![resolved_pack("p", "1.0.0", &pack_root)];
        write_lock(&build_lock(&packs, &dir).unwrap(), &dir).unwrap();

        // 一致
        assert!(verify_lock(&packs, &dir, LockMode::Auto).unwrap().is_empty());

        // 版本漂移 → Auto 即报错
        let bumped = vec![resolved_pack("p", "1.1.0", &pack_root)];
        let err = verify_lock(&bumped, &dir, LockMode::Auto).unwrap_err();
        assert!(format!("{err}").contains("1.1.0"), "err: {err}");

        // 缺条目 → 报错
        let extra = vec![
            resolved_pack("p", "1.0.0", &pack_root),
            resolved_pack("q", "1.0.0", &pack_root),
        ];
        let err = verify_lock(&extra, &dir, LockMode::Auto).unwrap_err();
        assert!(format!("{err}").contains("'q'"), "err: {err}");
    }

    #[test]
    fn verify_strict_detects_checksum_change() {
        let dir = tmp("strict_ck");
        let pack_root = dir.join("p");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("rules-pack.toml"), "x").unwrap();

        let packs = vec![resolved_pack("p", "1.0.0", &pack_root)];
        write_lock(&build_lock(&packs, &dir).unwrap(), &dir).unwrap();

        // Auto 不检测内容变化（零文件读取）
        fs::write(pack_root.join("extra.yml"), "new").unwrap();
        assert!(verify_lock(&packs, &dir, LockMode::Auto).unwrap().is_empty());

        // Strict 检测到 checksum 不符
        let err = verify_lock(&packs, &dir, LockMode::Strict).unwrap_err();
        assert!(format!("{err}").contains("checksum mismatch"), "err: {err}");
    }

    #[test]
    fn verify_strict_rejects_stale_lock_entries() {
        let dir = tmp("strict_stale");
        let pack_root = dir.join("p");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("rules-pack.toml"), "x").unwrap();

        let packs = vec![resolved_pack("p", "1.0.0", &pack_root)];
        write_lock(&build_lock(&packs, &dir).unwrap(), &dir).unwrap();

        // 配置不再解析该包 → Strict 报「锁里有、配置里没有」
        let err = verify_lock(&[], &dir, LockMode::Strict).unwrap_err();
        assert!(format!("{err}").contains("no longer resolves"), "err: {err}");
    }

    #[test]
    fn source_is_relative_to_config_dir_when_inside() {
        let dir = tmp("source_rel");
        let pack_root = dir.join("vendor/rules/p");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("m.toml"), "x").unwrap();

        let lock = build_lock(&[resolved_pack("p", "1.0.0", &pack_root)], &dir).unwrap();
        assert_eq!(lock.packs[0].source, "path:vendor/rules/p");
    }

    #[test]
    fn source_relativizes_with_empty_config_dir() {
        // 默认配置路径 `java-guard.toml` 的 parent 是空串，等价于当前目录。
        // 此时 source 仍应相对化，而不是退化成带 `\\?\` 的绝对路径。
        let dir = tmp("source_empty_dir");
        let pack_root = dir.join("vendor/rules/p");
        fs::create_dir_all(&pack_root).unwrap();
        fs::write(pack_root.join("m.toml"), "x").unwrap();

        let cwd = std::env::current_dir().unwrap();
        let rel_root = match pack_root.strip_prefix(&cwd) {
            Ok(r) => r.to_path_buf(),
            // 临时目录不在 cwd 下时跳过（避免平台相关的假失败）
            Err(_) => return,
        };
        let lock = build_lock(&[resolved_pack("p", "1.0.0", &rel_root)], Path::new("")).unwrap();
        assert_eq!(
            lock.packs[0].source,
            "path:vendor/rules/p",
            "source: {}",
            lock.packs[0].source
        );
    }
}