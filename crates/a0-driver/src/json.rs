//! JSON values, enough for the Language Server Protocol: parsing, building and printing.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Members in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Parses one JSON value (surrounding whitespace allowed).
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            bytes: text.as_bytes(),
            text,
            pos: 0,
        };
        let v = p.value(0)?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(format!("unexpected text at byte {}", p.pos));
        }
        Ok(v)
    }

    /// An object from `(key, value)` pairs.
    pub fn object<const N: usize>(members: [(&str, Json); N]) -> Json {
        Json::Object(
            members
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    /// The member `key` of an object; `Null` for anything else.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(members) => members
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&Json::Null, |(_, v)| v),
            _ => &Json::Null,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_f64()
            .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= u32::MAX as f64)
            .map(|n| n as u32)
    }

    pub fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => &[],
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::String(s.to_string())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::String(s)
    }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}

impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Number(n as f64)
    }
}

impl From<Vec<Json>> for Json {
    fn from(items: Vec<Json>) -> Json {
        Json::Array(items)
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Number(n) if !n.is_finite() => f.write_str("null"),
            // Integers (ids, positions) print without a fraction.
            Json::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => write!(f, "{}", *n as i64),
            Json::Number(n) => write!(f, "{n}"),
            Json::String(s) => write_string(f, s),
            Json::Array(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Json::Object(members) => {
                f.write_str("{")?;
                for (i, (k, v)) in members.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write_string(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_str("}")
            }
        }
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

/// Nesting deeper than this is rejected rather than risking the stack.
const MAX_DEPTH: u32 = 256;

struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn error<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.pos))
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.text[self.pos..].starts_with(word) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: u32) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.error("nesting too deep");
        }
        self.skip_ws();
        match self.bytes.get(self.pos) {
            None => self.error("unexpected end"),
            Some(b'{') => {
                self.pos += 1;
                let mut members = Vec::new();
                self.skip_ws();
                if self.eat("}") {
                    return Ok(Json::Object(members));
                }
                loop {
                    self.skip_ws();
                    let key = self.string()?;
                    self.skip_ws();
                    if !self.eat(":") {
                        return self.error("expected ':'");
                    }
                    let v = self.value(depth + 1)?;
                    members.push((key, v));
                    self.skip_ws();
                    if self.eat("}") {
                        return Ok(Json::Object(members));
                    }
                    if !self.eat(",") {
                        return self.error("expected ',' or '}'");
                    }
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.eat("]") {
                    return Ok(Json::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.skip_ws();
                    if self.eat("]") {
                        return Ok(Json::Array(items));
                    }
                    if !self.eat(",") {
                        return self.error("expected ',' or ']'");
                    }
                }
            }
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.pos;
                while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') =
                    self.bytes.get(self.pos)
                {
                    self.pos += 1;
                }
                match self.text[start..self.pos].parse::<f64>() {
                    Ok(n) => Ok(Json::Number(n)),
                    Err(_) => {
                        self.pos = start;
                        self.error("invalid number")
                    }
                }
            }
            Some(_) => self.error("unexpected character"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat("\"") {
            return self.error("expected a string");
        }
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(&self.text[start..self.pos]);
            match self.bytes.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = match self.bytes.get(self.pos) {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            let hi = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                            continue;
                        }
                        _ => return self.error("invalid escape"),
                    };
                    self.pos += 1;
                    out.push(c);
                }
                _ => return self.error("unterminated string"),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.text.get(self.pos..self.pos + 4);
        match digits.and_then(|d| u32::from_str_radix(d, 16).ok()) {
            Some(v) => {
                self.pos += 4;
                Ok(v)
            }
            None => self.error("invalid \\u escape"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let text = r#"{"a":[1,-2.5,true,false,null],"b":"x\"\\\n\u0001é","c":{}}"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.get("a").as_array().len(), 5);
        assert_eq!(v.get("a").as_array()[1].as_f64(), Some(-2.5));
        assert_eq!(v.get("b").as_str(), Some("x\"\\\n\u{1}é"));
        assert_eq!(v.to_string(), text);
        assert!(v.get("missing").is_null());
        assert_eq!(Json::parse(" [ ] ").unwrap(), Json::Array(Vec::new()));
    }

    #[test]
    fn escapes_and_surrogate_pairs() {
        let v = Json::parse(r#""\ud83d\ude00 \u00e9 \/""#).unwrap();
        assert_eq!(v.as_str(), Some("😀 é /"));
        assert_eq!(Json::Number(3.0).to_string(), "3");
        assert_eq!(Json::Number(0.5).to_string(), "0.5");
        assert_eq!(Json::from(7u32).as_u32(), Some(7));
    }

    #[test]
    fn bad_input_is_an_error() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "\"abc",
            "tru",
            "1 2",
            "\"\\x\"",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(300) + &"]".repeat(300);
        assert!(Json::parse(&deep).is_err());
    }
}
