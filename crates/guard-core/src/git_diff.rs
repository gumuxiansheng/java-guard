//! git diff 集成：获取变更文件列表 + 行范围。
//!
//! 支持两种 diff 模式：
//! - `--diff HEAD~1`：与指定 ref 比较
//! - `--diff main...feature`：三个点语法，比较分支差异

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitDiffError {
    #[error("git command failed: {0}")]
    GitCommand(String),
    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),
    #[error("invalid diff spec: {0}")]
    InvalidSpec(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// git diff 变更文件 + 行范围。
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// 文件路径（相对于 git root）。
    pub path: String,
    /// 变更类型。
    pub kind: DiffKind,
    /// 变更行范围列表（1-indexed, inclusive）。
    pub line_ranges: Vec<LineRange>,
    /// 是否为新增文件。
    pub is_new: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// 行范围（inclusive, 1-indexed）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    pub fn contains(&self, line: usize) -> bool {
        line >= self.start && line <= self.end
    }
}

/// 解析 `git diff` 输出，返回变更文件列表。
///
/// 使用 `git diff --unified=0 --diff-filter=d <spec>` 获取变更：
/// - `--unified=0`：不输出上下文行
/// - `--diff-filter=d`：排除纯删除文件（已删除文件无需扫描）
pub fn get_diff(repo_root: &Path, spec: &str) -> Result<Vec<FileDiff>, GitDiffError> {
    // 验证 git 仓库
    if !repo_root.join(".git").exists() {
        // 尝试向上查找
        let _ = find_git_root(repo_root)?;
    }

    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "diff",
            "--unified=0",
            "--diff-filter=d",
            "--no-color",
            spec,
        ])
        .output()
        .map_err(|e| GitDiffError::GitCommand(format!("failed to run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitDiffError::GitCommand(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_diff(&stdout))
}

/// 解析 `git diff` 文本输出。
fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current_file: Option<FileDiff> = None;

    for line in text.lines() {
        if line.starts_with("diff --git") || line.starts_with("diff --cc") {
            // 保存上一个文件
            if let Some(f) = current_file.take() {
                files.push(f);
            }
            // 不解析路径，等 +++ 行
        } else if line.starts_with("--- ") {
            // 旧文件路径：如果还没有 current_file，先创建（Modified 默认）
            if current_file.is_none() {
                let old_path = parse_diff_path(line);
                current_file = Some(FileDiff {
                    path: old_path,
                    kind: DiffKind::Modified,
                    line_ranges: Vec::new(),
                    is_new: false,
                });
            }
        } else if line.starts_with("+++ ") {
            // 新文件路径
            let new_path = parse_diff_path(line);
            match current_file.as_mut() {
                Some(f) => {
                    // 已通过 --- 行创建，更新路径为新路径
                    f.path = new_path;
                }
                None => {
                    // 没有 --- 行（纯新增文件）
                    current_file = Some(FileDiff {
                        path: new_path,
                        kind: DiffKind::Added,
                        line_ranges: Vec::new(),
                        is_new: true,
                    });
                }
            }
        } else if line.starts_with("@@ ") {
            // hunk header: @@ -old_start,old_len +new_start,new_len @@
            if let Some(f) = current_file.as_mut() {
                if let Some(range) = parse_hunk_header(line) {
                    f.line_ranges.push(range);
                }
            }
        } else if line.starts_with("new file mode") {
            if let Some(f) = current_file.as_mut() {
                f.is_new = true;
                f.kind = DiffKind::Added;
            } else {
                // new file mode 在 --- 之前出现，先创建占位
                current_file = Some(FileDiff {
                    path: String::new(),
                    kind: DiffKind::Added,
                    line_ranges: Vec::new(),
                    is_new: true,
                });
            }
        } else if line.starts_with("deleted file mode") {
            // 已被 --diff-filter=d 排除，但以防万一
            if let Some(f) = current_file.as_mut() {
                f.kind = DiffKind::Deleted;
            }
        } else if line.starts_with("rename from") || line.starts_with("rename to") {
            if let Some(f) = current_file.as_mut() {
                f.kind = DiffKind::Renamed;
            }
        }
    }

    if let Some(f) = current_file {
        files.push(f);
    }

    // 过滤 Deleted 文件
    files.retain(|f| f.kind != DiffKind::Deleted);

    files
}

/// 从 `+++ b/path/to/file.java` 中提取路径。
fn parse_diff_path(line: &str) -> String {
    let line = line.trim_start_matches("+++ ");
    // b/path/to/file.java → path/to/file.java
    if let Some(stripped) = line.strip_prefix("b/") {
        stripped.to_string()
    } else {
        line.to_string()
    }
}

/// 解析 `@@ -old_start,old_len +new_start,new_len @@` hunk header。
fn parse_hunk_header(line: &str) -> Option<LineRange> {
    // 格式: @@ -1,5 +10,3 @@ context
    //       @@ -1 +10 @@
    let line = line.strip_prefix("@@ ")?;
    let plus_pos = line.find(" +")?;
    let rest = &line[plus_pos + 2..]; // skip " +"
    let end = rest.find(' ').unwrap_or(rest.len());
    let range_str = &rest[..end];

    if let Some((start_s, len_s)) = range_str.split_once(',') {
        let start: usize = start_s.parse().ok()?;
        let len: usize = len_s.parse().ok().unwrap_or(1);
        if len == 0 {
            return None;
        }
        Some(LineRange {
            start,
            end: start + len.saturating_sub(1),
        })
    } else {
        // 单行: +10
        let start: usize = range_str.parse().ok()?;
        Some(LineRange { start, end: start })
    }
}

/// 查找 git 仓库根目录。
pub fn find_git_root(start: &Path) -> Result<PathBuf, GitDiffError> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(GitDiffError::NotARepo(start.to_path_buf()));
        }
    }
}

