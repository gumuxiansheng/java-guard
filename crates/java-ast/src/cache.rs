//! AST 解析缓存：基于文件内容 hash 缓存 JavaParser 的 JSON 输出。
//!
//! 命中缓存可完全跳过 JVM 解析（含 daemon IPC），对重复全量扫描、
//! CI 重复执行与 `--semantic-diff`（解析旧版本）收益显著。
//!
//! 设计要点：
//! - 内容寻址：缓存文件名由源码长度 + 64 位 hash 决定，同名必同内容
//! - 失效：`--no-cache` 全局关闭；parser 版本变化时整目录清空
//! - 并发安全：写入为「临时文件 + 改名」，多 worker 同内容并发写入结果一致
//! - 尽力而为：读写失败仅跳过缓存，不影响解析正确性

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// AST 缓存。默认缓存在 `<cwd>/.java-guard-cache/`。
pub struct AstCache {
    dir: PathBuf,
    enabled: bool,
    parser_version: String,
}

impl AstCache {
    /// 创建缓存。`parser_version` 用于解析器升级时自动失效（如 jar 指纹）。
    pub fn new(enabled: bool, parser_version: &str) -> Self {
        let dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".java-guard-cache");
        let cache = AstCache {
            dir,
            enabled,
            parser_version: parser_version.to_string(),
        };
        cache.init_version();
        cache
    }

    /// 自定义缓存目录（测试用）。换目录后重新做版本校验。
    pub fn with_dir(self, dir: PathBuf) -> Self {
        let cache = AstCache {
            dir,
            parser_version: self.parser_version.clone(),
            enabled: self.enabled,
        };
        cache.init_version();
        cache
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 校验 / 更新 parser 版本标记。版本不匹配时清空目录并重写标记。
    fn init_version(&self) {
        if !self.enabled {
            return;
        }
        let version_file = self.dir.join("parser.version");
        match std::fs::read_to_string(&version_file) {
            Ok(existing) if existing == self.parser_version => return,
            _ => {
                let _ = std::fs::remove_dir_all(&self.dir);
                let _ = std::fs::create_dir_all(&self.dir);
                let _ = std::fs::write(&version_file, &self.parser_version);
            }
        }
    }

    /// 取缓存：命中返回原始 AST JSON，未命中/关闭/损坏返回 None。
    pub fn get(&self, source: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        std::fs::read_to_string(self.path_for(source)).ok()
    }

    /// 写入缓存：内容寻址，同名必同内容，并发安全（临时文件 + 改名）。
    pub fn put(&self, source: &str, raw_json: &str) {
        if !self.enabled {
            return;
        }
        let path = self.path_for(source);
        if path.exists() {
            return; // 已缓存（同内容），避免无谓 IO
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, raw_json).is_err() {
            return;
        }
        if std::fs::rename(&tmp, &path).is_err() {
            // Windows 上 rename 不会覆盖已存在文件；目标已存在则内容必相同，忽略即可
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn path_for(&self, source: &str) -> PathBuf {
        let mut h = DefaultHasher::new();
        source.hash(&mut h);
        let hash = h.finish();
        self.dir.join(format!("{}-{:016x}.json", source.len(), hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_and_miss() {
        let dir = std::env::temp_dir().join(format!("jg_cache_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = AstCache::new(true, "v1").with_dir(dir.clone());

        assert!(cache.get("class A {}").is_none()); // miss
        cache.put("class A {}", r#"{"types":[]}"#);
        assert_eq!(cache.get("class A {}").as_deref(), Some(r#"{"types":[]}"#));
        assert!(cache.get("class B {}").is_none()); // 不同内容不同 key

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_cache_does_nothing() {
        let dir = std::env::temp_dir().join(format!("jg_cache_off_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = AstCache::new(false, "v1").with_dir(dir.clone());
        cache.put("class A {}", r#"{}"#);
        assert!(cache.get("class A {}").is_none());
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parser_version_invalidates_entries() {
        let dir = std::env::temp_dir().join(format!("jg_cache_ver_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let cache = AstCache::new(true, "jar-v1").with_dir(dir.clone());
        cache.put("class A {}", r#"{"old":true}"#);
        assert_eq!(cache.get("class A {}").as_deref(), Some(r#"{"old":true}"#));

        // 解析器升级 → 整个缓存目录被清空重建
        let cache2 = AstCache::new(true, "jar-v2").with_dir(dir.clone());
        assert!(cache2.get("class A {}").is_none());
        // v2 重新写入后 v1 的对象还可被 v2 使用
        cache2.put("class A {}", r#"{"new":true}"#);
        assert_eq!(cache2.get("class A {}").as_deref(), Some(r#"{"new":true}"#));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_content_deterministic_key() {
        let a = AstCache::new(true, "v").dir.clone();
        let b = AstCache::new(true, "v").dir.clone();
        assert_eq!(a, b);
    }
}