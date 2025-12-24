//! Minimal JSON parser for config files
//!
//! This is a simple JSON parser that handles the subset of JSON needed for config files:
//! - Objects with string keys
//! - String, integer, boolean values
//! - Arrays of strings

#![allow(dead_code)] // Library provides full API even if not all used

use std::collections::HashMap;

/// A JSON value
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    /// Get as bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get as i64
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get as u64
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        }
    }

    /// Get as string slice
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get as array
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Get object field
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }
}

/// Simple JSON parser
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Option<Value> {
        self.skip_whitespace();
        match self.peek()? {
            '"' => self.parse_string().map(Value::String),
            '{' => self.parse_object(),
            '[' => self.parse_array(),
            't' | 'f' => self.parse_bool(),
            'n' => self.parse_null(),
            c if c == '-' || c.is_ascii_digit() => self.parse_number(),
            _ => None,
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.peek()? != '"' {
            return None;
        }
        self.advance(); // consume opening quote

        let mut result = String::new();
        loop {
            match self.peek()? {
                '"' => {
                    self.advance();
                    return Some(result);
                }
                '\\' => {
                    self.advance();
                    match self.peek()? {
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        '/' => result.push('/'),
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        _ => return None,
                    }
                    self.advance();
                }
                c => {
                    result.push(c);
                    self.advance();
                }
            }
        }
    }

    fn parse_number(&mut self) -> Option<Value> {
        let start = self.pos;
        if self.peek()? == '-' {
            self.advance();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        let s = &self.input[start..self.pos];
        s.parse::<i64>().ok().map(Value::Number)
    }

    fn parse_bool(&mut self) -> Option<Value> {
        if self.input[self.pos..].starts_with("true") {
            self.pos += 4;
            Some(Value::Bool(true))
        } else if self.input[self.pos..].starts_with("false") {
            self.pos += 5;
            Some(Value::Bool(false))
        } else {
            None
        }
    }

    fn parse_null(&mut self) -> Option<Value> {
        if self.input[self.pos..].starts_with("null") {
            self.pos += 4;
            Some(Value::Null)
        } else {
            None
        }
    }

    fn parse_array(&mut self) -> Option<Value> {
        if self.peek()? != '[' {
            return None;
        }
        self.advance();

        let mut arr = Vec::new();
        self.skip_whitespace();

        if self.peek()? == ']' {
            self.advance();
            return Some(Value::Array(arr));
        }

        loop {
            arr.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek()? {
                ',' => {
                    self.advance();
                    self.skip_whitespace();
                }
                ']' => {
                    self.advance();
                    return Some(Value::Array(arr));
                }
                _ => return None,
            }
        }
    }

    fn parse_object(&mut self) -> Option<Value> {
        if self.peek()? != '{' {
            return None;
        }
        self.advance();

        let mut map = HashMap::new();
        self.skip_whitespace();

        if self.peek()? == '}' {
            self.advance();
            return Some(Value::Object(map));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.peek()? != ':' {
                return None;
            }
            self.advance();
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_whitespace();
            match self.peek()? {
                ',' => {
                    self.advance();
                }
                '}' => {
                    self.advance();
                    return Some(Value::Object(map));
                }
                _ => return None,
            }
        }
    }
}

/// Parse a JSON string
pub fn parse(input: &str) -> Option<Value> {
    let mut parser = Parser::new(input);
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.pos == parser.input.len() {
        Some(value)
    } else {
        None // trailing garbage
    }
}

/// Write a JSON value to a string (pretty-printed)
pub fn to_string_pretty(value: &Value) -> String {
    let mut output = String::new();
    write_value(&mut output, value, 0);
    output
}

fn write_value(out: &mut String, value: &Value, indent: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                out.push_str("[]");
            } else {
                out.push_str("[\n");
                for (i, item) in arr.iter().enumerate() {
                    for _ in 0..indent + 2 {
                        out.push(' ');
                    }
                    write_value(out, item, indent + 2);
                    if i < arr.len() - 1 {
                        out.push(',');
                    }
                    out.push('\n');
                }
                for _ in 0..indent {
                    out.push(' ');
                }
                out.push(']');
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{\n");
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort(); // consistent ordering
                for (i, key) in keys.iter().enumerate() {
                    let value = &map[*key];
                    for _ in 0..indent + 2 {
                        out.push(' ');
                    }
                    out.push('"');
                    out.push_str(key);
                    out.push_str("\": ");
                    write_value(out, value, indent + 2);
                    if i < keys.len() - 1 {
                        out.push(',');
                    }
                    out.push('\n');
                }
                for _ in 0..indent {
                    out.push(' ');
                }
                out.push('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_string() {
        let v = parse(r#""hello""#).unwrap();
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn test_parse_number() {
        let v = parse("42").unwrap();
        assert_eq!(v.as_i64(), Some(42));
    }

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse("true").unwrap().as_bool(), Some(true));
        assert_eq!(parse("false").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_parse_array() {
        let v = parse(r#"["a", "b", "c"]"#).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str(), Some("a"));
    }

    #[test]
    fn test_parse_object() {
        let v = parse(r#"{"key": "value", "num": 42}"#).unwrap();
        assert_eq!(v.get("key").unwrap().as_str(), Some("value"));
        assert_eq!(v.get("num").unwrap().as_i64(), Some(42));
    }

    #[test]
    fn test_roundtrip() {
        let input = r#"{"bool": true, "num": 123, "str": "hello"}"#;
        let v = parse(input).unwrap();
        let output = to_string_pretty(&v);
        let v2 = parse(&output).unwrap();
        assert_eq!(v, v2);
    }
}
