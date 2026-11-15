//! The integrated terminal: script output (with levels, sources and stack
//! traces), commands that manage the running game, and interactive Lua
//! evaluated in the game's own context.

use std::collections::VecDeque;

use araria_core::{DataModel, InstanceId, Variant, quat_to_euler_deg};
use araria_platform::{icons, theme};
use araria_runtime::{LogEntry, LogLevel, Mode, Runtime};
use egui::{Color32, Key, RichText, Ui};

const MAX_LINES: usize = 3000;
const MAX_HISTORY: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// What the user typed.
    Input,
    /// The result of a command or expression.
    Result,
    Info,
    Warn,
    Error,
    /// Messages from the studio itself (saved, reloaded, ...).
    System,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub kind: LineKind,
    pub text: String,
    /// The script that printed it.
    pub source: Option<String>,
    /// Where an error happened: project path and one-based line.
    pub location: Option<(String, u32)>,
    /// Seconds of game time.
    pub time: f64,
}

/// A request from a terminal command that the studio carries out.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Play,
    Pause,
    Resume,
    Stop,
    Select(InstanceId),
    /// Open a file in the editor at a (one-based) line.
    Open(String, u32),
    Save,
}

/// Which output levels are shown.
#[derive(Clone, Copy, Debug)]
pub struct Filter {
    pub info: bool,
    pub warn: bool,
    pub error: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Filter { info: true, warn: true, error: true }
    }
}

impl Filter {
    fn shows(self, kind: LineKind) -> bool {
        match kind {
            LineKind::Info => self.info,
            LineKind::Warn => self.warn,
            LineKind::Error => self.error,
            _ => true,
        }
    }
}

pub struct Terminal {
    pub lines: VecDeque<Line>,
    input: String,
    history: Vec<String>,
    /// Position while browsing history with the arrow keys.
    browsing: Option<usize>,
    pub filter: Filter,
    text_filter: String,
    focus: bool,
    scroll_to_end: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        let mut terminal = Terminal { lines: VecDeque::new(), input: String::new(), history: Vec::new(), browsing: None, filter: Filter::default(), text_filter: String::new(), focus: false, scroll_to_end: true };
        terminal.system("Araria terminal — type `help` for commands, or any Lua to run it in the game.");
        terminal
    }
}

pub const HELP: &[(&str, &str)] = &[
    ("help", "List commands"),
    ("clear", "Clear the output"),
    ("play / pause / resume / stop", "Control the game"),
    ("scripts", "List scripts and whether they are running"),
    ("reload [script]", "Hot-reload one script (by name or path) or all of them"),
    ("start <script> / stop <script>", "Start or stop one script while playing"),
    ("inspect [object]", "Show an object's properties (default: the selection)"),
    ("select <object>", "Select an object, e.g. `select Workspace.Door`"),
    ("find <name>", "Find objects whose name contains the text"),
    ("scene", "Summarize the scene"),
    ("tree [object] [depth]", "Print the hierarchy below an object"),
    ("players", "List players and their characters"),
    ("open <file> [line]", "Open a script in the editor"),
    ("save", "Save the scene and all scripts"),
    ("stats", "Scheduler and memory statistics"),
    ("history", "Previously entered commands"),
    ("= <expression>", "Evaluate Lua explicitly (anything unrecognised is Lua too)"),
];

