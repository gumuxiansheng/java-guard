//! rule: J202
//! title: 缩进禁止使用 tab 字符
//! severity: minor
//! category: format

// 规范 4.3【强制】缩进采用 4 个空格，禁止使用 tab 字符。
// 注意：字符串字面量中的 \t 也会命中，属可接受的小概率误报，
// 如需完全避免则应在 Rust 侧按 token 位置判定（P1）。
let vs = [];
let i = 1;
while i <= line_count {
    let t = lines[i].to_string();
    if t.contains("\t") {
        vs.push(#{
            line: i,
            message: "第 " + i + " 行包含 tab 字符，缩进请统一使用 4 个空格"
        });
    }
    i = i + 1;
}
vs