/// 行级过滤器：判断某文件的某行是否在变更范围内。
#[derive(Debug, Clone)]
pub struct LineFilter {
    /// 文件路径 → 行范围列表。
    ranges: std::collections::HashMap<String, Vec<LineRange>>,
}

impl LineFilter {
    /// 创建全量过滤器（允许所有行）。
    pub fn all() -> Self {
        Self {
            ranges: std::collections::HashMap::new(),
        }
    }

    /// 从 FileDiff 列表构造过滤器。
    pub fn from_diffs(diffs: &[FileDiff]) -> Self {
        let mut ranges: std::collections::HashMap<String, Vec<LineRange>> = std::collections::HashMap::new();
        for d in diffs {
            ranges
                .entry(d.path.clone())
                .or_default()
                .extend(d.line_ranges.iter().copied());
        }
        Self { ranges }
    }

    /// 判断某文件的某行是否在变更范围内。
    pub fn allows(&self, file: &str, line: usize) -> bool {
        self.allows_range(file, line, None)
    }

    /// 检查某文件中 [line, end_line] 范围是否在 diff 范围内。
    /// 如果 end_line 为 None，退化为单行检查。
    pub fn allows_range(&self, file: &str, line: usize, end_line: Option<usize>) -> bool {
        match self.ranges.get(file) {
            None => true, // 不在 diff 列表中的文件，全量扫描
            Some(ranges) => {
                if ranges.is_empty() {
                    return true;
                }
                let end = end_line.unwrap_or(line);
                ranges.iter().any(|r| r.contains(line) || r.contains(end) || (line <= r.start && end >= r.end))
            }
        }
    }

    /// 获取某文件的变更行范围（空列表表示全量扫描）。
    pub fn get_ranges(&self, file: &str) -> Option<&[LineRange]> {
        self.ranges.get(file).map(|v| v.as_slice())
    }

    /// 是否为增量模式（有 diff 范围限制）。
    pub fn is_incremental(&self) -> bool {
        !self.ranges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_diff() {
        let diff = r#"diff --git a/src/Foo.java b/src/Foo.java
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/src/Foo.java
@@ -0,0 +1,5 @@
+public class Foo {
+    public void bar() {
+        System.out.println("hello");
+    }
+}
"#;
        let files = parse_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/Foo.java");
        assert!(files[0].is_new);
        assert_eq!(files[0].line_ranges.len(), 1);
        assert_eq!(files[0].line_ranges[0].start, 1);
        assert_eq!(files[0].line_ranges[0].end, 5);
    }

    #[test]
    fn parse_modified_diff() {
        let diff = r#"diff --git a/src/Bar.java b/src/Bar.java
index 1234567..abcdef0 100644
--- a/src/Bar.java
+++ b/src/Bar.java
@@ -10,3 +10,8 @@
+    System.out.println("new line");
+    System.out.println("new line 2");
@@ -20,2 +25,3 @@
+    // another change
"#;
        let files = parse_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/Bar.java");
        assert!(!files[0].is_new);
        assert_eq!(files[0].line_ranges.len(), 2);
        assert_eq!(files[0].line_ranges[0].start, 10);
        assert_eq!(files[0].line_ranges[0].end, 17);
        assert_eq!(files[0].line_ranges[1].start, 25);
        assert_eq!(files[0].line_ranges[1].end, 27);
    }

    #[test]
    fn parse_single_line_hunk() {
        let diff = "--- a/x.java\n+++ b/x.java\n@@ -5 +5 @@\n+// change\n";
        let files = parse_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].line_ranges[0].start, 5);
        assert_eq!(files[0].line_ranges[0].end, 5);
    }

    #[test]
    fn line_filter_all() {
        let filter = LineFilter::all();
        assert!(filter.allows("any/file.java", 1));
        assert!(filter.allows("any/file.java", 999));
    }

    #[test]
    fn line_filter_incremental() {
        let diffs = vec![FileDiff {
            path: "src/Foo.java".to_string(),
            kind: DiffKind::Modified,
            line_ranges: vec![
                LineRange { start: 10, end: 15 },
                LineRange { start: 20, end: 22 },
            ],
            is_new: false,
        }];
        let filter = LineFilter::from_diffs(&diffs);

        assert!(filter.is_incremental());
        assert!(filter.allows("src/Foo.java", 10));
        assert!(filter.allows("src/Foo.java", 15));
        assert!(!filter.allows("src/Foo.java", 16));
        assert!(filter.allows("src/Foo.java", 20));
        assert!(!filter.allows("src/Foo.java", 18));
        // 其他文件不受限
        assert!(filter.allows("src/Other.java", 1));
    }

    #[test]
    fn line_filter_new_file() {
        let diffs = vec![FileDiff {
            path: "src/New.java".to_string(),
            kind: DiffKind::Added,
            line_ranges: vec![],
            is_new: true,
        }];
        let filter = LineFilter::from_diffs(&diffs);
        // 新增文件无行范围，全量扫描
        assert!(filter.allows("src/New.java", 1));
        assert!(filter.allows("src/New.java", 999));
    }
}
