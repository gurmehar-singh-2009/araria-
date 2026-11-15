//! Text operations behind the script editor's keyboard behaviour:
//! automatic indentation, block indent and outdent, comment toggling,
//! bracket matching and search and replace.
//!
//! Everything works on byte offsets into the source and returns the new
//! text plus the new selection, so it can be tested without a UI.

use std::ops::Range;

use super::highlight::{TokenKind, tokenize};

/// One level of indentation.
pub const INDENT: &str = "    ";

/// The result of an edit: the new source and the selection afterwards
/// (`start == end` for a plain cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

/// Byte offset of the start of the line containing `at`.
pub fn line_start(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].rfind('\n').map_or(0, |i| i + 1)
}

/// Byte offset of the end of the line containing `at` (before its `\n`).
pub fn line_end(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// The leading whitespace of the line containing `at`.
pub fn indentation(text: &str, at: usize) -> &str {
    let start = line_start(text, at);
    let line = &text[start..line_end(text, at)];
    &line[..line.len() - line.trim_start().len()]
}

/// Zero-based line number of a byte offset.
pub fn line_of(text: &str, at: usize) -> usize {
    text.as_bytes()[..at.min(text.len())].iter().filter(|b| **b == b'\n').count()
}

/// Byte offset of the start of a zero-based line (clamped to the text).
pub fn offset_of_line(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n').nth(line - 1).map_or(text.len(), |(i, _)| i + 1)
}

/// Code tokens (no whitespace or comments) on the line `line`, as text.
fn line_words(text: &str, range: Range<usize>) -> Vec<(TokenKind, &str)> {
    tokenize(&text[range.clone()]).into_iter().filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment)).map(|t| (t.kind, &text[range.start + t.range.start..range.start + t.range.end])).collect()
}

/// Words that close a block when they start a line.
fn is_closer(word: &str) -> bool {
    matches!(word, "end" | "else" | "elseif" | "until" | "}" | ")" | "]")
}

/// How a line affects indentation: whether it starts by closing a block
/// (`end`, `else`, `}`, ...), and how many blocks the rest of it leaves
/// open (`then`, `do`, `repeat`, `function`, `{`, `(` open; `end`, `until`,
/// `}`, `)` close). `else` both closes the previous branch and opens one.
fn line_effect(line: &str) -> (bool, i32) {
    let words = line_words(line, 0..line.len());
    let leading_close = words.first().is_some_and(|(_, w)| is_closer(w));
    let mut net = 0;
    for (index, (kind, word)) in words.iter().enumerate() {
        if index == 0 && leading_close {
            if *word == "else" {
                net += 1;
            }
            continue;
        }
        match (kind, *word) {
            (TokenKind::Keyword, "then" | "do" | "repeat" | "function") | (TokenKind::Operator, "{" | "(" | "[") => net += 1,
            (TokenKind::Keyword, "end" | "until") | (TokenKind::Operator, "}" | ")" | "]") => net -= 1,
            _ => {}
        }
    }
    (leading_close, net)
}

fn starts_with_closer(line: &str) -> bool {
    line_effect(line).0
}

/// Replaces the selection with a newline and indents the new line to match
/// the code: one level deeper after a line that opens a block. Pressing
/// Enter between `then` and `end` (or `{` and `}`) puts the closer on its
/// own line too.
pub fn newline(text: &str, selection: Range<usize>) -> Edit {
    let (start, end) = (selection.start.min(selection.end), selection.start.max(selection.end));
    let base = indentation(text, start).to_string();
    let before = &text[line_start(text, start)..start];
    let after = &text[end..line_end(text, end)];
    let opens = line_effect(before).1 > 0;
    let mut inserted = String::from("\n");
    inserted.push_str(&base);
    if opens {
        inserted.push_str(INDENT);
    }
    let cursor = start + inserted.len();
    // Split `{|}`, `(|)` and `then|end` apart.
    let closer_follows = opens && starts_with_closer(after);
    if closer_follows {
        inserted.push('\n');
        inserted.push_str(&base);
    }
    let tail = if closer_follows { after.trim_start() } else { after };
    let mut out = String::with_capacity(text.len() + inserted.len());
    out.push_str(&text[..start]);
    out.push_str(&inserted);
    out.push_str(tail);
    out.push_str(&text[end + after.len()..]);
    Edit { text: out, selection: cursor..cursor }
}

