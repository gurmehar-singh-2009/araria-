//! Tokenizer for Lua source text.

use crate::error::{Error, Result};
use crate::value::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Eof,
    Name(String),
    Str(Vec<u8>),
    Int(i64),
    Num(f64),
    // keywords
    And,
    Break,
    Do,
    Else,
    ElseIf,
    End,
    False,
    For,
    Function,
    Goto,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,
    // operators and punctuation
    Plus,
    Minus,
    Star,
    Slash,
    DSlash,
    Percent,
    Caret,
    Hash,
    Amp,
    Tilde,
    Pipe,
    Shl,
    Shr,
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    DColon,
    Semi,
    Colon,
    Comma,
    Dot,
    Concat,
    Ellipsis,
    /// Compound assignment (`+=`, `..=`, ...); the payload is the operator token.
    Compound(Box<Tok>),
}

impl Tok {
    /// How the token is spelled in error messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Eof => "<eof>".into(),
            Tok::Name(n) => n.clone(),
            Tok::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s)),
            Tok::Int(i) => i.to_string(),
            Tok::Num(n) => n.to_string(),
            Tok::Compound(op) => format!("{}=", op.describe()),
            other => other.symbol().to_string(),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Tok::And => "and",
            Tok::Break => "break",
            Tok::Do => "do",
            Tok::Else => "else",
            Tok::ElseIf => "elseif",
            Tok::End => "end",
            Tok::False => "false",
            Tok::For => "for",
            Tok::Function => "function",
            Tok::Goto => "goto",
            Tok::If => "if",
            Tok::In => "in",
            Tok::Local => "local",
            Tok::Nil => "nil",
            Tok::Not => "not",
            Tok::Or => "or",
            Tok::Repeat => "repeat",
            Tok::Return => "return",
            Tok::Then => "then",
            Tok::True => "true",
            Tok::Until => "until",
            Tok::While => "while",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::DSlash => "//",
            Tok::Percent => "%",
            Tok::Caret => "^",
            Tok::Hash => "#",
            Tok::Amp => "&",
            Tok::Tilde => "~",
            Tok::Pipe => "|",
            Tok::Shl => "<<",
            Tok::Shr => ">>",
            Tok::Eq => "==",
            Tok::Ne => "~=",
            Tok::Le => "<=",
            Tok::Ge => ">=",
            Tok::Lt => "<",
            Tok::Gt => ">",
            Tok::Assign => "=",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::DColon => "::",
            Tok::Semi => ";",
            Tok::Colon => ":",
            Tok::Comma => ",",
            Tok::Dot => ".",
            Tok::Concat => "..",
            Tok::Ellipsis => "...",
            _ => "?",
        }
    }
}

