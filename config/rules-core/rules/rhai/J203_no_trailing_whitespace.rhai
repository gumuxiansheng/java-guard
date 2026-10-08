//! rule: J203
//! title: 行尾不得有多余空白字符
//! severity: minor
//! category: format

// 行尾空白会造成无意义的 diff 噪声。
// 不能用 trim()（本引擎未注册该方法且会静默返回 unit），改为取末位字符判断。
let vs = [];
let i = 1;
while i <= line_count {
    let t = lines[i].to_string();
    let n = len(t);
    if n > 0 {
        let last = t.sub_string(n - 1, 1);
        if last == " " || last == "\t" {
            vs.push(#{
                line: i,
                message: "第 " + i + " 行行尾存在多余空白字符，请删除"
            });
        }
    }
    i = i + 1;
}
vs