/// After typing, moves a line that now consists of a block closer
/// (`end`, `else`, `elseif`, `until`, `}`) back to the indentation of the
/// line that opened the block. Returns `None` if nothing needs to change.
pub fn dedent_closer(text: &str, cursor: usize) -> Option<Edit> {
    let start = line_start(text, cursor);
    let line = &text[start..line_end(text, cursor)];
    let trimmed = line.trim();
    if !matches!(trimmed, "end" | "else" | "elseif" | "until" | "}" | ")" | "end)" | "end," | "})" | "}," | "]") {
        return None;
    }
    // Find the line that opened the innermost block still open above.
    let mut depth = 0;
    let mut target: Option<String> = None;
    let mut line_no = line_of(text, start);
    while line_no > 0 {
        line_no -= 1;
        let from = offset_of_line(text, line_no);
        let above = &text[from..line_end(text, from)];
        let (leading_close, net) = line_effect(above);
        if depth + net > 0 {
            target = Some(indentation(text, from).to_string());
            break;
        }
        depth += net - i32::from(leading_close);
    }
    let target = target?;
    let current = &line[..line.len() - line.trim_start().len()];
    if current == target {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push_str(&target);
    out.push_str(line.trim_start());
    out.push_str(&text[start + line.len()..]);
    let cursor = (cursor + target.len()).saturating_sub(current.len()).max(start + target.len());
    Some(Edit { text: out, selection: cursor..cursor })
}

/// The whole lines touched by a selection.
fn selected_lines(text: &str, selection: &Range<usize>) -> Range<usize> {
    let (start, mut end) = (selection.start.min(selection.end), selection.start.max(selection.end));
    // A selection ending at the very start of a line does not include it.
    if end > start && line_start(text, end) == end {
        end -= 1;
    }
    line_start(text, start)..line_end(text, end)
}

/// Applies `change` to every line of the selection and keeps the same lines
/// selected.
fn map_lines(text: &str, selection: &Range<usize>, change: impl Fn(&str) -> String) -> Edit {
    let lines = selected_lines(text, selection);
    let changed: Vec<String> = text[lines.clone()].split('\n').map(change).collect();
    let block = changed.join("\n");
    let mut out = String::with_capacity(text.len() + block.len());
    out.push_str(&text[..lines.start]);
    out.push_str(&block);
    out.push_str(&text[lines.end..]);
    Edit { selection: lines.start..lines.start + block.len(), text: out }
}

/// Tab: inserts one indentation level at the cursor, or indents every
/// selected line when the selection spans lines.
pub fn indent(text: &str, selection: Range<usize>) -> Edit {
    let (start, end) = (selection.start.min(selection.end), selection.start.max(selection.end));
    // A selection covering a whole line indents it, like a multi-line one.
    let whole_line = end > start && text[line_start(text, start)..start].trim().is_empty() && end == line_end(text, end);
    if !text[start..end].contains('\n') && !whole_line {
        // Align to the next multiple of the indent width.
        let column = start - line_start(text, start);
        let spaces = INDENT.len() - column % INDENT.len();
        let mut out = String::with_capacity(text.len() + spaces);
        out.push_str(&text[..start]);
        out.push_str(&" ".repeat(spaces));
        out.push_str(&text[end..]);
        let cursor = start + spaces;
        return Edit { text: out, selection: cursor..cursor };
    }
    map_lines(text, &selection, |line| if line.trim().is_empty() { line.to_string() } else { format!("{INDENT}{line}") })
}

/// Shift+Tab: removes one indentation level from every selected line (or
/// the cursor's line).
pub fn outdent(text: &str, selection: Range<usize>) -> Edit {
    let single = !text[selection.start.min(selection.end)..selection.start.max(selection.end)].contains('\n');
    let removed_on_first = {
        let line = &text[line_start(text, selection.start)..];
        line.len() - line.trim_start_matches(' ').len().max(line.len().saturating_sub(INDENT.len()))
    };
    let edit = map_lines(text, &selection, |line| {
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if line.starts_with('\t') { line[1..].to_string() } else { line[spaces.min(INDENT.len())..].to_string() }
    });
    if single {
        // Keep the cursor where it was relative to the code.
        let cursor = selection.end.saturating_sub(removed_on_first).max(line_start(text, selection.start));
        return Edit { text: edit.text, selection: cursor..cursor };
    }
    edit
}

/// Ctrl+/: comments out the selected lines with `-- `, or removes the
/// comment markers if every non-blank line already has one.
pub fn toggle_comment(text: &str, selection: Range<usize>) -> Edit {
    let lines = selected_lines(text, &selection);
    let block = &text[lines];
    let all_commented = block.split('\n').filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with("--"));
    let column = block.split('\n').filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let edit = map_lines(text, &selection, |line| {
        if line.trim().is_empty() {
            line.to_string()
        } else if all_commented {
            let at = line.find("--").unwrap_or(0);
            let rest = &line[at + 2..];
            format!("{}{}", &line[..at], rest.strip_prefix(' ').unwrap_or(rest))
        } else {
            format!("{}-- {}", &line[..column], &line[column..])
        }
    });
    if selection.start == selection.end {
        // A plain cursor stays a cursor, moved with the text it was in.
        let start = line_start(text, selection.start);
        let delta = edit.text.len() as isize - text.len() as isize;
        let cursor = (selection.start as isize + delta).max(start as isize) as usize;
        let cursor = cursor.min(line_end(&edit.text, start));
        return Edit { text: edit.text, selection: cursor..cursor };
    }
    edit
}