/// Reserved words, exposed so tooling (the script editor) can highlight them.
pub const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn keyword(name: &str) -> Option<Tok> {
    Some(match name {
        "and" => Tok::And,
        "break" => Tok::Break,
        "do" => Tok::Do,
        "else" => Tok::Else,
        "elseif" => Tok::ElseIf,
        "end" => Tok::End,
        "false" => Tok::False,
        "for" => Tok::For,
        "function" => Tok::Function,
        "goto" => Tok::Goto,
        "if" => Tok::If,
        "in" => Tok::In,
        "local" => Tok::Local,
        "nil" => Tok::Nil,
        "not" => Tok::Not,
        "or" => Tok::Or,
        "repeat" => Tok::Repeat,
        "return" => Tok::Return,
        "then" => Tok::Then,
        "true" => Tok::True,
        "until" => Tok::Until,
        "while" => Tok::While,
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    chunk: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, chunk: &'a str) -> Self {
        let mut lexer = Lexer { src: src.as_bytes(), pos: 0, line: 1, chunk };
        // Skip a leading shebang line.
        if lexer.src.starts_with(b"#") {
            while lexer.peek().is_some_and(|c| c != b'\n') {
                lexer.pos += 1;
            }
        }
        lexer
    }

    /// Tokenizes the whole input.
    pub fn tokenize(mut self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        loop {
            let token = self.next_token()?;
            let done = token.tok == Tok::Eof;
            out.push(token);
            if done {
                return Ok(out);
            }
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T> {
        Err(Error::syntax(self.chunk, self.line, msg))
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match self.peek() {
                Some(b'\n') => {
                    self.line += 1;
                    self.pos += 1;
                }
                Some(b' ' | b'\t' | b'\r' | 0x0b | 0x0c) => self.pos += 1,
                Some(b'-') if self.peek_at(1) == Some(b'-') => {
                    self.pos += 2;
                    if self.peek() == Some(b'[') {
                        if let Some(level) = self.long_bracket_level() {
                            self.read_long_string(level)?;
                            continue;
                        }
                    }
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.pos += 1;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// If positioned at `[`, `[=`... followed by `[`, returns the level.
    fn long_bracket_level(&self) -> Option<usize> {
        let mut i = 1;
        while self.peek_at(i) == Some(b'=') {
            i += 1;
        }
        (self.peek_at(i) == Some(b'[')).then_some(i - 1)
    }

    fn read_long_string(&mut self, level: usize) -> Result<Vec<u8>> {
        let start_line = self.line;
        self.pos += level + 2;
        // A newline directly after the opening bracket is skipped.
        if self.peek() == Some(b'\r') {
            self.pos += 1;
        }
        if self.peek() == Some(b'\n') {
            self.line += 1;
            self.pos += 1;
        }
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => {
                    return Err(Error::syntax(self.chunk, start_line, "unfinished long string or comment"));
                }
                Some(b']') => {
                    let mut i = 1;
                    while self.peek_at(i) == Some(b'=') {
                        i += 1;
                    }
                    if i - 1 == level && self.peek_at(i) == Some(b']') {
                        self.pos += i + 1;
                        return Ok(out);
                    }
                    out.push(b']');
                    self.pos += 1;
                }
                Some(c) => {
                    if c == b'\n' {
                        self.line += 1;
                    }
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn read_string(&mut self, quote: u8) -> Result<Vec<u8>> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return self.err("unfinished string");
            };
            self.pos += 1;
            match c {
                c if c == quote => return Ok(out),
                b'\n' => return self.err("unfinished string"),
                b'\\' => {
                    let Some(e) = self.peek() else {
                        return self.err("unfinished string");
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'a' => out.push(7),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'v' => out.push(11),
                        b'\\' => out.push(b'\\'),
                        b'"' => out.push(b'"'),
                        b'\'' => out.push(b'\''),
                        b'\n' => {
                            self.line += 1;
                            out.push(b'\n');
                        }
                        b'x' => {
                            let mut v = 0u32;
                            for _ in 0..2 {
                                match self.peek().and_then(|c| (c as char).to_digit(16)) {
                                    Some(d) => {
                                        v = v * 16 + d;
                                        self.pos += 1;
                                    }
                                    None => return self.err("hexadecimal digit expected"),
                                }
                            }
                            out.push(v as u8);
                        }
                        b'z' => {
                            while let Some(c) = self.peek() {
                                if c == b'\n' {
                                    self.line += 1;
                                }
                                if c.is_ascii_whitespace() {
                                    self.pos += 1;
                                } else {
                                    break;
                                }
                            }
                        }
                        b'u' => {
                            if !self.eat(b'{') {
                                return self.err("missing '{' in \\u{xxxx}");
                            }
                            let mut v = 0u32;
                            let mut digits = 0;
                            while let Some(d) = self.peek().and_then(|c| (c as char).to_digit(16)) {
                                v = v.saturating_mul(16).saturating_add(d);
                                digits += 1;
                                self.pos += 1;
                            }
                            if digits == 0 || !self.eat(b'}') {
                                return self.err("malformed \\u{xxxx} escape");
                            }
                            match char::from_u32(v) {
                                Some(ch) => {
                                    let mut buf = [0u8; 4];
                                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                }
                                None => return self.err("UTF-8 value too large"),
                            }
                        }
                        d if d.is_ascii_digit() => {
                            let mut v = u32::from(d - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(c) if c.is_ascii_digit() => {
                                        v = v * 10 + u32::from(c - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if v > 255 {
                                return self.err("decimal escape too large");
                            }
                            out.push(v as u8);
                        }
                        _ => return self.err("invalid escape sequence"),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn read_number(&mut self) -> Result<Tok> {
        let start = self.pos;
        let is_hex = self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X'));
        if is_hex {
            self.pos += 2;
        }
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_alphanumeric() || c == b'.' || c == b'_' => {
                    let exp = if is_hex { matches!(c, b'p' | b'P') } else { matches!(c, b'e' | b'E') };
                    self.pos += 1;
                    if exp && matches!(self.peek(), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        match parse_number(text) {
            Some(Value::Int(i)) => Ok(Tok::Int(i)),
            Some(Value::Num(n)) => Ok(Tok::Num(n)),
            _ => self.err(format!("malformed number near '{text}'")),
        }
    }

    fn next_token(&mut self) -> Result<Token> {
        self.skip_trivia()?;
        let line = self.line;
        let Some(c) = self.peek() else {
            return Ok(Token { tok: Tok::Eof, line });
        };
        let tok = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
                    self.pos += 1;
                }
                let name = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                keyword(name).unwrap_or_else(|| Tok::Name(name.to_string()))
            }
            b'0'..=b'9' => self.read_number()?,
            b'.' if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => self.read_number()?,
            b'"' | b'\'' => Tok::Str(self.read_string(c)?),
            b'[' => {
                if let Some(level) = self.long_bracket_level() {
                    Tok::Str(self.read_long_string(level)?)
                } else {
                    self.pos += 1;
                    Tok::LBracket
                }
            }
            _ => {
                self.pos += 1;
                let tok = match c {
                    b'+' => Tok::Plus,
                    b'-' => Tok::Minus,
                    b'*' => Tok::Star,
                    b'/' => {
                        if self.eat(b'/') {
                            Tok::DSlash
                        } else {
                            Tok::Slash
                        }
                    }
                    b'%' => Tok::Percent,
                    b'^' => Tok::Caret,
                    b'#' => Tok::Hash,
                    b'&' => Tok::Amp,
                    b'|' => Tok::Pipe,
                    b'~' => {
                        if self.eat(b'=') {
                            Tok::Ne
                        } else {
                            Tok::Tilde
                        }
                    }
                    b'<' => {
                        if self.eat(b'<') {
                            Tok::Shl
                        } else if self.eat(b'=') {
                            Tok::Le
                        } else {
                            Tok::Lt
                        }
                    }
                    b'>' => {
                        if self.eat(b'>') {
                            Tok::Shr
                        } else if self.eat(b'=') {
                            Tok::Ge
                        } else {
                            Tok::Gt
                        }
                    }
                    b'=' => {
                        if self.eat(b'=') {
                            Tok::Eq
                        } else {
                            Tok::Assign
                        }
                    }
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b']' => Tok::RBracket,
                    b';' => Tok::Semi,
                    b',' => Tok::Comma,
                    b':' => {
                        if self.eat(b':') {
                            Tok::DColon
                        } else {
                            Tok::Colon
                        }
                    }
                    b'.' => {
                        if self.eat(b'.') {
                            if self.eat(b'.') {
                                Tok::Ellipsis
                            } else {
                                Tok::Concat
                            }
                        } else {
                            Tok::Dot
                        }
                    }
                    other => {
                        let shown = if other.is_ascii_graphic() {
                            (other as char).to_string()
                        } else {
                            format!("\\{other}")
                        };
                        return self.err(format!("unexpected symbol near '{shown}'"));
                    }
                };
                // Compound assignment: an arithmetic/concat operator glued to `=`
                // (but not `==`).
                let compoundable = matches!(
                    tok,
                    Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::DSlash | Tok::Percent | Tok::Caret | Tok::Concat
                );
                if compoundable && self.peek() == Some(b'=') && self.peek_at(1) != Some(b'=') {
                    self.pos += 1;
                    Tok::Compound(Box::new(tok))
                } else {
                    tok
                }
            }
        };
        Ok(Token { tok, line })
    }
}

/// Parses a Lua numeric literal (decimal or hexadecimal, integer or float).
pub fn parse_number(text: &str) -> Option<Value> {
    let (neg, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if body.is_empty() {
        return None;
    }
    let value = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        parse_hex(hex)?
    } else {
        if !body.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
            return None;
        }
        if body.bytes().all(|c| c.is_ascii_digit()) {
            match body.parse::<i64>() {
                Ok(i) => Value::Int(i),
                Err(_) => Value::Num(body.parse::<f64>().ok()?),
            }
        } else {
            if !body.bytes().any(|c| c.is_ascii_digit()) {
                return None;
            }
            Value::Num(body.parse::<f64>().ok()?)
        }
    };
    Some(match (neg, value) {
        (true, Value::Int(i)) => Value::Int(i.wrapping_neg()),
        (true, Value::Num(n)) => Value::Num(-n),
        (_, v) => v,
    })
}

fn parse_hex(hex: &str) -> Option<Value> {
    let (mantissa, exponent) = match hex.find(['p', 'P']) {
        Some(i) => (&hex[..i], Some(hex[i + 1..].parse::<i32>().ok()?)),
        None => (hex, None),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (mantissa, None),
    };
    if int_part.is_empty() && frac_part.is_none_or(str::is_empty) {
        return None;
    }
    if frac_part.is_none() && exponent.is_none() {
        // Hex integers wrap around modulo 2^64, as in Lua.
        let mut v: u64 = 0;
        for c in int_part.chars() {
            v = v.wrapping_mul(16).wrapping_add(u64::from(c.to_digit(16)?));
        }
        return Some(Value::Int(v as i64));
    }
    let mut v = 0f64;
    for c in int_part.chars() {
        v = v * 16.0 + f64::from(c.to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.unwrap_or("").chars() {
        v += f64::from(c.to_digit(16)?) * scale;
        scale /= 16.0;
    }
    Some(Value::Num(v * 2f64.powi(exponent.unwrap_or(0))))
}
