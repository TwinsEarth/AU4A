//! 规范 JSON（AU4A-CJ）。
//!
//! 签名和哈希都建立在字节之上，所以「同一份语义必须得到同一串字节」不是风格问题。
//! AU4A-CJ 的规则：
//!
//! 1. 对象键按 UTF-8 字节序升序（不是按 Unicode 码点，也不是插入序）。
//! 2. 无任何多余空白：`{"a":1,"b":[2,3]}`。
//! 3. **禁止浮点**：出现浮点直接 [`CoreError::FloatForbidden`]。金额、比率、分数一律整数
//!    （微积分 / 万分比），这样守恒断言永远精确。
//! 4. 数组保持原序。
//! 5. 字符串按 JSON 最小转义规则输出（`"`、`\`、控制字符），其余原样 UTF-8。

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{CoreError, CoreResult};

/// 把任意 JSON 值编码为 AU4A 规范 JSON 字符串。
pub fn canonicalize(value: &Value) -> CoreResult<String> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

/// 规范 JSON 字节的 SHA-256（小写 hex）。这是 AU4A 里所有「内容寻址」的来源。
pub fn canonical_hash(value: &Value) -> CoreResult<String> {
    let s = canonicalize(value)?;
    Ok(hex::encode(Sha256::digest(s.as_bytes())))
}

fn write_canonical(value: &Value, out: &mut String) -> CoreResult<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                return Err(CoreError::FloatForbidden);
            }
        }
        Value::String(s) => out.push_str(&encode_string(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&encode_string(k));
                out.push(':');
                write_canonical(&map[k.as_str()], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn encode_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_by_bytes_not_insertion() {
        let v = json!({"b": 1, "a": 2, "Z": 3});
        assert_eq!(canonicalize(&v).unwrap(), r#"{"Z":3,"a":2,"b":1}"#);
    }

    #[test]
    fn floats_are_refused() {
        let v = json!({"price": 1.5});
        assert_eq!(canonicalize(&v), Err(CoreError::FloatForbidden));
    }

    #[test]
    fn same_semantics_same_bytes_regardless_of_key_order() {
        let a: Value = serde_json::from_str(r#"{"z":1,"a":{"y":2,"b":[1,2]}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"b":[1,2],"y":2},"z":1}"#).unwrap();
        assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
        assert_eq!(canonical_hash(&a).unwrap(), canonical_hash(&b).unwrap());
    }

    #[test]
    fn integers_survive_roundtrip_exactly() {
        let big = json!({"v": 9_007_199_254_740_993i64});
        assert_eq!(canonicalize(&big).unwrap(), r#"{"v":9007199254740993}"#);
    }

    #[test]
    fn control_characters_are_escaped_and_unicode_is_not() {
        let v = json!({"s": "智能体\n宇宙"});
        assert_eq!(canonicalize(&v).unwrap(), "{\"s\":\"智能体\\n宇宙\"}");
    }
}