/// A readable description of a property value.
pub fn describe(dm: &DataModel, value: &Variant) -> String {
    match value {
        Variant::Nil => "nil".into(),
        Variant::Bool(b) => b.to_string(),
        Variant::Number(n) => format_number(*n),
        Variant::String(s) => format!("\"{s}\""),
        Variant::Vec2(v) => format!("({}, {})", format_number(f64::from(v.x)), format_number(f64::from(v.y))),
        Variant::Vec3(v) => format!("({}, {}, {})", format_number(f64::from(v.x)), format_number(f64::from(v.y)), format_number(f64::from(v.z))),
        Variant::Quat(q) => {
            let e = quat_to_euler_deg(*q);
            format!("({}°, {}°, {}°)", format_number(f64::from(e.x)), format_number(f64::from(e.y)), format_number(f64::from(e.z)))
        }
        Variant::Color(c) => c.to_hex(),
        Variant::UDim2(u) => format!("{{{}, {}}}, {{{}, {}}}", format_number(f64::from(u.x_scale)), format_number(f64::from(u.x_offset)), format_number(f64::from(u.y_scale)), format_number(f64::from(u.y_offset))),
        Variant::Ref(None) => "nil".into(),
        Variant::Ref(Some(id)) => if dm.contains(*id) { dm.path(*id) } else { "(destroyed)".into() },
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        let text = format!("{n:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Resolves what the user typed as an object: a path such as
/// `Workspace.Door` or `workspace.Door.Handle`, case-insensitively for the
/// first segment.
pub fn resolve(dm: &DataModel, path: &str) -> Option<InstanceId> {
    let path = path.trim().trim_start_matches("game.");
    if let Some(id) = dm.find_by_path(path) {
        return Some(id);
    }
    let (first, rest) = path.split_once('.').unwrap_or((path, ""));
    let service = dm.children(dm.root()).iter().copied().find(|c| dm.get(*c).is_some_and(|i| i.name.eq_ignore_ascii_case(first)))?;
    let mut current = service;
    for name in rest.split('.').filter(|s| !s.is_empty()) {
        current = dm.find_first_child(current, name)?;
    }
    Some(current)
}

/// Finds `path:line` references (as in error messages and tracebacks).
pub fn locations(text: &str) -> Vec<(String, u32)> {
    let mut found = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '\'' || c == '"') {
        let mut parts = word.split(':');
        let (Some(path), Some(line)) = (parts.next(), parts.next()) else { continue };
        if !path.ends_with(".lua") {
            continue;
        }
        if let Ok(line) = line.parse::<u32>() {
            if !found.iter().any(|(p, l)| p == path && *l == line) {
                found.push((path.to_string(), line));
            }
        }
    }
    found
}

impl Terminal {
    fn push(&mut self, line: Line) {
        if self.lines.len() >= MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.scroll_to_end = true;
    }

    fn write(&mut self, kind: LineKind, text: impl Into<String>) {
        self.push(Line { kind, text: text.into(), source: None, location: None, time: 0.0 });
    }

    /// A message from the studio.
    pub fn system(&mut self, text: impl Into<String>) {
        self.write(LineKind::System, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.write(LineKind::Error, text);
    }

    /// Adds script output.
    pub fn absorb(&mut self, entries: Vec<LogEntry>) {
        for entry in entries {
            let kind = match entry.level {
                LogLevel::Info => LineKind::Info,
                LogLevel::Warn => LineKind::Warn,
                LogLevel::Error => LineKind::Error,
            };
            let location = entry.location.clone().or_else(|| if kind == LineKind::Error { locations(&entry.text).into_iter().next() } else { None });
            self.push(Line { kind, text: entry.text, source: entry.source, location, time: entry.time });
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn counts(&self) -> (usize, usize) {
        let warnings = self.lines.iter().filter(|l| l.kind == LineKind::Warn).count();
        let errors = self.lines.iter().filter(|l| l.kind == LineKind::Error).count();
        (warnings, errors)
    }

    /// Runs one command line. `selection` is the studio's current
    /// selection (used by `inspect` with no argument).
    #[allow(clippy::too_many_lines)]
    pub fn execute(&mut self, command: &str, runtime: &mut Runtime, selection: Option<InstanceId>) -> Vec<Request> {
        let command = command.trim();
        let mut requests = Vec::new();
        if command.is_empty() {
            return requests;
        }
        // Output the game produced before this command comes first.
        let pending = runtime.scripts.take_log();
        self.absorb(pending);
        self.write(LineKind::Input, command);
        if self.history.last().is_none_or(|last| last != command) {
            self.history.push(command.to_string());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        let (name, rest) = command.split_once(char::is_whitespace).map_or((command, ""), |(n, r)| (n, r.trim()));
        let playing = runtime.mode() != Mode::Edit;
        match name {
            "help" | "?" => {
                for (usage, about) in HELP {
                    self.write(LineKind::Result, format!("{usage:<34}{about}"));
                }
            }
            "clear" | "cls" => self.clear(),
            "play" => requests.push(Request::Play),
            "pause" => requests.push(Request::Pause),
            "resume" => requests.push(Request::Resume),
            "stop" if rest.is_empty() => requests.push(Request::Stop),
            "save" => requests.push(Request::Save),
            "scripts" => {
                let scripts = runtime.scripts.scripts();
                if scripts.is_empty() {
                    let dm = runtime.dm.borrow();
                    let defined: Vec<String> = dm.instances().filter(|i| i.class_name() == "Script").map(|i| format!("  {}  →  {}", dm.path(i.id), i.str("Source"))).collect();
                    if defined.is_empty() {
                        self.write(LineKind::Result, "The game has no scripts.");
                    } else {
                        self.write(LineKind::Result, "Scripts (they run when you press Play):");
                        for line in defined {
                            self.write(LineKind::Result, line);
                        }
                    }
                } else {
                    for script in scripts {
                        let state = if let Some(error) = &script.error { format!("error: {error}") } else if script.running { "running".into() } else { "stopped".into() };
                        self.write(if script.error.is_some() { LineKind::Error } else { LineKind::Result }, format!("  {:<20} {:<28} {state}", script.name, script.path));
                    }
                }
            }
            "reload" => {
                if !playing {
                    self.write(LineKind::Result, "Nothing is running; scripts load fresh when you press Play.");
                } else if rest.is_empty() {
                    let count = runtime.scripts.reload_all();
                    self.write(LineKind::Result, format!("Reloaded {count} script{}.", if count == 1 { "" } else { "s" }));
                } else {
                    let path = runtime.scripts.scripts().into_iter().find(|s| s.name == rest || s.path == rest).map_or_else(|| rest.to_string(), |s| s.path);
                    match runtime.scripts.reload(&path) {
                        Ok(0) => self.write(LineKind::Warn, format!("No running script uses '{path}'.")),
                        Ok(count) => self.write(LineKind::Result, format!("Reloaded {path} ({count} script{}).", if count == 1 { "" } else { "s" })),
                        Err(error) => self.write(LineKind::Error, error),
                    }
                }
            }
            "start" | "stop" => {
                if !playing {
                    self.write(LineKind::Warn, "Scripts can only be started and stopped while the game is playing.");
                } else {
                    let count = if name == "start" { runtime.scripts.start(rest) } else { runtime.scripts.stop(rest) };
                    let verb = if name == "start" { "Started" } else { "Stopped" };
                    if count == 0 {
                        self.write(LineKind::Warn, format!("No {} script called '{rest}'.", if name == "start" { "stopped" } else { "running" }));
                    } else {
                        self.write(LineKind::Result, format!("{verb} {count} script{}.", if count == 1 { "" } else { "s" }));
                    }
                }
            }
            "inspect" | "i" => {
                let dm = runtime.dm.borrow();
                let target = if rest.is_empty() { selection } else { resolve(&dm, rest) };
                match target.and_then(|id| dm.get(id)) {
                    None if rest.is_empty() => self.write(LineKind::Warn, "Nothing is selected. Use `inspect Workspace.Name`."),
                    None => self.write(LineKind::Error, format!("No object at '{rest}'.")),
                    Some(instance) => {
                        let mut out = vec![format!("{} ({})  id {}", dm.path(instance.id), instance.class_name(), instance.id.0)];
                        for (index, def) in instance.info().props.iter().enumerate() {
                            if def.hidden && !def.transient {
                                continue;
                            }
                            let value = instance.prop_at(index);
                            if def.transient && *value == def.default {
                                continue;
                            }
                            out.push(format!("  {:<22}{}", def.name, describe(&dm, value)));
                        }
                        for (key, value) in &instance.attributes {
                            out.push(format!("  @{key:<21}{}", describe(&dm, value)));
                        }
                        if !instance.tags.is_empty() {
                            out.push(format!("  tags                  {}", instance.tags.join(", ")));
                        }
                        let children = dm.children(instance.id);
                        if !children.is_empty() {
                            let names: Vec<&str> = children.iter().filter_map(|c| dm.get(*c)).map(|c| c.name.as_str()).take(12).collect();
                            out.push(format!("  children ({})          {}{}", children.len(), names.join(", "), if children.len() > 12 { ", …" } else { "" }));
                        }
                        drop(dm);
                        for line in out {
                            self.write(LineKind::Result, line);
                        }
                    }
                }
            }
            "select" => {
                let found = resolve(&runtime.dm.borrow(), rest);
                match found {
                    Some(id) => requests.push(Request::Select(id)),
                    None => self.write(LineKind::Error, format!("No object at '{rest}'.")),
                }
            }
            "find" => {
                if rest.is_empty() {
                    self.write(LineKind::Warn, "Usage: find <name>");
                } else {
                    let dm = runtime.dm.borrow();
                    let needle = rest.to_lowercase();
                    let found: Vec<String> = dm.descendants(dm.root()).into_iter().filter_map(|id| dm.get(id)).filter(|i| i.name.to_lowercase().contains(&needle)).map(|i| format!("  {}  ({})", dm.path(i.id), i.class_name())).collect();
                    drop(dm);
                    if found.is_empty() {
                        self.write(LineKind::Result, format!("Nothing named like '{rest}'."));
                    }
                    let total = found.len();
                    for line in found.into_iter().take(50) {
                        self.write(LineKind::Result, line);
                    }
                    if total > 50 {
                        self.write(LineKind::Result, format!("  … and {} more", total - 50));
                    }
                }
            }
            "scene" => {
                let dm = runtime.dm.borrow();
                let mut classes: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
                for instance in dm.instances() {
                    *classes.entry(instance.class_name()).or_default() += 1;
                }
                let mode = match runtime.mode() {
                    Mode::Edit => "editing",
                    Mode::Play => "playing",
                    Mode::Paused => "paused",
                };
                let mut out = vec![format!("{} objects · {mode} · {:.1}s of play time", dm.len(), runtime.time())];
                for service in dm.children(dm.root()) {
                    if let Some(s) = dm.get(*service) {
                        out.push(format!("  {:<12}{} descendants", s.name, dm.descendants(*service).len()));
                    }
                }
                let summary: Vec<String> = classes.iter().filter(|(c, _)| !matches!(**c, "Game")).map(|(c, n)| format!("{c} {n}")).collect();
                out.push(format!("  {}", summary.join(" · ")));
                drop(dm);
                for line in out {
                    self.write(LineKind::Result, line);
                }
            }
            "tree" => {
                let mut parts = rest.split_whitespace();
                let dm = runtime.dm.borrow();
                let (start, depth) = match (parts.next(), parts.next()) {
                    (Some(a), b) if a.parse::<usize>().is_err() => (resolve(&dm, a), b.and_then(|d| d.parse().ok()).unwrap_or(3)),
                    (Some(a), _) => (Some(dm.workspace()), a.parse().unwrap_or(3)),
                    (None, _) => (Some(dm.workspace()), 2),
                };
                let Some(start) = start else {
                    drop(dm);
                    self.write(LineKind::Error, format!("No object at '{rest}'."));
                    return requests;
                };
                let mut out = Vec::new();
                fn walk(dm: &DataModel, id: InstanceId, level: usize, depth: usize, out: &mut Vec<String>) {
                    let Some(instance) = dm.get(id) else { return };
                    if out.len() > 300 {
                        return;
                    }
                    out.push(format!("{}{} ({})", "  ".repeat(level), instance.name, instance.class_name()));
                    if level < depth {
                        for child in dm.children(id) {
                            walk(dm, *child, level + 1, depth, out);
                        }
                    } else if !dm.children(id).is_empty() {
                        out.push(format!("{}…", "  ".repeat(level + 1)));
                    }
                }
                walk(&dm, start, 0, depth, &mut out);
                drop(dm);
                for line in out {
                    self.write(LineKind::Result, line);
                }
            }
            "players" => {
                let dm = runtime.dm.borrow();
                let players = dm.children(dm.service("Players")).to_vec();
                let mut out = Vec::new();
                if players.is_empty() {
                    out.push(if playing { "No players.".to_string() } else { "No players while editing — press Play to join.".to_string() });
                }
                for player in players {
                    let Some(p) = dm.get(player) else { continue };
                    let character = p.reference("Character").and_then(|c| dm.get(c));
                    let detail = character.map_or("no character".to_string(), |c| format!("health {} at {}", format_number(c.number("Health")), describe(&dm, &Variant::Vec3(c.vec3("Position")))));
                    out.push(format!("  {} (user {}{})  {detail}", p.name, p.number("UserId"), if p.bool("IsLocal") { ", this device" } else { "" }));
                }
                drop(dm);
                for line in out {
                    self.write(LineKind::Result, line);
                }
            }
            "open" => {
                let mut parts = rest.split_whitespace();
                match parts.next() {
                    Some(path) => {
                        let path = if path.contains('/') { path.to_string() } else { format!("scripts/{path}") };
                        let path = if path.ends_with(".lua") { path } else { format!("{path}.lua") };
                        requests.push(Request::Open(path, parts.next().and_then(|l| l.parse().ok()).unwrap_or(1)));
                    }
                    None => self.write(LineKind::Warn, "Usage: open <file> [line]"),
                }
            }
            "stats" => {
                let (sleeping, connections, tweens) = runtime.scripts.stats();
                let gc = runtime.scripts.collect_garbage();
                self.write(LineKind::Result, format!("{} scripts · {connections} event connections · {sleeping} waiting threads · {tweens} tweens", runtime.scripts.scripts().len()));
                self.write(LineKind::Result, format!("Lua heap: {gc:?}"));
                self.write(LineKind::Result, format!("{} physics bodies · {} objects", runtime.physics.borrow().body_count(), runtime.dm.borrow().len()));
            }
            "history" => {
                let history: Vec<String> = self.history.iter().rev().skip(1).take(30).rev().cloned().collect();
                for (i, entry) in history.iter().enumerate() {
                    self.write(LineKind::Result, format!("  {:>3}  {entry}", i + 1));
                }
            }
            _ => {
                let code = command.strip_prefix('=').unwrap_or(command);
                let single_word = code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                match runtime.scripts.eval(code) {
                    Ok(result) if result.is_empty() => {}
                    Ok(result) if result == "nil" && single_word => self.write(LineKind::Warn, format!("'{code}' is not a command (and is nil in Lua). Type `help` for commands.")),
                    Ok(result) => self.write(LineKind::Result, result),
                    Err(error) => {
                        let location = locations(&error).into_iter().find(|(p, _)| p != "console");
                        self.push(Line { kind: LineKind::Error, text: error, source: Some("console".into()), location, time: runtime.time() });
                    }
                }
                // Output printed by the code appears right after it.
                let log = runtime.scripts.take_log();
                self.absorb(log);
            }
        }
        requests
    }

    /// Draws the terminal. Returns requests from commands typed this frame
    /// and from clicking an error location.
    pub fn show(&mut self, ui: &mut Ui, runtime: &mut Runtime, selection: Option<InstanceId>) -> Vec<Request> {
        let mut requests = Vec::new();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (warnings, errors) = self.counts();
            ui.toggle_value(&mut self.filter.info, RichText::new(format!("{} Output", icons::INFO)).font(theme::regular(12.0)));
            ui.toggle_value(&mut self.filter.warn, RichText::new(format!("{} {warnings}", icons::WARNING)).font(theme::regular(12.0)).color(theme::WARNING));
            ui.toggle_value(&mut self.filter.error, RichText::new(format!("{} {errors}", icons::BUG)).font(theme::regular(12.0)).color(theme::DANGER));
            ui.add(egui::TextEdit::singleline(&mut self.text_filter).hint_text("Filter output").desired_width(140.0).font(theme::regular(12.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(RichText::new(format!("{} Clear", icons::TRASH)).font(theme::regular(12.0))).clicked() {
                    self.clear();
                }
            });
        });
        let input_height = 30.0;
        // Leave exactly enough room below for the frame margins, spacing and
        // the command line, or the panel would grow every frame.
        let spacing = ui.spacing().item_spacing.y;
        let output_height = (ui.available_height() - input_height - 8.0 - 4.0 - spacing * 3.0 - 2.0).max(40.0);
        let needle = self.text_filter.to_lowercase();
        let filter = self.filter;
        let scroll = std::mem::take(&mut self.scroll_to_end);
        egui::Frame::new().fill(theme::mix(theme::BG, theme::SURFACE, 0.4)).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("terminal-output").max_height(output_height).min_scrolled_height(output_height).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for line in self.lines.iter().filter(|l| filter.shows(l.kind)).filter(|l| needle.is_empty() || l.text.to_lowercase().contains(&needle)) {
                    let (color, prefix) = match line.kind {
                        LineKind::Input => (theme::ACCENT_HI, "› "),
                        LineKind::Result => (theme::TEXT, "  "),
                        LineKind::Info => (theme::TEXT_DIM, ""),
                        LineKind::Warn => (theme::WARNING, "⚠ "),
                        LineKind::Error => (theme::DANGER, "✖ "),
                        LineKind::System => (theme::ACCENT_2, ""),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if let Some(source) = &line.source {
                            ui.label(RichText::new(format!("[{source}]")).font(theme::mono(12.0)).color(theme::TEXT_FAINT));
                        }
                        let response = ui.add(egui::Label::new(RichText::new(format!("{prefix}{}", line.text)).font(theme::mono(12.5)).color(color)).selectable(true));
                        if let Some((path, number)) = &line.location {
                            let link = ui.add(egui::Label::new(RichText::new(format!("{path}:{number}")).font(theme::mono(12.0)).color(theme::ACCENT_2).underline()).sense(egui::Sense::click()));
                            if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Open in the editor").clicked() {
                                requests.push(Request::Open(path.clone(), *number));
                            }
                        }
                        let _ = response;
                    });
                }
                if scroll {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        });
        ui.add_space(4.0);
        let id = egui::Id::new("terminal-input");
        let (up, down, tab) = if ui.ctx().memory(|m| m.has_focus(id)) {
            ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, Key::Tab)))
        } else {
            (false, false, false)
        };
        if up && !self.history.is_empty() {
            let at = self.browsing.map_or(self.history.len() - 1, |i| i.saturating_sub(1));
            self.browsing = Some(at);
            self.input.clone_from(&self.history[at]);
        }
        if down {
            match self.browsing {
                Some(i) if i + 1 < self.history.len() => {
                    self.browsing = Some(i + 1);
                    self.input.clone_from(&self.history[i + 1]);
                }
                _ => {
                    self.browsing = None;
                    self.input.clear();
                }
            }
        }
        if tab {
            let completion = crate::code::complete::complete(&self.input, self.input.len());
            if let Some(first) = completion.items.first() {
                self.input.replace_range(completion.start.., &first.label);
            }
        }
        let mut submitted = false;
        ui.horizontal(|ui| {
            let mode = match runtime.mode() {
                Mode::Edit => ("edit", theme::TEXT_FAINT),
                Mode::Play => ("play", theme::SUCCESS),
                Mode::Paused => ("paused", theme::WARNING),
            };
            ui.label(RichText::new(format!("{} ›", mode.0)).font(theme::mono_bold(13.0)).color(mode.1));
            let response = ui.add(egui::TextEdit::singleline(&mut self.input).id(id).hint_text("Lua or a command — try `help`").font(theme::mono(13.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE).text_color(Color32::WHITE));
            if self.focus {
                response.request_focus();
                self.focus = false;
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submitted = true;
            }
            if up || down || tab {
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                    let end = egui::text::CCursor::new(self.input.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), id);
                }
            }
        });
        if submitted {
            let command = std::mem::take(&mut self.input);
            self.browsing = None;
            requests.extend(self.execute(&command, runtime, selection));
            self.focus = true;
        }
        requests
    }

    /// Puts the keyboard focus on the command line.
    pub fn focus(&mut self) {
        self.focus = true;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use araria_core::Vec3;
    use araria_core::assets::AssetManifest;
    use araria_runtime::ProjectFiles;

    use super::*;

    fn runtime() -> Runtime {
        let mut dm = DataModel::new();
        let ws = dm.workspace();
        let floor = dm.create("Part", Some(ws)).unwrap();
        dm.set_name(floor, "Floor").unwrap();
        dm.set(floor, "Size", Vec3::new(50.0, 1.0, 50.0)).unwrap();
        let door = dm.create("Model", Some(ws)).unwrap();
        dm.set_name(door, "Door").unwrap();
        let handle = dm.create("Part", Some(door)).unwrap();
        dm.set_name(handle, "Handle").unwrap();
        dm.set_attribute(handle, "Locked", Variant::Bool(true)).unwrap();
        let holder = dm.service("Scripts");
        let script = dm.create("Script", Some(holder)).unwrap();
        dm.set_name(script, "main").unwrap();
        dm.set(script, "Source", "scripts/main.lua").unwrap();
        let files = ProjectFiles::new(|path| (path == "scripts/main.lua").then(|| "shared.hits = 0\nprint('ready')".to_string()), |_| Some(1));
        Runtime::with_files(dm, Arc::new(AssetManifest::default()), files)
    }

    fn output(terminal: &Terminal) -> String {
        terminal.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn evaluates_lua_in_the_game_context() {
        let mut rt = runtime();
        let mut terminal = Terminal::default();
        terminal.clear();
        terminal.execute("1 + 2", &mut rt, None);
        terminal.execute("workspace.Floor.Size.X", &mut rt, None);
        terminal.execute("print('hello from lua')", &mut rt, None);
        terminal.execute("local x = nil; x()", &mut rt, None);
        let lines: Vec<(LineKind, &str)> = terminal.lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert!(lines.contains(&(LineKind::Result, "3")), "{lines:?}");
        assert!(lines.contains(&(LineKind::Result, "50")), "{lines:?}");
        assert!(lines.iter().any(|(k, t)| *k == LineKind::Info && t.contains("hello from lua")), "{lines:?}");
        assert!(lines.iter().any(|(k, t)| *k == LineKind::Error && t.contains("nil")), "{lines:?}");
        // The console changes the real scene.
        terminal.execute("workspace.Floor.Name = 'Ground'", &mut rt, None);
        assert!(rt.dm.borrow().find_by_path("Workspace.Ground").is_some());
    }

    #[test]
    fn commands_manage_scripts_and_inspect_objects() {
        let mut rt = runtime();
        let mut terminal = Terminal::default();
        terminal.execute("scripts", &mut rt, None);
        assert!(output(&terminal).contains("scripts/main.lua"), "{}", output(&terminal));
        assert_eq!(terminal.execute("play", &mut rt, None), vec![Request::Play]);
        rt.start();
        rt.step(1.0 / 60.0);
        terminal.absorb(rt.scripts.take_log());
        assert!(output(&terminal).contains("ready"));
        terminal.execute("scripts", &mut rt, None);
        assert!(output(&terminal).contains("running"));
        terminal.execute("stop main", &mut rt, None);
        assert!(!rt.scripts.scripts()[0].running);
        terminal.execute("start main", &mut rt, None);
        assert!(rt.scripts.scripts()[0].running);
        terminal.execute("reload main", &mut rt, None);
        assert!(output(&terminal).contains("Reloaded scripts/main.lua"), "{}", output(&terminal));
        terminal.execute("players", &mut rt, None);
        assert!(output(&terminal).contains("this device"));

        terminal.clear();
        terminal.execute("inspect workspace.Door.Handle", &mut rt, None);
        let text = output(&terminal);
        assert!(text.contains("Workspace.Door.Handle (Part)") && text.contains("Anchored") && text.contains("@Locked"), "{text}");
        let handle = rt.dm.borrow().find_by_path("Workspace.Door.Handle").unwrap();
        assert_eq!(terminal.execute("select Workspace.Door.Handle", &mut rt, None), vec![Request::Select(handle)]);
        terminal.clear();
        terminal.execute("inspect", &mut rt, Some(handle));
        assert!(output(&terminal).contains("Handle (Part)"));
        terminal.execute("find hand", &mut rt, None);
        terminal.execute("tree", &mut rt, None);
        terminal.execute("scene", &mut rt, None);
        let text = output(&terminal);
        assert!(text.contains("Door (Model)") && text.contains("playing"), "{text}");
        assert_eq!(terminal.execute("stop", &mut rt, None), vec![Request::Stop]);
        assert_eq!(terminal.execute("open main 4", &mut rt, None), vec![Request::Open("scripts/main.lua".into(), 4)]);
        terminal.execute("nosuchcommand", &mut rt, None);
        assert_eq!(terminal.lines.back().unwrap().kind, LineKind::Warn);
        terminal.execute("nosuch.thing", &mut rt, None);
        assert_eq!(terminal.lines.back().unwrap().kind, LineKind::Error);
        terminal.execute("history", &mut rt, None);
        assert!(output(&terminal).contains("inspect workspace.Door.Handle"));
    }

    #[test]
    fn finds_error_locations() {
        assert_eq!(locations("scripts/main.lua:12: attempt to index nil\n    scripts/util.lua:3: in function 'go'"), vec![("scripts/main.lua".into(), 12), ("scripts/util.lua".into(), 3)]);
        assert!(locations("console:1: oops").is_empty());
        let dm = DataModel::new();
        assert_eq!(resolve(&dm, "workspace"), Some(dm.workspace()));
        assert_eq!(resolve(&dm, "game.Workspace"), Some(dm.workspace()));
        assert_eq!(describe(&dm, &Variant::Number(2.50)), "2.5");
    }
}
