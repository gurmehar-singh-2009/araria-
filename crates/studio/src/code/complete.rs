//! Autocomplete and signature hints for the script editor.
//!
//! Completion looks at the expression being typed (`workspace.Do`,
//! `part:Get`, `Vector3.n`, `pri`) and offers matching globals, library
//! members, instance members and names used elsewhere in the file. Local
//! variables assigned from `Instance.new("Class")` or a service remember
//! their class, so their members are completed precisely.

use std::collections::{BTreeSet, HashMap};

use araria_core::schema::{ClassInfo, schema};

use super::api::{self, Kind};
use super::highlight::{TokenKind, tokenize};

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub kind: Kind,
    /// Signature or type, shown dimmed next to the label.
    pub detail: String,
    pub doc: String,
}

/// The suggestions for a cursor position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    /// Byte offset where the word being completed starts.
    pub start: usize,
    pub items: Vec<Suggestion>,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// What the receiver of a `.`/`:` access is known to be.
enum Receiver {
    Library(&'static api::Library),
    Class(&'static ClassInfo),
    Unknown,
}

/// Guesses the classes of local variables from simple assignments.
fn variable_classes(source: &str) -> HashMap<&str, &'static ClassInfo> {
    let tokens: Vec<_> = tokenize(source).into_iter().filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment)).collect();
    let text = |i: usize| tokens.get(i).map(|t| &source[t.range.clone()]);
    let mut classes = HashMap::new();
    for i in 0..tokens.len() {
        if text(i + 1) != Some("=") || !matches!(tokens[i].kind, TokenKind::Identifier) {
            continue;
        }
        let name = &source[tokens[i].range.clone()];
        // name = Instance.new("Class"
        if text(i + 2) == Some("Instance") && text(i + 4) == Some("new") && text(i + 5) == Some("(") {
            if let Some(class) = text(i + 6).and_then(|s| schema().find(s.trim_matches(['"', '\'']))) {
                classes.insert(name, class);
            }
        } else if let Some(class) = text(i + 2).and_then(api::global_class) {
            // name = workspace   (but not name = workspace.Something)
            if !matches!(text(i + 3), Some("." | ":")) {
                classes.insert(name, class);
            } else if text(i + 2) == Some("Players") && text(i + 4) == Some("LocalPlayer") && !matches!(text(i + 5), Some("." | ":")) {
                classes.insert(name, schema().find("Player").expect("Player class"));
            }
        }
    }
    classes
}

fn receiver(source: &str, name: &str, chained: bool) -> Receiver {
    if chained {
        // `a.b.` — the type of `a.b` is not tracked.
        return Receiver::Unknown;
    }
    if let Some(class) = variable_classes(source).get(name) {
        return Receiver::Class(class);
    }
    if let Some(class) = api::global_class(name) {
        return Receiver::Class(class);
    }
    match api::library(name) {
        Some(lib) => Receiver::Library(lib),
        None => Receiver::Unknown,
    }
}

fn class_suggestions(class: &ClassInfo, methods_only: bool) -> Vec<Suggestion> {
    api::class_members(class)
        .into_iter()
        .filter(|m| !methods_only || m.kind == Kind::Method)
        .map(|m| Suggestion { label: m.name.to_string(), kind: m.kind, detail: m.signature, doc: m.doc.to_string() })
        .collect()
}

fn entry_suggestions(entries: &[api::Entry], methods_only: bool) -> Vec<Suggestion> {
    entries
        .iter()
        .filter(|e| !methods_only || !e.signature.is_empty())
        .map(|e| Suggestion {
            label: e.name.to_string(),
            kind: if e.signature.is_empty() { Kind::Value } else { Kind::Function },
            detail: e.signature.trim_start_matches(':').to_string(),
            doc: e.doc.to_string(),
        })
        .collect()
}

/// Members of every class and value type, for receivers of unknown type.
fn any_member_suggestions(methods_only: bool) -> Vec<Suggestion> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for class in schema().classes() {
        for item in class_suggestions(class, methods_only) {
            if seen.insert(item.label.clone()) {
                out.push(item);
            }
        }
    }
    for lib in api::LIBRARIES {
        for item in entry_suggestions(lib.instance_members, methods_only) {
            if seen.insert(item.label.clone()) {
                out.push(item);
            }
        }
    }
    out
}

/// Ranks candidates: prefix matches first, then matches elsewhere in the
/// name; shorter names win ties.
fn filter(mut items: Vec<Suggestion>, prefix: &str) -> Vec<Suggestion> {
    let needle = prefix.to_lowercase();
    items.retain(|s| s.label != prefix && s.label.to_lowercase().contains(&needle));
    items.sort_by_key(|s| {
        let label = s.label.to_lowercase();
        (!s.label.starts_with(prefix), !label.starts_with(&needle), s.label.len(), s.label.clone())
    });
    items.truncate(40);
    items
}

