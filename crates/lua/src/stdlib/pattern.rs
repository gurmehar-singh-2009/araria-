//! Lua pattern matching (the engine behind `string.find`, `match`, `gmatch`
//! and `gsub`). Patterns operate on bytes, exactly as in reference Lua.

use crate::error::{Error, Result};
use crate::value::{LuaStr, Value};

const MAX_CAPTURES: usize = 32;
const MAX_RECURSION: u32 = 200;
const CAP_UNFINISHED: isize = -1;
const CAP_POSITION: isize = -2;
const ESC: u8 = b'%';

/// State for matching one pattern against one subject string.
pub struct Matcher<'a> {
    pub src: &'a [u8],
    pub pat: &'a [u8],
    level: usize,
    capture: [(usize, isize); MAX_CAPTURES],
    depth: u32,
}

fn class_matches(c: u8, class: u8) -> bool {
    let result = match class.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => c.is_ascii_whitespace() || c == 0x0b,
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        _ => return class == c,
    };
    if class.is_ascii_uppercase() { !result } else { result }
}

impl<'a> Matcher<'a> {
    pub fn new(src: &'a [u8], pat: &'a [u8]) -> Self {
        Matcher { src, pat, level: 0, capture: [(0, 0); MAX_CAPTURES], depth: 0 }
    }

    /// Clears captures before a fresh match attempt.
    pub fn reset(&mut self) {
        self.level = 0;
        self.depth = 0;
    }

