//! rule: J204
//! title: 源文件必须使用 Unix 换行符
//! severity: minor
//! category: format

// 规范 4.3【强制】IDE 中文件的换行符使用 Unix 格式（\n），不要使用 Windows 格式（\r\n）。
// source 为未切分的全文，保留原始换行符，因此可直接判断。
let vs = [];
if line_count > 0 {
    if source.contains("\r\n") {
        vs.push(#{
            line: 1,
            message: "源文件使用了 Windows 换行符（CRLF），请统一为 Unix 换行符（LF）"
        });
    }
}
vs