/// Computes suggestions for the cursor at byte offset `cursor`.
pub fn complete(source: &str, cursor: usize) -> Completion {
    let bytes = source.as_bytes();
    let cursor = cursor.min(bytes.len());
    // Nothing is offered inside comments and ordinary strings.
    let token = tokenize(source).into_iter().find(|t| t.range.start < cursor && cursor <= t.range.end);
    let mut start = cursor;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let prefix = &source[start..cursor];
    if let Some(token) = &token {
        match token.kind {
            TokenKind::Comment | TokenKind::Number => return Completion::default(),
            TokenKind::String => {
                // Inside Instance.new(" offer class names.
                let before = source[..token.range.start].trim_end();
                if before.ends_with("Instance.new(") {
                    let typed = &source[token.range.start + 1..cursor];
                    let items = schema().classes().filter(|c| c.def.creatable).map(|c| Suggestion { label: c.name().to_string(), kind: Kind::Class, detail: String::new(), doc: c.def.doc.to_string() }).collect();
                    return Completion { start: token.range.start + 1, items: filter(items, typed) };
                }
                return Completion::default();
            }
            _ => {}
        }
    }

    let separator = (start > 0).then(|| bytes[start - 1]).filter(|b| matches!(b, b'.' | b':'));
    let Some(separator) = separator else {
        if prefix.is_empty() {
            return Completion::default();
        }
        let mut items: Vec<Suggestion> = api::KEYWORDS.iter().map(|k| Suggestion { label: (*k).to_string(), kind: Kind::Keyword, detail: String::new(), doc: String::new() }).collect();
        items.extend(api::GLOBALS.iter().map(|g| Suggestion { label: g.name.to_string(), kind: if g.signature.is_empty() { Kind::Global } else { Kind::Function }, detail: g.signature.to_string(), doc: g.doc.to_string() }));
        items.extend(api::LIBRARIES.iter().map(|l| Suggestion { label: l.name.to_string(), kind: Kind::Library, detail: String::new(), doc: l.doc.to_string() }));
        // Names used elsewhere in the file.
        let known: BTreeSet<String> = items.iter().map(|s| s.label.clone()).collect();
        let mut locals = BTreeSet::new();
        for token in tokenize(source) {
            if matches!(token.kind, TokenKind::Identifier | TokenKind::Call) && !(token.range.start <= cursor && cursor <= token.range.end) {
                locals.insert(&source[token.range]);
            }
        }
        items.extend(locals.into_iter().filter(|l| !known.contains(*l)).map(|l| Suggestion { label: l.to_string(), kind: Kind::Local, detail: String::new(), doc: String::new() }));
        return Completion { start, items: filter(items, prefix) };
    };

    // `receiver.prefix` or `receiver:prefix`.
    let name_end = start - 1;
    let mut name_start = name_end;
    while name_start > 0 && is_word(bytes[name_start - 1]) {
        name_start -= 1;
    }
    let name = &source[name_start..name_end];
    let chained = name_start > 0 && matches!(bytes[name_start - 1], b'.' | b':' | b')' | b']');
    let methods_only = separator == b':';
    // Enum.Material.<option>
    if !methods_only && name_start >= 5 && &source[name_start - 5..name_start] == "Enum." {
        let options: &[&str] = match name {
            "Material" => araria_core::schema::MATERIALS,
            "Shape" | "PartType" => araria_core::schema::SHAPES,
            "CameraMode" => araria_core::schema::CAMERA_MODES,
            "TextAlign" => araria_core::schema::TEXT_ALIGN,
            "CharacterState" => araria_core::schema::CHARACTER_STATES,
            "EasingDirection" => &["In", "Out", "InOut"],
            _ => &[],
        };
        let items = options.iter().map(|o| Suggestion { label: (*o).to_string(), kind: Kind::Value, detail: String::new(), doc: String::new() }).collect();
        return Completion { start, items: filter(items, prefix) };
    }
    let items = if name.is_empty() {
        any_member_suggestions(methods_only)
    } else {
        match receiver(source, name, chained) {
            Receiver::Library(lib) => entry_suggestions(lib.members, false),
            Receiver::Class(class) => class_suggestions(class, methods_only),
            Receiver::Unknown => any_member_suggestions(methods_only),
        }
    };
    Completion { start, items: filter(items, prefix) }
}

/// A call's signature, shown while typing its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureHint {
    pub name: String,
    pub signature: String,
    pub doc: String,
}

