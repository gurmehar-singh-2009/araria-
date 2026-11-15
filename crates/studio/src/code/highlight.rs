//! Lua syntax highlighting.
//!
//! A forgiving tokenizer (it never fails, whatever is being typed) whose
//! tokens are turned into a coloured text layout.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

use super::api;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    /// `true`, `false`, `nil`.
    Constant,
    /// A documented global or library such as `workspace` or `Vector3`.
    Builtin,
    /// An identifier followed by `(`, a string or a table: a call.
    Call,
    /// An identifier after `.` or `:`.
    Member,
    Identifier,
    Operator,
    Whitespace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
}

/// Length of a long-bracket opener (`[[`, `[==[`) at the start of `text`.
fn long_bracket(text: &[u8]) -> Option<usize> {
    if text.first() != Some(&b'[') {
        return None;
    }
    let level = text[1..].iter().take_while(|b| **b == b'=').count();
    (text.get(1 + level) == Some(&b'[')).then_some(level)
}

/// Byte offset just past the matching `]==]`, or the end of the text.
fn long_bracket_end(text: &[u8], start: usize, level: usize) -> usize {
    let mut i = start;
    while i < text.len() {
        if text[i] == b']' && text[i + 1..].iter().take_while(|b| **b == b'=').count() >= level && text.get(i + 1 + level) == Some(&b']') {
            return i + 2 + level;
        }
        i += 1;
    }
    text.len()
}

fn is_builtin(word: &str) -> bool {
    api::GLOBALS.iter().any(|g| g.name == word) || api::LIBRARIES.iter().any(|l| l.name == word)
}

pub fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];
        let kind = if c.is_ascii_whitespace() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            TokenKind::Whitespace
        } else if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            match long_bracket(&bytes[i..]) {
                Some(level) => i = long_bracket_end(bytes, i + 2 + level, level),
                None => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            TokenKind::Comment
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != c && bytes[i] != b'\n' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
            TokenKind::String
        } else if let Some(level) = long_bracket(&bytes[i..]) {
            i = long_bracket_end(bytes, i + 2 + level, level);
            TokenKind::String
        } else if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_' || (matches!(bytes[i], b'+' | b'-') && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P'))) {
                i += 1;
            }
            TokenKind::Number
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];
            let after_dot = tokens.iter().rev().find(|t| t.kind != TokenKind::Whitespace).is_some_and(|t| t.kind == TokenKind::Operator && matches!(&source[t.range.clone()], "." | ":"));
            let next = bytes[i..].iter().find(|b| !b.is_ascii_whitespace()).copied();
            if matches!(word, "true" | "false" | "nil") {
                TokenKind::Constant
            } else if api::KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if matches!(next, Some(b'(' | b'"' | b'\'' | b'{')) {
                TokenKind::Call
            } else if after_dot {
                TokenKind::Member
            } else if is_builtin(word) {
                TokenKind::Builtin
            } else {
                TokenKind::Identifier
            }
        } else {
            // Operators and punctuation; step over whole UTF-8 characters.
            i += 1;
            while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                i += 1;
            }
            TokenKind::Operator
        };
        // Escapes at the very end of the text can step past it.
        i = i.min(bytes.len());
        tokens.push(Token { kind, range: start..i });
    }
    tokens
}

/// The editor's colour scheme.
pub fn color(kind: TokenKind) -> Color32 {
    match kind {
        TokenKind::Comment => Color32::from_rgb(106, 115, 145),
        TokenKind::String => Color32::from_rgb(158, 206, 106),
        TokenKind::Number | TokenKind::Constant => Color32::from_rgb(255, 158, 100),
        TokenKind::Keyword => Color32::from_rgb(187, 154, 247),
        TokenKind::Builtin => Color32::from_rgb(42, 195, 222),
        TokenKind::Call => Color32::from_rgb(122, 162, 247),
        TokenKind::Member => Color32::from_rgb(125, 207, 255),
        TokenKind::Identifier => Color32::from_rgb(214, 219, 240),
        TokenKind::Operator | TokenKind::Whitespace => Color32::from_rgb(137, 148, 184),
    }
}

/// Lays `source` out as highlighted, unwrapped code.
pub fn layout(source: &str, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    for token in tokenize(source) {
        let format = TextFormat { font_id: font.clone(), color: color(token.kind), ..Default::default() };
        job.append(&source[token.range], 0.0, format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(TokenKind, &str)> {
        tokenize(source).into_iter().filter(|t| t.kind != TokenKind::Whitespace).map(|t| (t.kind, &source[t.range])).collect()
    }

    #[test]
    fn classifies_tokens() {
        use TokenKind::*;
        assert_eq!(
            kinds("local part = Instance.new(\"Part\") -- make\npart.Size = Vector3.new(1, 2.5, 0x1F)"),
            vec![
                (Keyword, "local"),
                (Identifier, "part"),
                (Operator, "="),
                (Builtin, "Instance"),
                (Operator, "."),
                (Call, "new"),
                (Operator, "("),
                (String, "\"Part\""),
                (Operator, ")"),
                (Comment, "-- make"),
                (Identifier, "part"),
                (Operator, "."),
                (Member, "Size"),
                (Operator, "="),
                (Builtin, "Vector3"),
                (Operator, "."),
                (Call, "new"),
                (Operator, "("),
                (Number, "1"),
                (Operator, ","),
                (Number, "2.5"),
                (Operator, ","),
                (Number, "0x1F"),
                (Operator, ")"),
            ]
        );
        assert_eq!(kinds("x = [[a\nb]] --[==[ c\n]] d ]==] nil"), vec![(Identifier, "x"), (Operator, "="), (String, "[[a\nb]]"), (Comment, "--[==[ c\n]] d ]==]"), (Constant, "nil")]);
    }

    #[test]
    fn unfinished_input_never_panics_and_covers_the_text() {
        for source in ["\"unterminated", "'esc\\", "--[[ open", "[==[ open", "1e", "a.", "é ≠ ö", "x = \"a\\", ""] {
            let tokens = tokenize(source);
            let mut end = 0;
            for token in &tokens {
                assert_eq!(token.range.start, end, "{source:?}");
                end = token.range.end;
                assert!(source.is_char_boundary(end), "{source:?}");
            }
            assert_eq!(end, source.len(), "{source:?}");
        }
    }
}
