//! 文件扫描器：递归查找 .java 文件。

use std::path::{Path, PathBuf};

/// 扫描结果。
#[derive(Debug)]
pub struct ScanResult {
    /// 找到的 .java 文件列表（绝对路径）。
    pub files: Vec<PathBuf>,
    /// 扫描根目录的规范路径。
    pub root: PathBuf,
}

/// 递归扫描目录下的 .java 文件。
///
/// - `root`：扫描根目录
/// - `exclude`：排除的目录名（如 `target`、`node_modules`）
pub fn scan_java_files(root: &Path, exclude: &[&str]) -> ScanResult {
    let mut files = Vec::new();

    // Windows 上 canonicalize 可能返回 `\\?\` 扩展路径前缀（如 \\?\C:\foo），
    // 与 walkdir 生成的普通路径前缀不一致，会导致后续 strip_prefix 全部失败
    // （增量扫描文件匹配、报告相对路径都依赖它），故统一去掉该前缀。
    let raw_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let raw_str = raw_root.to_string_lossy();
    let canonical_root = PathBuf::from(raw_str.strip_prefix("\\\\?\\").unwrap_or(&raw_str));

    let exclude_set: std::collections::HashSet<String> =
        exclude.iter().map(|s| s.to_string()).collect();

    let walker = walkdir::WalkDir::new(&canonical_root).into_iter();
    for entry in walker.filter_entry(|e| {
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy().into_owned();
            !exclude_set.contains(&name)
        } else {
            true
        }
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("java") {
                files.push(path.to_path_buf());
            }
        }
    }

    files.sort();

    ScanResult {
        files,
        root: canonical_root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_finds_java_files() {
        let tmp = std::env::temp_dir().join("javaguard_scan_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src/main/java/com/example")).unwrap();
        fs::create_dir_all(tmp.join("target")).unwrap();

        fs::write(tmp.join("src/main/java/com/example/Foo.java"), "class Foo {}").unwrap();
        fs::write(tmp.join("src/main/java/com/example/Bar.java"), "class Bar {}").unwrap();
        fs::write(tmp.join("target/Generated.java"), "class Generated {}").unwrap();
        fs::write(tmp.join("README.md"), "# readme").unwrap();

        let result = scan_java_files(&tmp, &["target"]);

        assert_eq!(result.files.len(), 2);
        assert!(result.files.iter().any(|f| f.ends_with("Foo.java")));
        assert!(result.files.iter().any(|f| f.ends_with("Bar.java")));
        assert!(!result.files.iter().any(|f| f.ends_with("Generated.java")));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_root_normalizes_windows_extended_prefix() {
        // Windows canonicalize 会返回 \\?\ 前缀扩展路径，必须去掉，
        // 否则文件相对根路径的 strip_prefix 全部失败（增量扫描匹配、报告路径都会错）。
        let tmp = std::env::temp_dir().join("javaguard_scan_root_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("Foo.java"), "class Foo {}").unwrap();

        let result = scan_java_files(&tmp, &[]);
        assert!(!result.root.to_string_lossy().starts_with("\\\\?\\"));
        assert_eq!(result.files.len(), 1);
        let rel = result.files[0].strip_prefix(&result.root).unwrap();
        assert_eq!(rel.to_string_lossy(), "Foo.java");

        let _ = fs::remove_dir_all(&tmp);
    }
}