    /// Index just past the single-character class starting at `p`.
    fn class_end(&self, mut p: usize) -> Result<usize> {
        let c = self.pat[p];
        p += 1;
        if c == ESC {
            if p >= self.pat.len() {
                return Err(Error::runtime("malformed pattern (ends with '%')"));
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if self.pat.get(p) == Some(&b'^') {
                p += 1;
            }
            loop {
                if p >= self.pat.len() {
                    return Err(Error::runtime("malformed pattern (missing ']')"));
                }
                let c = self.pat[p];
                p += 1;
                if c == ESC && p < self.pat.len() {
                    p += 1;
                }
                if self.pat.get(p) == Some(&b']') {
                    return Ok(p + 1);
                }
            }
        }
        Ok(p)
    }

    /// Tests `c` against the set `[...]` spanning `p..=end` (`end` at `]`).
    fn set_matches(&self, c: u8, mut p: usize, end: usize) -> bool {
        let mut sig = true;
        if self.pat.get(p + 1) == Some(&b'^') {
            sig = false;
            p += 1;
        }
        p += 1;
        while p < end {
            if self.pat[p] == ESC {
                p += 1;
                if p < end && class_matches(c, self.pat[p]) {
                    return sig;
                }
            } else if self.pat.get(p + 1) == Some(&b'-') && p + 2 < end {
                if self.pat[p] <= c && c <= self.pat[p + 2] {
                    return sig;
                }
                p += 2;
            } else if self.pat[p] == c {
                return sig;
            }
            p += 1;
        }
        !sig
    }

    fn single_matches(&self, s: usize, p: usize, ep: usize) -> bool {
        let Some(&c) = self.src.get(s) else { return false };
        match self.pat[p] {
            b'.' => true,
            ESC => class_matches(c, self.pat[p + 1]),
            b'[' => self.set_matches(c, p, ep - 1),
            literal => literal == c,
        }
    }

    fn match_balance(&self, s: usize, p: usize) -> Result<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return Err(Error::runtime("malformed pattern (missing arguments to '%b')"));
        }
        if self.src.get(s) != Some(&self.pat[p]) {
            return Ok(None);
        }
        let (open, close) = (self.pat[p], self.pat[p + 1]);
        let mut depth = 1;
        for (i, &c) in self.src.iter().enumerate().skip(s + 1) {
            if c == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(i + 1));
                }
            } else if c == open {
                depth += 1;
            }
        }
        Ok(None)
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        let mut count = 0;
        while self.single_matches(s + count, p, ep) {
            count += 1;
        }
        loop {
            if let Some(end) = self.do_match(s + count, ep + 1)? {
                return Ok(Some(end));
            }
            if count == 0 {
                return Ok(None);
            }
            count -= 1;
        }
    }

    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> Result<Option<usize>> {
        loop {
            if let Some(end) = self.do_match(s, ep + 1)? {
                return Ok(Some(end));
            }
            if self.single_matches(s, p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> Result<Option<usize>> {
        if self.level >= MAX_CAPTURES {
            return Err(Error::runtime("too many captures"));
        }
        self.capture[self.level] = (s, what);
        self.level += 1;
        let result = self.do_match(s, p)?;
        if result.is_none() {
            self.level -= 1;
        }
        Ok(result)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> Result<Option<usize>> {
        let Some(l) = (0..self.level).rev().find(|&l| self.capture[l].1 == CAP_UNFINISHED) else {
            return Err(Error::runtime("invalid pattern capture"));
        };
        self.capture[l].1 = (s - self.capture[l].0) as isize;
        let result = self.do_match(s, p)?;
        if result.is_none() {
            self.capture[l].1 = CAP_UNFINISHED;
        }
        Ok(result)
    }

    fn match_capture(&self, s: usize, digit: u8) -> Result<Option<usize>> {
        let l = (digit as usize).wrapping_sub(b'1' as usize);
        if l >= self.level || self.capture[l].1 == CAP_UNFINISHED {
            return Err(Error::runtime(format!("invalid capture index %{}", l + 1)));
        }
        let (start, len) = (self.capture[l].0, self.capture[l].1.max(0) as usize);
        if self.src.len() - s >= len && self.src[start..start + len] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    /// Tries to match the pattern from `p` at subject position `s`. Returns
    /// the end of the match.
    pub fn do_match(&mut self, mut s: usize, mut p: usize) -> Result<Option<usize>> {
        self.depth += 1;
        if self.depth > MAX_RECURSION {
            return Err(Error::runtime("pattern too complex"));
        }
        let result = loop {
            if p >= self.pat.len() {
                break Some(s);
            }
            match self.pat[p] {
                b'(' => {
                    break if self.pat.get(p + 1) == Some(&b')') {
                        self.start_capture(s, p + 2, CAP_POSITION)?
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)?
                    };
                }
                b')' => break self.end_capture(s, p + 1)?,
                b'$' if p + 1 == self.pat.len() => break (s == self.src.len()).then_some(s),
                ESC if self.pat.get(p + 1) == Some(&b'b') => match self.match_balance(s, p + 2)? {
                    Some(end) => {
                        s = end;
                        p += 4;
                        continue;
                    }
                    None => break None,
                },
                ESC if self.pat.get(p + 1) == Some(&b'f') => {
                    p += 2;
                    if self.pat.get(p) != Some(&b'[') {
                        return Err(Error::runtime("missing '[' after '%f' in pattern"));
                    }
                    let ep = self.class_end(p)?;
                    let previous = if s == 0 { 0 } else { self.src[s - 1] };
                    let current = self.src.get(s).copied().unwrap_or(0);
                    if !self.set_matches(previous, p, ep - 1) && self.set_matches(current, p, ep - 1) {
                        p = ep;
                        continue;
                    }
                    break None;
                }
                ESC if self.pat.get(p + 1).is_some_and(u8::is_ascii_digit) => {
                    match self.match_capture(s, self.pat[p + 1])? {
                        Some(end) => {
                            s = end;
                            p += 2;
                            continue;
                        }
                        None => break None,
                    }
                }
                _ => {}
            }
            let ep = self.class_end(p)?;
            match self.pat.get(ep) {
                Some(b'?') => {
                    if self.single_matches(s, p, ep) {
                        if let Some(end) = self.do_match(s + 1, ep + 1)? {
                            break Some(end);
                        }
                    }
                    p = ep + 1;
                }
                Some(b'+') => {
                    break if self.single_matches(s, p, ep) { self.max_expand(s + 1, p, ep)? } else { None };
                }
                Some(b'*') => break self.max_expand(s, p, ep)?,
                Some(b'-') => break self.min_expand(s, p, ep)?,
                _ => {
                    if !self.single_matches(s, p, ep) {
                        break None;
                    }
                    s += 1;
                    p = ep;
                }
            }
        };
        self.depth -= 1;
        Ok(result)
    }

    fn capture_value(&self, i: usize, s: usize, e: usize) -> Result<Value> {
        if i >= self.level {
            if i == 0 {
                return Ok(Value::Str(LuaStr::new(&self.src[s..e])));
            }
            return Err(Error::runtime(format!("invalid capture index %{}", i + 1)));
        }
        let (start, len) = self.capture[i];
        match len {
            CAP_UNFINISHED => Err(Error::runtime("unfinished capture")),
            CAP_POSITION => Ok(Value::Int(start as i64 + 1)),
            len => Ok(Value::Str(LuaStr::new(&self.src[start..start + len as usize]))),
        }
    }

    /// The captures of a successful match spanning `s..e`. With no explicit
    /// captures, the whole match is returned when `whole_if_none` is set.
    pub fn captures(&self, s: usize, e: usize, whole_if_none: bool) -> Result<Vec<Value>> {
        let count = if self.level == 0 && whole_if_none { 1 } else { self.level };
        (0..count).map(|i| self.capture_value(i, s, e)).collect()
    }

    /// Capture `i` (0-based), or the whole match for `i == 0` without captures.
    pub fn capture(&self, i: usize, s: usize, e: usize) -> Result<Value> {
        self.capture_value(i, s, e)
    }
}

/// Whether a pattern contains no magic characters (so plain search suffices).
pub fn is_plain(pat: &[u8]) -> bool {
    !pat.iter().any(|c| b"^$*+?.([%-".contains(c))
}

/// Plain substring search.
pub fn find_plain(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Finds the first match of `pat` in `src` at or after `init`. Returns the
/// match bounds and its captures.
pub fn find(src: &[u8], pat: &[u8], init: usize) -> Result<Option<(usize, usize, Vec<Value>)>> {
    let (anchored, start) = match pat.first() {
        Some(b'^') => (true, 1),
        _ => (false, 0),
    };
    let mut matcher = Matcher::new(src, pat);
    let mut s = init;
    loop {
        matcher.reset();
        if let Some(e) = matcher.do_match(s, start)? {
            let captures = matcher.captures(s, e, false)?;
            return Ok(Some((s, e, captures)));
        }
        s += 1;
        if anchored || s > src.len() {
            return Ok(None);
        }
    }
}
