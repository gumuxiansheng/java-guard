//! git diff 集成：获取变更文件列表 + 行范围。
//!
//! 支持两种 diff 模式：
//! - `--diff HEAD~1`：与指定 ref 比较
//! - `--diff main...feature`：三个点语法，比较分支差异

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use crate::rule::SpanPolicy;

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
    /// 变更行范围列表（1-indexed, inclusive，新文件侧）。
    pub line_ranges: Vec<LineRange>,
    /// hunk 列表（新旧行号区间，供语义对比 / 行号映射用）。
    pub hunks: Vec<Hunk>,
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

/// 一次变更块（hunk）的新旧行号区间（1-indexed，长度可为 0）。
///
/// - `old_len == 0`：纯插入（插入点位于旧行 `old_start` 之前）
/// - `new_len == 0`：纯删除
///
/// 用于把旧文件行号精确翻译为新文件行号（语义对比 / baseline 精确匹配）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
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
                    hunks: Vec::new(),
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
                        hunks: Vec::new(),
                        is_new: true,
                    });
                }
            }
        } else if line.starts_with("@@ ") {
            // hunk header: @@ -old_start,old_len +new_start,new_len @@
            if let Some(f) = current_file.as_mut() {
                if let Some(hunk) = parse_hunk_header(line) {
                    f.hunks.push(hunk);
                    if hunk.new_len > 0 {
                        f.line_ranges.push(LineRange {
                            start: hunk.new_start,
                            end: hunk.new_start + hunk.new_len - 1,
                        });
                    }
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
                    hunks: Vec::new(),
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
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    // 格式: @@ -1,5 +10,3 @@ context
    //       @@ -1 +10 @@
    let line = line.strip_prefix("@@ ")?;
    let plus_pos = line.find(" +")?;
    let old_part = &line[..plus_pos]; // "-1,5" 或 "-1"
    let rest = &line[plus_pos + 2..]; // "10,3 @@ ..."
    let end = rest.find(' ').unwrap_or(rest.len());
    let new_part = &rest[..end];

    let (old_start, old_len) = parse_range(old_part.strip_prefix('-')?)?;
    let (new_start, new_len) = parse_range(new_part)?;
    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
    })
}