/// Finds the call the cursor is inside and describes the function.
pub fn signature_hint(source: &str, cursor: usize) -> Option<SignatureHint> {
    let bytes = source.as_bytes();
    let cursor = cursor.min(bytes.len());
    // Walk back to the unmatched `(` on this statement.
    let mut depth = 0;
    let mut i = cursor;
    let open = loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' if depth == 0 => break i,
            b'(' => depth -= 1,
            b'\n' if depth == 0 => return None,
            _ => {}
        }
    };
    let mut start = open;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let function = &source[start..open];
    if function.is_empty() {
        return None;
    }
    let separator = (start > 0).then(|| bytes[start - 1]).filter(|b| matches!(b, b'.' | b':'));
    let hint = |name: String, signature: &str, doc: &str| Some(SignatureHint { name, signature: signature.trim_start_matches(':').to_string(), doc: doc.to_string() });
    let Some(_) = separator else {
        let global = api::GLOBALS.iter().find(|g| g.name == function && !g.signature.is_empty())?;
        return hint(function.to_string(), global.signature, global.doc);
    };
    let name_end = start - 1;
    let mut name_start = name_end;
    while name_start > 0 && is_word(bytes[name_start - 1]) {
        name_start -= 1;
    }
    let name = &source[name_start..name_end];
    let chained = name_start > 0 && matches!(bytes[name_start - 1], b'.' | b':' | b')' | b']');
    if !chained {
        if let Some(entry) = api::library(name).and_then(|lib| lib.members.iter().find(|m| m.name == function)) {
            return hint(format!("{name}.{function}"), entry.signature, entry.doc);
        }
    }
    let class = match receiver(source, name, chained) {
        Receiver::Class(class) => Some(class),
        _ => None,
    };
    let classes: Vec<&ClassInfo> = match class {
        Some(class) => vec![class],
        None => schema().classes().collect(),
    };
    for class in classes {
        if let Some(method) = class.all_methods().find(|m| m.name == function) {
            return hint(format!("{}:{function}", class.name()), method.signature, method.doc);
        }
    }
    let entry = api::LIBRARIES.iter().flat_map(|l| l.instance_members.iter()).find(|m| m.name == function && !m.signature.is_empty())?;
    hint(function.to_string(), entry.signature, entry.doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(source: &str) -> Vec<String> {
        complete(source, source.len()).items.into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn completes_globals_keywords_and_locals() {
        assert_eq!(labels("pri")[0], "print");
        assert!(labels("wor").starts_with(&["workspace".to_string()]));
        assert!(labels("fun").contains(&"function".to_string()));
        assert_eq!(labels("local doorway = 1\ndoo"), vec!["doorway"]);
        assert!(labels("-- pri").is_empty(), "nothing inside comments");
        assert!(labels("x = \"pri").is_empty(), "nothing inside strings");
        assert!(labels("x = 12").is_empty());
    }

    #[test]
    fn completes_library_and_class_members() {
        assert_eq!(labels("Vector3.n")[0], "new");
        assert!(labels("task.").contains(&"wait".to_string()));
        let workspace = labels("workspace:");
        assert!(workspace.contains(&"Raycast".to_string()) && !workspace.contains(&"Gravity".to_string()), "':' offers only methods");
        assert!(labels("workspace.Gra").starts_with(&["Gravity".to_string()]));
        assert!(labels("Enum.Material.Ne").starts_with(&["Neon".to_string()]));
        assert!(labels("Instance.new(\"Par").starts_with(&["Part".to_string()]));
    }

    #[test]
    fn remembers_what_variables_hold() {
        let source = "local door = Instance.new(\"Part\", workspace)\nlocal me = Players.LocalPlayer\ndoor.";
        let door = labels(source);
        assert!(door.contains(&"Anchored".to_string()) && door.contains(&"Touched".to_string()));
        assert!(!door.contains(&"WalkSpeed".to_string()), "a Part has no character properties");
        assert!(labels("local me = Players.LocalPlayer\nme:").contains(&"LoadCharacter".to_string()));
        // Unknown receivers fall back to every known member.
        assert!(labels("local x = something()\nx.Walk").starts_with(&["WalkSpeed".to_string()]));
    }

    #[test]
    fn completion_replaces_the_typed_word() {
        let source = "local p = workspace.Gra";
        let completion = complete(source, source.len());
        assert_eq!(&source[completion.start..], "Gra");
    }

    #[test]
    fn signature_hints() {
        let hint = |source: &str| signature_hint(source, source.len()).map(|h| (h.name, h.signature));
        assert_eq!(hint("task.wait("), Some(("task.wait".into(), "(seconds: number?): number".into())));
        assert_eq!(hint("print(1, "), Some(("print".into(), "(...: any)".into())));
        assert_eq!(hint("workspace:Raycast(origin, Vector3.new(0, -1, 0), ").map(|h| h.0), Some("Workspace:Raycast".into()));
        assert_eq!(hint("x:FindFirstChild(").map(|h| h.0), Some("Instance:FindFirstChild".into()));
        assert_eq!(hint("Audio:Play(").map(|h| h.1), Some("(asset: string, volume: number?, pitch: number?)".into()));
        assert_eq!(hint("local x = 5"), None);
        assert_eq!(hint("foo(1)\nbar"), None);
    }
}