/// Finds the bracket matching the one just before or just after `cursor`.
/// Brackets inside strings and comments are ignored. Returns the byte
/// offsets of both brackets.
pub fn matching_bracket(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let brackets: Vec<(usize, u8)> = tokenize(text).into_iter().filter(|t| t.kind == TokenKind::Operator && t.range.len() == 1).map(|t| (t.range.start, text.as_bytes()[t.range.start])).filter(|(_, b)| b"()[]{}".contains(b)).collect();
    let at = brackets.iter().position(|(i, _)| *i + 1 == cursor).or_else(|| brackets.iter().position(|(i, _)| *i == cursor))?;
    let (position, byte) = brackets[at];
    let (open, close, forward) = match byte {
        b'(' => (b'(', b')', true),
        b'[' => (b'[', b']', true),
        b'{' => (b'{', b'}', true),
        b')' => (b'(', b')', false),
        b']' => (b'[', b']', false),
        _ => (b'{', b'}', false),
    };
    let mut depth = 0;
    let candidates: Box<dyn Iterator<Item = &(usize, u8)>> = if forward { Box::new(brackets[at..].iter()) } else { Box::new(brackets[..=at].iter().rev()) };
    for (i, b) in candidates {
        if *b == open {
            depth += if forward { 1 } else { -1 };
        } else if *b == close {
            depth += if forward { -1 } else { 1 };
        }
        if depth == 0 {
            return Some((position, *i));
        }
    }
    None
}

/// Search options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Every occurrence of the query, as byte ranges.
pub fn find_all(text: &str, query: &Query) -> Vec<Range<usize>> {
    if query.text.is_empty() {
        return Vec::new();
    }
    let (haystack, needle) = if query.match_case { (text.to_string(), query.text.clone()) } else { (text.to_lowercase(), query.text.to_lowercase()) };
    // Lowercasing can change lengths for some scripts; fall back to exact matching then.
    let (haystack, needle) = if haystack.len() == text.len() { (haystack, needle) } else { (text.to_string(), query.text.clone()) };
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = haystack[from..].find(&needle) {
        let start = from + i;
        let end = start + needle.len();
        let bounded = !query.whole_word || ((start == 0 || !is_word(bytes[start - 1])) && (end == bytes.len() || !is_word(bytes[end])));
        if bounded {
            found.push(start..end);
        }
        from = start + needle.len().max(1);
        while from < text.len() && !text.is_char_boundary(from) {
            from += 1;
        }
    }
    found
}

/// Replaces every occurrence. Returns the new text and how many were replaced.
pub fn replace_all(text: &str, query: &Query, replacement: &str) -> (String, usize) {
    let matches = find_all(text, query);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in &matches {
        out.push_str(&text[last..range.start]);
        out.push_str(replacement);
        last = range.end;
    }
    out.push_str(&text[last..]);
    (out, matches.len())
}

/// Converts between egui's character indices and byte offsets.
pub fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