/// 解析 `start[,len]` 形式的行号区间；省略 len 时视为 1。
fn parse_range(s: &str) -> Option<(usize, usize)> {
    if let Some((start_s, len_s)) = s.split_once(',') {
        let start: usize = start_s.parse().ok()?;
        let len: usize = len_s.parse().ok()?;
        Some((start, len))
    } else {
        let start: usize = s.parse().ok()?;
        Some((start, 1))
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

    /// 按规则报告策略判定违规是否在变更范围内。
    ///
    /// - `SpanPolicy::Anchor`：仅锚点行 `line` 落在变更行范围才放行（默认，忽略 end_line）；
    /// - `SpanPolicy::Intersect`：违规区间 `[line, end_line]` 与变更行范围相交即放行。
    pub fn allows_policy(
        &self,
        file: &str,
        line: usize,
        end_line: Option<usize>,
        policy: SpanPolicy,
    ) -> bool {
        match policy {
            SpanPolicy::Anchor => self.allows(file, line),
            SpanPolicy::Intersect => self.allows_range(file, line, end_line),
        }
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

/// 新旧行号映射器：把旧文件行号精确翻译为新文件行号。
///
/// 基于 hunk 的旧/新行号区间构建偏移表，用于语义对比（违规集合差）与
/// baseline 精确匹配，避免行号漂移造成的误判。
#[derive(Debug, Clone, Default)]
pub struct LineMapper {
    /// 文件路径 → 按 old_start 排序的 hunk 列表。
    hunks: std::collections::HashMap<String, Vec<Hunk>>,
}

impl LineMapper {
    /// 从 FileDiff 列表构造映射器。
    pub fn from_diffs(diffs: &[FileDiff]) -> Self {
        let mut hunks: std::collections::HashMap<String, Vec<Hunk>> =
            std::collections::HashMap::new();
        for d in diffs {
            if d.hunks.is_empty() {
                continue;
            }
            let mut hs = d.hunks.clone();
            hs.sort_by_key(|h| h.old_start);
            hunks.insert(d.path.clone(), hs);
        }
        Self { hunks }
    }

    /// 旧行号 → 新行号；`None` 表示该行在变更中被删除（无对应新行）。
    ///
    /// 规则：
    /// - 位于某个 hunk 旧区间内的行：按区间内偏移映射（超出 new_len 视为被删除）；
    /// - 位于所有 hunk 之后的行：累加各 hunk 的 `new_len - old_len` 偏移；
    /// - 纯插入 hunk（old_len == 0）不占用旧行，只产生偏移。
    pub fn translate(&self, file: &str, old_line: usize) -> Option<usize> {
        let hs = self.hunks.get(file)?;
        let mut delta: i64 = 0;
        for h in hs {
            let old_end = h.old_start + h.old_len; // 半开区间 [old_start, old_end)
            if old_line < h.old_start {
                break; // hunk 按 old_start 升序，后续 hunk 不会影响更早的行
            }
            if old_line < old_end {
                // 行位于该 hunk 的旧区间内：按偏移映射，超出新长度视为删除
                let off = old_line - h.old_start;
                return if off < h.new_len {
                    Some(h.new_start + off)
                } else {
                    None
                };
            }
            delta += h.new_len as i64 - h.old_len as i64;
        }
        Some((old_line as i64 + delta) as usize)
    }
}

/// 读取某文件在旧版本中的原始字节（`git show <ref>:<path>`）。
///
/// - `Ok(Some(bytes))`：旧版本内容
/// - `Ok(None)`：该文件在旧版本中不存在（新增文件）
/// - `Err`：git 命令执行失败
pub fn read_old_source(
    repo_root: &Path,
    old_ref: Option<&str>,
    rel_path: &str,
) -> Result<Option<Vec<u8>>, GitDiffError> {
    let spec = match old_ref {
        Some(r) => format!("{r}:{rel_path}"),
        None => format!(":{rel_path}"), // 未指定 ref：对比索引版本
    };
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["show", &spec])
        .output()
        .map_err(|e| GitDiffError::GitCommand(format!("failed to run git show {spec}: {e}")))?;
    if !output.status.success() {
        // 常见情形：新增文件在旧版本不存在（git show 报 "exists on disk but not in <ref>"）
        return Ok(None);
    }
    Ok(Some(output.stdout))
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
            hunks: vec![],
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
            hunks: vec![],
            is_new: true,
        }];
        let filter = LineFilter::from_diffs(&diffs);
        // 新增文件无行范围，全量扫描
        assert!(filter.allows("src/New.java", 1));
        assert!(filter.allows("src/New.java", 999));
    }

    #[test]
    fn line_filter_policy_anchor_vs_intersect() {
        let diffs = vec![FileDiff {
            path: "src/Foo.java".to_string(),
            kind: DiffKind::Modified,
            line_ranges: vec![LineRange { start: 20, end: 25 }],
            hunks: vec![],
            is_new: false,
        }];
        let filter = LineFilter::from_diffs(&diffs);

        // Anchor：锚点行在变更范围内才放行，忽略 end_line
        assert!(filter.allows_policy("src/Foo.java", 20, None, SpanPolicy::Anchor));
        assert!(filter.allows_policy("src/Foo.java", 25, None, SpanPolicy::Anchor));
        // 锚点行 19 不在范围：即使区间 [19,30] 覆盖了整个 hunk 也不放行
        assert!(!filter.allows_policy("src/Foo.java", 19, Some(30), SpanPolicy::Anchor));
        assert!(!filter.allows_policy("src/Foo.java", 26, None, SpanPolicy::Anchor));

        // Intersect：区间与 hunk 相交即放行
        assert!(filter.allows_policy("src/Foo.java", 19, Some(30), SpanPolicy::Intersect));
        assert!(filter.allows_policy("src/Foo.java", 25, Some(30), SpanPolicy::Intersect));
        // 区间 [26,30] 与 hunk [20,25] 相邻但不重叠 → 不放行
        assert!(!filter.allows_policy("src/Foo.java", 26, Some(30), SpanPolicy::Intersect));
        // 完全不相交：区间 [30,40] 与 hunk [20,25] 无交集
        assert!(!filter.allows_policy("src/Foo.java", 30, Some(40), SpanPolicy::Intersect));
        // 区间覆盖整个 hunk 也放行（intersect 语义）
        assert!(filter.allows_policy("src/Foo.java", 10, Some(40), SpanPolicy::Intersect));
        // 无 end_line 时 intersect 退化为锚点行判定
        assert!(!filter.allows_policy("src/Foo.java", 19, None, SpanPolicy::Intersect));
        assert!(filter.allows_policy("src/Foo.java", 21, None, SpanPolicy::Intersect));
    }

    #[test]
    fn parse_hunk_header_basic() {
        let hunk = parse_hunk_header("@@ -10,5 +10,5 @@ fn foo() {").unwrap();
        assert_eq!(
            hunk,
            Hunk {
                old_start: 10,
                old_len: 5,
                new_start: 10,
                new_len: 5,
            }
        );
    }

    #[test]
    fn parse_hunk_header_single_line() {
        let hunk = parse_hunk_header("@@ -3 +4 @@").unwrap();
        assert_eq!(
            hunk,
            Hunk {
                old_start: 3,
                old_len: 1,
                new_start: 4,
                new_len: 1,
            }
        );
    }

    #[test]
    fn parse_hunk_header_zero_len() {
        // 纯插入：旧侧长度为 0
        let hunk = parse_hunk_header("@@ -7,0 +8,3 @@").unwrap();
        assert_eq!(
            hunk,
            Hunk {
                old_start: 7,
                old_len: 0,
                new_start: 8,
                new_len: 3,
            }
        );
        // 纯删除：新侧长度为 0
        let hunk = parse_hunk_header("@@ -4,2 +3,0 @@").unwrap();
        assert_eq!(
            hunk,
            Hunk {
                old_start: 4,
                old_len: 2,
                new_start: 3,
                new_len: 0,
            }
        );
    }

    #[test]
    fn line_mapper_translate_with_offset() {
        // 单个修改 hunk：旧 [10,14] → 新 [10,14]（5 行替换为 5 行，无偏移）
        let diffs = vec![FileDiff {
            path: "src/Foo.java".to_string(),
            kind: DiffKind::Modified,
            line_ranges: vec![LineRange { start: 10, end: 14 }],
            hunks: vec![Hunk {
                old_start: 10,
                old_len: 5,
                new_start: 10,
                new_len: 5,
            }],
            is_new: false,
        }];
        let mapper = LineMapper::from_diffs(&diffs);
        assert_eq!(mapper.translate("src/Foo.java", 9), Some(9));
        assert_eq!(mapper.translate("src/Foo.java", 12), Some(12));
        assert_eq!(mapper.translate("src/Foo.java", 14), Some(14));
        assert_eq!(mapper.translate("src/Foo.java", 15), Some(15));
        assert_eq!(mapper.translate("src/Other.java", 5), None);
    }

    #[test]
    fn line_mapper_translate_with_insertion() {
        // 在旧行 5 处插入 3 行（hunk 1），旧 [10,10] 修改（hunk 2）
        let diffs = vec![FileDiff {
            path: "src/Foo.java".to_string(),
            kind: DiffKind::Modified,
            line_ranges: vec![LineRange { start: 8, end: 10 }],
            hunks: vec![
                Hunk {
                    old_start: 5,
                    old_len: 0,
                    new_start: 8,
                    new_len: 3,
                },
                Hunk {
                    old_start: 10,
                    old_len: 1,
                    new_start: 13,
                    new_len: 1,
                },
            ],
            is_new: false,
        }];
        let mapper = LineMapper::from_diffs(&diffs);
        // 插入点之前不受影响
        assert_eq!(mapper.translate("src/Foo.java", 4), Some(4));
        // 插入点后的行整体下移 3
        assert_eq!(mapper.translate("src/Foo.java", 6), Some(9));
        assert_eq!(mapper.translate("src/Foo.java", 9), Some(12));
        // 修改行按区间内偏移映射
        assert_eq!(mapper.translate("src/Foo.java", 10), Some(13));
        // 修改行之后继续偏移
        assert_eq!(mapper.translate("src/Foo.java", 11), Some(14));
        assert_eq!(mapper.translate("src/Foo.java", 100), Some(103));
    }

    #[test]
    fn line_mapper_translate_with_deletion() {
        // 删除旧 [5,7] 三行（hunk 1），hunk 2 在旧行 12 修改
        let diffs = vec![FileDiff {
            path: "src/Foo.java".to_string(),
            kind: DiffKind::Modified,
            line_ranges: vec![LineRange { start: 9, end: 9 }],
            hunks: vec![
                Hunk {
                    old_start: 5,
                    old_len: 3,
                    new_start: 5,
                    new_len: 0,
                },
                Hunk {
                    old_start: 12,
                    old_len: 1,
                    new_start: 9,
                    new_len: 1,
                },
            ],
            is_new: false,
        }];
        let mapper = LineMapper::from_diffs(&diffs);
        // 删除区前不受影响
        assert_eq!(mapper.translate("src/Foo.java", 4), Some(4));
        // 被删除的行 → None
        assert_eq!(mapper.translate("src/Foo.java", 5), None);
        assert_eq!(mapper.translate("src/Foo.java", 7), None);
        // 删除后行整体上移 3
        assert_eq!(mapper.translate("src/Foo.java", 8), Some(5));
        assert_eq!(mapper.translate("src/Foo.java", 12), Some(9));
        assert_eq!(mapper.translate("src/Foo.java", 13), Some(10));
    }
}
