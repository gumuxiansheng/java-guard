//! rule: J201
//! title: 单行字符数不超过 120
//! severity: minor
//! category: format
//! params: max_len=120

// 文本类规则：直接读源码行。
//   lines      1-based，lines[i] 即第 i 行
//   line_count 实际行数（遍历上界；注意 len(lines) == line_count + 1）
let vs = [];
let max_len = config["max_len"];
if max_len == () { max_len = 120; }

let i = 1;
while i <= line_count {
    let t = lines[i].to_string();
    let n = len(t);
    if n > max_len {
        vs.push(#{
            line: i,
            message: "第 " + i + " 行长度 " + n + " 超过 " + max_len + " 字符，请换行（第二行缩进 4 空格，运算符与下文一起换行）"
        });
    }
    i = i + 1;
}
vs