pub fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices().nth(char_index).map_or(text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits `|` out of a test string as the cursor.
    fn at(text: &str) -> (String, usize) {
        let cursor = text.find('|').expect("cursor marker");
        (text.replacen('|', "", 1), cursor)
    }

    fn show(edit: &Edit) -> String {
        let mut text = edit.text.clone();
        text.insert(edit.selection.end, '|');
        text
    }

    #[test]
    fn enter_keeps_and_deepens_indentation() {
        let cases = [
            ("    local x = 1|", "    local x = 1\n    |"),
            ("if ready then|", "if ready then\n    |"),
            ("for i = 1, 10 do|", "for i = 1, 10 do\n    |"),
            ("part.Touched:Connect(function(hit)|", "part.Touched:Connect(function(hit)\n    |"),
            ("local t = {|}", "local t = {\n    |\n}"),
            ("if a then|end", "if a then\n    |\nend"),
            ("    else|", "    else\n        |"),
            ("x = f(a) -- then|", "x = f(a) -- then\n|"),
            ("local s = \"do\"|", "local s = \"do\"\n|"),
            ("end|", "end\n|"),
            ("local function go() return 1 end|", "local function go() return 1 end\n|"),
        ];
        for (input, expected) in cases {
            let (text, cursor) = at(input);
            assert_eq!(show(&newline(&text, cursor..cursor)), expected, "{input:?}");
        }
    }

    #[test]
    fn typing_a_closer_dedents_it() {
        let (text, cursor) = at("if a then\n    print(1)\n    end|");
        let edit = dedent_closer(&text, cursor).unwrap();
        assert_eq!(show(&edit), "if a then\n    print(1)\nend|");
        let (text, cursor) = at("while true do\n    if x then\n        y()\n        else|");
        assert_eq!(show(&dedent_closer(&text, cursor).unwrap()), "while true do\n    if x then\n        y()\n    else|");
        // Already right: no change.
        let (text, cursor) = at("if a then\n    b()\nend|");
        assert!(dedent_closer(&text, cursor).is_none());
        // Not a closer on its own.
        let (text, cursor) = at("if a then\n    sendit|");
        assert!(dedent_closer(&text, cursor).is_none());
    }

    #[test]
    fn indent_outdent_and_comments_work_on_lines() {
        let text = "a()\nb()\n\nc()";
        let all = 0..text.len();
        let indented = indent(text, all.clone());
        assert_eq!(indented.text, "    a()\n    b()\n\n    c()");
        assert_eq!(&indented.text[indented.selection.clone()], indented.text.as_str());
        assert_eq!(outdent(&indented.text, indented.selection.clone()).text, text);
        // Tab with no selection aligns to the indent width.
        assert_eq!(show(&indent("ab", 2..2)), "ab  |");
        // Shift+Tab on one line keeps the cursor on the code.
        assert_eq!(show(&outdent("        x = 1", 13..13)), "    x = 1|");

        let commented = toggle_comment("    a()\n    b()", 0..5);
        assert_eq!(commented.text, "    -- a()\n    b()");
        let both = toggle_comment(&commented.text, 0..commented.text.len());
        assert_eq!(both.text, "    -- -- a()\n    -- b()", "mixed lines get commented");
        assert_eq!(toggle_comment("-- a()\n-- b()", 0..3).text, "a()\n-- b()");
        assert_eq!(show(&toggle_comment("x = 1", 5..5)), "-- x = 1|", "a cursor stays a cursor");
        assert_eq!(show(&toggle_comment("-- x = 1", 8..8)), "x = 1|");
        // A whole selected line is indented rather than replaced.
        assert_eq!(indent("x = 1", 0..5).text, "    x = 1");
    }

    #[test]
    fn brackets_match_outside_strings_and_comments() {
        let text = "f(a[1], { \")\" }) -- (";
        assert_eq!(matching_bracket(text, 2), Some((1, 15)), "after the opener");
        assert_eq!(matching_bracket(text, 16), Some((15, 1)), "after the closer");
        assert_eq!(matching_bracket(text, 3), Some((3, 5)));
        assert_eq!(matching_bracket(text, 9), Some((8, 14)));
        assert_eq!(matching_bracket("(()", 1), None, "unbalanced");
        assert_eq!(matching_bracket("abc", 1), None);
    }

    #[test]
    fn search_and_replace() {
        let text = "local Part = part.Parent -- part";
        let query = Query { text: "part".into(), ..Default::default() };
        assert_eq!(find_all(text, &query).len(), 3, "case-insensitive by default");
        let exact = Query { text: "part".into(), match_case: true, whole_word: true };
        assert_eq!(find_all(text, &exact), vec![13..17, 28..32]);
        let (replaced, count) = replace_all(text, &exact, "brick");
        assert_eq!((replaced.as_str(), count), ("local Part = brick.Parent -- brick", 2));
        assert!(find_all(text, &Query::default()).is_empty());
        assert_eq!(find_all("ééé", &Query { text: "é".into(), ..Default::default() }).len(), 3);
        assert_eq!(byte_to_char("aé b", 4), 3);
        assert_eq!(char_to_byte("aé b", 3), 4);
    }

    #[test]
    fn line_helpers() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_of(text, 5), 1);
        assert_eq!(offset_of_line(text, 2), 8);
        assert_eq!(offset_of_line(text, 9), text.len());
        assert_eq!((line_start(text, 5), line_end(text, 5)), (4, 7));
    }
}
