//! The script editor widget: tabs of open files, a highlighted text area
//! with line numbers, automatic indentation, bracket matching,
//! autocomplete, signature hints, error markers, search and replace and an
//! outline for jumping between functions.
//!
//! The editor works on real project files: [`ScriptEditor::open`] reads a
//! file, saving writes it back (the running game then hot-reloads it).

use std::ops::Range;

use araria_platform::{icons, theme};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::text_edit::TextEditState;
use egui::{Align2, Color32, CornerRadius, Event, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2, vec2};

use super::api::Kind;
use super::complete::{self, Suggestion};
use super::edit::{self, Edit, Query};
use super::highlight;

/// Seconds after the last keystroke before the file is syntax-checked.
const CHECK_DELAY: f64 = 0.35;
const MAX_HIGHLIGHTS: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A problem shown on a line of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// Zero-based line.
    pub line: usize,
    pub message: String,
    pub severity: Severity,
}

/// An open file.
pub struct Document {
    /// Project path, e.g. `scripts/main.lua`.
    pub path: String,
    pub text: String,
    /// The content as last loaded from or saved to disk.
    saved: String,
    /// From compiling the current text.
    pub syntax_error: Option<Diagnostic>,
    /// Reported by the running game; cleared when the file changes.
    pub runtime_errors: Vec<Diagnostic>,
    /// Text the syntax check last ran on, and when the text last changed.
    checked: Option<u64>,
    edited_at: f64,
    /// Scroll to and put the cursor on this line next frame.
    reveal_line: Option<usize>,
}

fn text_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl Document {
    pub fn new(path: &str, text: String) -> Self {
        let mut document = Document { path: path.to_string(), saved: text.clone(), text, syntax_error: None, runtime_errors: Vec::new(), checked: None, edited_at: 0.0, reveal_line: None };
        document.check();
        document
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    /// File name without the folder.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Records that the text is now what is on disk.
    pub fn mark_saved(&mut self) {
        self.saved.clone_from(&self.text);
    }

    /// Replaces the text with a newer version from disk (another tool
    /// changed the file).
    pub fn reload_from_disk(&mut self, text: String) {
        self.saved.clone_from(&text);
        self.text = text;
        self.runtime_errors.clear();
        self.check();
    }

    pub fn saved_text(&self) -> &str {
        &self.saved
    }

    /// Compiles the text and records its first syntax error.
    pub fn check(&mut self) {
        let hash = text_hash(&self.text);
        if self.checked == Some(hash) {
            return;
        }
        self.checked = Some(hash);
        self.syntax_error = match araria_lua::compiler::compile(&self.text, &self.path) {
            Ok(_) => None,
            Err(error) => {
                let message = error.message();
                // "path:12: message" → "message"
                let message = message.strip_prefix(&format!("{}:", self.path)).and_then(|rest| rest.split_once(": ").map(|(_, m)| m.to_string())).unwrap_or(message);
                Some(Diagnostic { line: error.line.unwrap_or(1).saturating_sub(1) as usize, message, severity: Severity::Error })
            }
        };
    }

    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.syntax_error.iter().chain(self.runtime_errors.iter())
    }

    /// Asks the editor to show a line (one-based, as in error messages).
    pub fn reveal(&mut self, line: usize) {
        self.reveal_line = Some(line.saturating_sub(1));
    }
}

/// The autocomplete popup.
struct Popup {
    /// Byte offset of the word being completed.
    start: usize,
    items: Vec<Suggestion>,
    selected: usize,
}

#[derive(Default)]
struct SearchBar {
    query: Query,
    replacement: String,
    show_replace: bool,
    /// Index of the highlighted match.
    current: usize,
    focus: bool,
}

/// Something the editor needs the studio to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorAction {
    /// Write the document at this index to disk.
    Save(usize),
    SaveAll,
    /// Save and reload the script in the running game now.
    Apply(usize),
    /// The user asked to close a tab with unsaved changes.
    ConfirmClose(usize),
}

pub struct ScriptEditor {
    pub documents: Vec<Document>,
    pub active: usize,
    popup: Option<Popup>,
    search: Option<SearchBar>,
    goto_line: Option<String>,
    pub font_size: f32,
    /// Text of the line the cursor is on, for the status bar: (line, column), one-based.
    cursor: (usize, usize),
}

impl Default for ScriptEditor {
    fn default() -> Self {
        ScriptEditor { documents: Vec::new(), active: 0, popup: None, search: None, goto_line: None, font_size: 14.0, cursor: (1, 1) }
    }
}

fn kind_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Keyword => highlight::color(highlight::TokenKind::Keyword),
        Kind::Global | Kind::Library | Kind::Class => highlight::color(highlight::TokenKind::Builtin),
        Kind::Function | Kind::Method => highlight::color(highlight::TokenKind::Call),
        Kind::Property => highlight::color(highlight::TokenKind::Member),
        Kind::Event => theme::WARNING,
        Kind::Value => highlight::color(highlight::TokenKind::String),
        Kind::Local => theme::TEXT_DIM,
    }
}

fn kind_letter(kind: Kind) -> &'static str {
    match kind {
        Kind::Keyword => "k",
        Kind::Global => "g",
        Kind::Library => "L",
        Kind::Function => "f",
        Kind::Property => "p",
        Kind::Method => "m",
        Kind::Event => "e",
        Kind::Class => "C",
        Kind::Value => "v",
        Kind::Local => "·",
    }
}

/// A function definition, for the outline menu.
pub struct OutlineItem {
    pub name: String,
    pub line: usize,
}

/// Functions defined in a file (`function a.b()`, `local function c()`,
/// `local d = function()`).
pub fn outline(text: &str) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let trimmed = content.trim_start();
        let rest = trimmed.strip_prefix("local ").unwrap_or(trimmed);
        if let Some(after) = rest.strip_prefix("function ") {
            let name: String = after.chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':')).collect();
            if !name.is_empty() {
                items.push(OutlineItem { name, line });
            }
        } else if let Some((name, value)) = rest.split_once('=') {
            let name = name.trim();
            if value.trim_start().starts_with("function") && !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.')) {
                items.push(OutlineItem { name: name.to_string(), line });
            }
        }
    }
    items
}

impl ScriptEditor {
    /// Opens a file in a tab (or switches to it if it is already open).
    pub fn open(&mut self, path: &str, text: impl FnOnce() -> Option<String>) -> Result<(), String> {
        if let Some(index) = self.documents.iter().position(|d| d.path == path) {
            self.active = index;
            return Ok(());
        }
        let text = text().ok_or_else(|| format!("'{path}' could not be read"))?;
        self.documents.push(Document::new(path, text));
        self.active = self.documents.len() - 1;
        self.popup = None;
        Ok(())
    }

    pub fn active_document(&mut self) -> Option<&mut Document> {
        self.documents.get_mut(self.active)
    }

    pub fn find(&mut self, path: &str) -> Option<&mut Document> {
        self.documents.iter_mut().find(|d| d.path == path)
    }

    /// Opens (if needed) and shows a file at a line.
    pub fn reveal(&mut self, path: &str, line: usize, text: impl FnOnce() -> Option<String>) -> Result<(), String> {
        self.open(path, text)?;
        if let Some(document) = self.active_document() {
            document.reveal(line);
        }
        Ok(())
    }

    /// Closes a tab, discarding unsaved changes.
    pub fn close(&mut self, index: usize) {
        if index < self.documents.len() {
            self.documents.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
            self.popup = None;
        }
    }

    /// Updates open tabs after a file was renamed or moved.
    pub fn renamed(&mut self, from: &str, to: &str) {
        for document in &mut self.documents {
            if document.path == from {
                document.path = to.to_string();
            }
        }
    }

    /// Records an error the running game reported at `path:line` (one-based).
    pub fn report_runtime_error(&mut self, path: &str, line: u32, message: &str) {
        if let Some(document) = self.find(path) {
            let diagnostic = Diagnostic { line: line.saturating_sub(1) as usize, message: message.to_string(), severity: Severity::Error };
            if !document.runtime_errors.contains(&diagnostic) {
                document.runtime_errors.push(diagnostic);
            }
        }
    }

    pub fn has_unsaved(&self) -> bool {
        self.documents.iter().any(Document::is_dirty)
    }

    fn text_id(path: &str) -> Id {
        Id::new(("araria-code", path))
    }

    /// Draws the editor. Returns what the studio should do (saving, etc.).
    pub fn show(&mut self, ui: &mut Ui) -> Vec<EditorAction> {
        let mut actions = Vec::new();
        if self.documents.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Open a script from the Explorer or the Files list.\nCtrl+S saves · Ctrl+F finds · Ctrl+Space completes").font(theme::regular(13.0)).color(theme::TEXT_FAINT));
            });
            return actions;
        }
        self.active = self.active.min(self.documents.len() - 1);
        self.tab_bar(ui, &mut actions);
        self.shortcuts(ui, &mut actions);
        self.toolbar(ui, &mut actions);
        if self.search.is_some() {
            self.search_bar(ui);
        }
        if self.text_area(ui) {
            actions.push(EditorAction::Apply(self.active));
        }
        actions
    }

    fn tab_bar(&mut self, ui: &mut Ui, actions: &mut Vec<EditorAction>) {
        let mut close = None;
        egui::ScrollArea::horizontal().id_salt("code-tabs").max_height(34.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (index, document) in self.documents.iter().enumerate() {
                    let active = index == self.active;
                    let has_error = document.diagnostics().next().is_some();
                    let label = format!("{}  {}", icons::FILE_CODE, document.name());
                    let font = theme::medium(13.0);
                    let galley = ui.painter().layout_no_wrap(label, font, if active { theme::TEXT } else { theme::TEXT_DIM });
                    let size = vec2(galley.size().x + 46.0, 30.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let painter = ui.painter();
                    let fill = if active { theme::SURFACE_HI } else if response.hovered() { theme::SURFACE_HOVER } else { theme::SURFACE };
                    painter.rect_filled(rect, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, fill);
                    if active {
                        painter.line_segment([rect.left_bottom() + vec2(6.0, -1.0), rect.right_bottom() + vec2(-6.0, -1.0)], Stroke::new(2.0, theme::ACCENT));
                    }
                    let text_color = if has_error { theme::DANGER } else if active { theme::TEXT } else { theme::TEXT_DIM };
                    painter.galley_with_override_text_color(rect.left_center() + vec2(12.0, -galley.size().y / 2.0), galley, text_color);
                    // Close button, or a dot for unsaved changes.
                    let close_rect = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), Vec2::splat(18.0));
                    let close_response = ui.interact(close_rect, Id::new(("code-tab-close", index)), Sense::click());
                    if close_response.hovered() || (!document.is_dirty() && (response.hovered() || active)) {
                        ui.painter().text(close_rect.center(), Align2::CENTER_CENTER, icons::X, theme::regular(12.0), if close_response.hovered() { theme::TEXT } else { theme::TEXT_FAINT });
                    } else if document.is_dirty() {
                        ui.painter().circle_filled(close_rect.center(), 4.0, theme::ACCENT_HI);
                    }
                    let response = response.on_hover_text(&document.path);
                    if close_response.clicked() || response.middle_clicked() {
                        close = Some(index);
                    } else if response.clicked() {
                        self.active = index;
                        self.popup = None;
                    }
                }
            });
        });
        if let Some(index) = close {
            if self.documents[index].is_dirty() {
                actions.push(EditorAction::ConfirmClose(index));
            } else {
                self.close(index);
            }
        }
    }

    fn shortcuts(&mut self, ui: &mut Ui, actions: &mut Vec<EditorAction>) {
        let command = Modifiers::COMMAND;
        let (save_all, save, find, replace, goto) = ui.input_mut(|i| {
            (
                i.consume_key(command | Modifiers::SHIFT, Key::S),
                i.consume_key(command, Key::S),
                i.consume_key(command, Key::F),
                i.consume_key(command, Key::H),
                i.consume_key(command, Key::G),
            )
        });
        if save_all {
            actions.push(EditorAction::SaveAll);
        } else if save {
            actions.push(EditorAction::Save(self.active));
        }
        if find || replace {
            let selected = self.selected_text(ui);
            let search = self.search.get_or_insert_with(SearchBar::default);
            if let Some(selected) = selected.filter(|s| !s.contains('\n') && !s.is_empty()) {
                search.query.text = selected;
            }
            search.show_replace |= replace;
            search.focus = true;
        }
        if goto {
            self.goto_line = Some(String::new());
        }
    }

    fn selected_text(&self, ui: &Ui) -> Option<String> {
        let document = self.documents.get(self.active)?;
        let range = TextEditState::load(ui.ctx(), Self::text_id(&document.path))?.cursor.char_range()?;
        let chars = range.as_sorted_char_range();
        let (start, end) = (edit::char_to_byte(&document.text, chars.start.0), edit::char_to_byte(&document.text, chars.end.0));
        Some(document.text[start..end].to_string())
    }

    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<EditorAction>) {
        let index = self.active;
        // Left to right, with the least important items last, so a narrow
        // editor clips the path rather than overlapping its neighbours.
        let narrow = ui.available_width() < 460.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let apply = if narrow { RichText::new(icons::LIGHTNING) } else { RichText::new(format!("{} Apply to game", icons::LIGHTNING)) };
            if ui.add(egui::Button::new(apply.font(theme::regular(12.5)))).on_hover_text("Save and hot-reload this script in the running game (Ctrl+Enter)").clicked() {
                actions.push(EditorAction::Apply(index));
            }
            let dirty = self.documents[index].is_dirty();
            let save = if narrow { RichText::new(icons::FLOPPY_DISK) } else { RichText::new(format!("{} Save", icons::FLOPPY_DISK)) };
            if ui.add_enabled(dirty, egui::Button::new(save.font(theme::regular(12.5)))).on_hover_text("Save (Ctrl+S)").clicked() {
                actions.push(EditorAction::Save(index));
            }
            if ui.add(egui::Button::new(RichText::new(icons::MAGNIFYING_GLASS).font(theme::regular(13.0)))).on_hover_text("Find and replace (Ctrl+F / Ctrl+H)").clicked() {
                let search = self.search.get_or_insert_with(SearchBar::default);
                search.focus = true;
            }
            // Outline: jump to a function.
            let items = outline(&self.documents[index].text);
            let mut jump = None;
            ui.add_enabled_ui(!items.is_empty(), |ui| {
                let label = if narrow { RichText::new(icons::LIST) } else { RichText::new(format!("{} Functions", icons::LIST)) };
                ui.menu_button(label.font(theme::regular(12.5)), |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for item in &items {
                            if ui.button(RichText::new(format!("{}   :{}", item.name, item.line + 1)).font(theme::mono(12.5))).clicked() {
                                jump = Some(item.line);
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Jump to a function");
            });
            if let Some(line) = jump {
                self.documents[index].reveal_line = Some(line);
            }
            let document = &self.documents[index];
            let problems = document.diagnostics().count();
            if problems > 0 {
                let first = document.diagnostics().next().map(|d| d.line).unwrap_or(0);
                let label = RichText::new(format!("{} {problems}{}", icons::WARNING, if narrow { String::new() } else { format!(" problem{}", if problems == 1 { "" } else { "s" }) })).font(theme::regular(12.5)).color(theme::DANGER);
                if ui.add(egui::Button::new(label).frame(false)).on_hover_text("Go to the first problem").clicked() {
                    self.documents[index].reveal_line = Some(first);
                }
            }
            let (line, column) = self.cursor;
            ui.label(RichText::new(format!("{line}:{column}")).font(theme::mono(12.0)).color(theme::TEXT_FAINT)).on_hover_text("Line and column");
            let room = ui.available_width() - 4.0;
            if room > 60.0 {
                let path = araria_platform::widgets::elide(ui, &self.documents[index].path, theme::mono(12.0), room);
                ui.label(RichText::new(path).font(theme::mono(12.0)).color(theme::TEXT_FAINT));
            }
        });
        if let Some(text) = &mut self.goto_line {
            let mut done = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Go to line").font(theme::regular(12.5)).color(theme::TEXT_DIM));
                let response = ui.add(egui::TextEdit::singleline(text).desired_width(80.0).font(theme::mono(12.5)));
                response.request_focus();
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    if let Ok(line) = text.trim().parse::<usize>() {
                        self.documents[index].reveal(line);
                    }
                    done = true;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    done = true;
                }
            });
            if done {
                self.goto_line = None;
            }
        }
    }

    fn search_bar(&mut self, ui: &mut Ui) {
        let index = self.active;
        let Some(search) = &mut self.search else { return };
        let document = &mut self.documents[index];
        let matches = edit::find_all(&document.text, &search.query);
        let mut close = false;
        let mut jump: Option<Range<usize>> = None;
        let mut replaced: Option<String> = None;
        egui::Frame::new().fill(theme::SURFACE).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut search.query.text).hint_text("Find").desired_width(220.0).font(theme::mono(12.5)));
                if search.focus {
                    response.request_focus();
                    search.focus = false;
                }
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let shift = ui.input(|i| i.modifiers.shift);
                if response.changed() {
                    search.current = 0;
                    if let Some(first) = edit::find_all(&document.text, &search.query).first() {
                        jump = Some(first.clone());
                    }
                }
                ui.toggle_value(&mut search.query.match_case, RichText::new("Aa").font(theme::mono(12.0))).on_hover_text("Match case");
                ui.toggle_value(&mut search.query.whole_word, RichText::new("W").font(theme::mono(12.0))).on_hover_text("Whole word");
                let count = matches.len();
                let label = if search.query.text.is_empty() { String::new() } else if count == 0 { "No results".into() } else { format!("{} of {count}", search.current.min(count - 1) + 1) };
                ui.label(RichText::new(label).font(theme::regular(12.0)).color(if count == 0 { theme::TEXT_FAINT } else { theme::TEXT_DIM }));
                let previous = ui.small_button(icons::CARET_UP).on_hover_text("Previous (Shift+Enter)").clicked() || (enter && shift);
                let next = ui.small_button(icons::CARET_DOWN).on_hover_text("Next (Enter)").clicked() || (enter && !shift);
                if (next || previous) && count > 0 {
                    search.current = if next { (search.current + 1) % count } else { (search.current + count - 1) % count };
                    jump = Some(matches[search.current].clone());
                    if enter {
                        search.focus = true;
                    }
                }
                ui.toggle_value(&mut search.show_replace, RichText::new("Replace").font(theme::regular(12.0)));
                if ui.small_button(icons::X).on_hover_text("Close (Esc)").clicked() || (response.has_focus() && ui.input(|i| i.key_pressed(Key::Escape))) {
                    close = true;
                }
            });
            if search.show_replace {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut search.replacement).hint_text("Replace with").desired_width(220.0).font(theme::mono(12.5)));
                    let current = matches.get(search.current.min(matches.len().saturating_sub(1))).cloned();
                    if ui.add_enabled(current.is_some(), egui::Button::new("Replace")).clicked() {
                        if let Some(range) = current {
                            let mut text = document.text.clone();
                            text.replace_range(range.clone(), &search.replacement);
                            replaced = Some(text);
                            jump = Some(range.start..range.start + search.replacement.len());
                        }
                    }
                    if ui.add_enabled(!matches.is_empty(), egui::Button::new(format!("Replace all ({})", matches.len()))).clicked() {
                        replaced = Some(edit::replace_all(&document.text, &search.query, &search.replacement).0);
                    }
                });
            }
        });
        let id = Self::text_id(&document.path);
        if let Some(text) = replaced {
            let selection = jump.clone().unwrap_or(0..0);
            apply_edit(ui.ctx(), id, document, Edit { text, selection });
        } else if let Some(range) = jump {
            set_selection(ui.ctx(), id, &document.text, range.clone());
            document.reveal_line = Some(edit::line_of(&document.text, range.start));
        }
        if close {
            self.search = None;
        }
    }

    /// Returns true when Ctrl+Enter (apply to game) was pressed.
    #[allow(clippy::too_many_lines)]
    fn text_area(&mut self, ui: &mut Ui) -> bool {
        let index = self.active;
        let font = theme::mono(self.font_size);
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let id = Self::text_id(&self.documents[index].path);
        let ctx = ui.ctx().clone();
        let focused = ctx.memory(|m| m.has_focus(id));
        let now = ui.input(|i| i.time);

        // ---- keys the editor handles itself (before the text box sees them)
        let apply = focused && self.handle_keys(ui, id);

        let document = &mut self.documents[index];
        let before = document.text.clone();
        let line_count = document.text.split('\n').count();
        let gutter_width = (line_count.to_string().len().max(2) as f32) * row_height * 0.62 + 26.0;
        let typed: Vec<String> = if focused { ui.input(|i| i.events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.clone()) } else { None }).collect()) } else { Vec::new() };
        let deleted = focused && ui.input(|i| i.key_pressed(Key::Backspace) || i.key_pressed(Key::Delete));

        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
            let job: LayoutJob = highlight::layout(text.as_str(), font.clone());
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let background = theme::mix(theme::BG, theme::SURFACE, 0.5);
        let area = egui::Frame::new().fill(background).corner_radius(CornerRadius::same(8));
        let mut output_info = None;
        let mut scroll_clip = ui.clip_rect();
        area.show(ui, |ui| {
            egui::ScrollArea::both().id_salt(("code-scroll", &document.path)).auto_shrink([false, false]).show(ui, |ui| {
                scroll_clip = ui.clip_rect();
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let (gutter, _) = ui.allocate_exact_size(vec2(gutter_width, row_height), Sense::hover());
                    let behind = ui.painter().add(Shape::Noop);
                    let output = egui::TextEdit::multiline(&mut document.text)
                        .id(id)
                        .font(font.clone())
                        .code_editor()
                        .lock_focus(true)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin { left: 8, right: 24, top: 6, bottom: 60 })
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .layouter(&mut layouter)
                        .show(ui);
                    output_info = Some((gutter, behind, output));
                });
            });
        });
        let Some((gutter, behind, output)) = output_info else { return apply };
        let changed = document.text != before;
        if changed {
            document.edited_at = now;
            document.runtime_errors.clear();
        }
        if now - document.edited_at > CHECK_DELAY {
            document.check();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(CHECK_DELAY));
        }

        let galley = &output.galley;
        let origin = output.galley_pos;
        let clip = output.text_clip_rect;
        // The gutter stays at the left edge when the code scrolls sideways.
        let gutter = gutter.translate(vec2((scroll_clip.left() - gutter.left()).max(0.0), 0.0));
        let painter = ui.painter_at(scroll_clip);
        let selection = output.cursor_range.map(|r| { let s = r.as_sorted_char_range(); s.start.0..s.end.0 });
        let cursor_char = output.cursor_range.map(|r| r.primary.index.0);
        let cursor_byte = cursor_char.map(|c| edit::char_to_byte(&document.text, c));
        let row_rect = |char_index: usize| -> Rect { galley.pos_from_cursor(CCursor::new(char_index)).translate(origin.to_vec2()) };
        let line_rect = |line: usize| -> Rect {
            let rect = galley.rows.get(line).map_or(Rect::from_min_size(Pos2::ZERO, vec2(0.0, row_height)), |row| row.rect());
            Rect::from_x_y_ranges(clip.x_range(), (origin.y + rect.top())..=(origin.y + rect.top() + row_height))
        };

        // ---- drawn behind the text
        let mut shapes = Vec::new();
        let cursor_line = cursor_byte.map(|b| edit::line_of(&document.text, b));
        if let Some(line) = cursor_line.filter(|_| focused) {
            shapes.push(Shape::rect_filled(line_rect(line), CornerRadius::ZERO, theme::with_alpha(theme::SURFACE_HOVER, 0.45)));
        }
        for diagnostic in document.diagnostics() {
            let color = if diagnostic.severity == Severity::Error { theme::DANGER } else { theme::WARNING };
            shapes.push(Shape::rect_filled(line_rect(diagnostic.line), CornerRadius::ZERO, theme::with_alpha(color, 0.10)));
        }
        let char_rect = |text: &str, range: Range<usize>| -> Rect {
            let (a, b) = (edit::byte_to_char(text, range.start), edit::byte_to_char(text, range.end));
            row_rect(a).union(row_rect(b))
        };
        if let Some(search) = &self.search {
            let matches = edit::find_all(&document.text, &search.query);
            for (i, range) in matches.into_iter().take(MAX_HIGHLIGHTS).enumerate() {
                let color = if i == search.current { theme::with_alpha(theme::WARNING, 0.45) } else { theme::with_alpha(theme::WARNING, 0.2) };
                shapes.push(Shape::rect_filled(char_rect(&document.text, range).expand2(vec2(1.0, 0.0)), CornerRadius::same(2), color));
            }
        } else if let Some(word) = selection.as_ref().filter(|r| r.end > r.start).map(|r| {
            let (a, b) = (edit::char_to_byte(&document.text, r.start), edit::char_to_byte(&document.text, r.end));
            document.text[a..b].to_string()
        }) {
            // Other occurrences of a selected word.
            if word.len() > 1 && word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                let query = Query { text: word, match_case: true, whole_word: true };
                for range in edit::find_all(&document.text, &query).into_iter().take(MAX_HIGHLIGHTS) {
                    shapes.push(Shape::rect_filled(char_rect(&document.text, range), CornerRadius::same(2), theme::with_alpha(theme::ACCENT, 0.18)));
                }
            }
        }
        if let Some(cursor) = cursor_byte.filter(|_| focused) {
            if let Some((a, b)) = edit::matching_bracket(&document.text, cursor) {
                for at in [a, b] {
                    shapes.push(Shape::rect_stroke(char_rect(&document.text, at..at + 1), CornerRadius::same(2), Stroke::new(1.0, theme::ACCENT_2), egui::StrokeKind::Inside));
                }
            }
        }
        ui.painter().set(behind, Shape::Vec(shapes));

        // ---- gutter: line numbers and markers
        let gutter_rect = Rect::from_x_y_ranges(gutter.left()..=gutter.right(), clip.y_range());
        painter.rect_filled(gutter_rect, CornerRadius::ZERO, background);
        let first_visible = ((clip.top() - origin.y) / row_height).floor().max(0.0) as usize;
        let last_visible = (((clip.bottom() - origin.y) / row_height).ceil().max(0.0) as usize).min(galley.rows.len());
        let number_font = FontId::monospace(self.font_size * 0.86);
        let number_font = FontId { family: font.family.clone(), ..number_font };
        for line in first_visible..last_visible {
            let y = origin.y + galley.rows[line].rect().center().y;
            let problem = document.diagnostics().find(|d| d.line == line);
            let color = if problem.is_some() { theme::DANGER } else if Some(line) == cursor_line { theme::TEXT_DIM } else { theme::with_alpha(theme::TEXT_FAINT, 0.7) };
            painter.text(Pos2::new(gutter.right() - 12.0, y), Align2::RIGHT_CENTER, (line + 1).to_string(), number_font.clone(), color);
            if problem.is_some() {
                painter.circle_filled(Pos2::new(gutter.left() + 7.0, y), 3.0, theme::DANGER);
            }
        }

        // ---- on top: squiggles and inline messages
        for diagnostic in document.diagnostics() {
            let Some(row) = galley.rows.get(diagnostic.line) else { continue };
            let rect = row.rect().translate(origin.to_vec2());
            let color = if diagnostic.severity == Severity::Error { theme::DANGER } else { theme::WARNING };
            let (left, right) = (rect.left(), rect.right().max(rect.left() + 24.0));
            let y = origin.y + row.rect().top() + row_height - 1.5;
            let mut points = Vec::new();
            let mut x = left;
            let mut up = true;
            while x <= right {
                points.push(Pos2::new(x, if up { y - 1.5 } else { y + 1.0 }));
                x += 3.0;
                up = !up;
            }
            painter.add(Shape::line(points, Stroke::new(1.0, color)));
            let message = if diagnostic.message.len() > 140 { format!("{}…", &diagnostic.message[..diagnostic.message.floor_char_boundary(140)]) } else { diagnostic.message.clone() };
            painter.text(Pos2::new(right + 18.0, rect.center().y), Align2::LEFT_CENTER, format!("● {message}"), theme::regular(self.font_size * 0.86), theme::with_alpha(color, 0.85));
        }

        // ---- reveal a requested line
        if let Some(line) = document.reveal_line.take() {
            let line = line.min(galley.rows.len().saturating_sub(1));
            let start = edit::offset_of_line(&document.text, line);
            let indent = edit::indentation(&document.text, start).len();
            set_selection(&ctx, id, &document.text, start + indent..start + indent);
            let target = line_rect(line);
            ui.scroll_to_rect(target, Some(egui::Align::Center));
            ctx.memory_mut(|m| m.request_focus(id));
        }

        // ---- status: cursor position
        if let Some(byte) = cursor_byte {
            let line_start = edit::line_start(&document.text, byte);
            self.cursor = (edit::line_of(&document.text, byte) + 1, document.text[line_start..byte].chars().count() + 1);
        }

        // ---- completion and dedent after typing
        let Some(cursor) = cursor_byte else {
            self.popup = None;
            return apply;
        };
        if changed && focused {
            let last = typed.last().and_then(|t| t.chars().last());
            if let Some(c) = last {
                if c.is_alphabetic() || c == '}' || c == ')' || c == ']' {
                    if let Some(edit) = edit::dedent_closer(&document.text, cursor) {
                        apply_edit(&ctx, id, document, edit);
                    }
                }
            }
            let text = &document.text;
            let cursor = output.cursor_range.map_or(cursor, |r| edit::char_to_byte(text, r.primary.index.0));
            let wants = last.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '"') || (deleted && self.popup.is_some());
            if wants {
                self.update_popup(cursor);
            } else {
                self.popup = None;
            }
        } else if self.popup.as_ref().is_some_and(|p| cursor < p.start) || !focused {
            self.popup = None;
        }
        let document = &self.documents[index];
        let cursor_rect = row_rect(output.cursor_range.map_or(0, |r| r.primary.index.0));

        // ---- the popup
        let mut accept: Option<usize> = None;
        if let Some(popup) = &mut self.popup {
            let pos = cursor_rect.left_bottom() + vec2(-4.0, 4.0);
            egui::Area::new(Id::new("code-completion")).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
                egui::Frame::new().fill(theme::SURFACE_HI).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                                for (i, item) in popup.items.iter().enumerate().take(80) {
                                    let selected = i == popup.selected;
                                    let (rect, response) = ui.allocate_exact_size(vec2(300.0, 22.0), Sense::click());
                                    if selected {
                                        ui.painter().rect_filled(rect, CornerRadius::same(5), theme::with_alpha(theme::ACCENT, 0.35));
                                        response.scroll_to_me(None);
                                    } else if response.hovered() {
                                        ui.painter().rect_filled(rect, CornerRadius::same(5), theme::SURFACE_HOVER);
                                    }
                                    let color = kind_color(item.kind);
                                    ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::CENTER_CENTER, kind_letter(item.kind), theme::mono_bold(11.5), color);
                                    ui.painter().text(rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, &item.label, theme::mono(12.5), theme::TEXT);
                                    if !item.detail.is_empty() {
                                        let detail = araria_platform::widgets::elide(ui, &item.detail, theme::mono(11.0), 130.0);
                                        ui.painter().text(rect.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, detail, theme::mono(11.0), theme::TEXT_FAINT);
                                    }
                                    if response.clicked() {
                                        accept = Some(i);
                                    }
                                }
                            });
                        });
                        if let Some(item) = popup.items.get(popup.selected).filter(|i| !i.doc.is_empty() || !i.detail.is_empty()) {
                            ui.vertical(|ui| {
                                ui.set_max_width(280.0);
                                ui.label(RichText::new(format!("{}{}", item.label, if item.kind == Kind::Function || item.kind == Kind::Method { item.detail.as_str() } else { "" })).font(theme::mono(12.0)).color(kind_color(item.kind)));
                                if !matches!(item.kind, Kind::Function | Kind::Method) && !item.detail.is_empty() {
                                    ui.label(RichText::new(&item.detail).font(theme::mono(11.5)).color(theme::TEXT_FAINT));
                                }
                                ui.label(RichText::new(&item.doc).font(theme::regular(12.0)).color(theme::TEXT_DIM));
                            });
                        }
                    });
                });
            });
        } else if focused {
            // Signature help while typing arguments.
            if let Some(hint) = complete::signature_hint(&document.text, cursor) {
                let pos = cursor_rect.left_top() - vec2(4.0, 6.0);
                egui::Area::new(Id::new("code-signature")).order(egui::Order::Foreground).pivot(Align2::LEFT_BOTTOM).fixed_pos(pos).interactable(false).show(&ctx, |ui| {
                    egui::Frame::new().fill(theme::SURFACE_HI).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(format!("{}{}", hint.name, hint.signature)).font(theme::mono(12.0)).color(highlight::color(highlight::TokenKind::Call)));
                        if !hint.doc.is_empty() {
                            ui.label(RichText::new(&hint.doc).font(theme::regular(11.5)).color(theme::TEXT_DIM));
                        }
                    });
                });
            }
        }
        if let Some(i) = accept {
            if let Some(popup) = &mut self.popup {
                popup.selected = i;
            }
            self.accept_completion(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
        }
        apply
    }

    /// Recomputes the completion list at `cursor`.
    fn update_popup(&mut self, cursor: usize) {
        let Some(document) = self.documents.get(self.active) else { return };
        let completion = complete::complete(&document.text, cursor);
        if completion.items.is_empty() {
            self.popup = None;
            return;
        }
        // Do not offer exactly what has been typed already.
        let typed = &document.text[completion.start..cursor];
        if completion.items.len() == 1 && completion.items[0].label == typed {
            self.popup = None;
            return;
        }
        self.popup = Some(Popup { start: completion.start, items: completion.items, selected: 0 });
    }

    fn accept_completion(&mut self, ctx: &egui::Context, id: Id) {
        let Some(popup) = self.popup.take() else { return };
        let Some(item) = popup.items.get(popup.selected) else { return };
        let Some(document) = self.documents.get_mut(self.active) else { return };
        let Some(range) = TextEditState::load(ctx, id).and_then(|s| s.cursor.char_range()) else { return };
        let cursor = edit::char_to_byte(&document.text, range.primary.index.0);
        let start = popup.start.min(cursor);
        // Replace the whole word under the cursor, not just the typed part.
        let end = cursor + document.text[cursor..].bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
        let mut text = document.text.clone();
        let mut insert = item.label.clone();
        let mut caret = insert.len();
        if matches!(item.kind, Kind::Function | Kind::Method) && !document.text[end..].starts_with('(') {
            let takes_arguments = !item.detail.starts_with("()");
            insert.push_str("()");
            caret = if takes_arguments { insert.len() - 1 } else { insert.len() };
        }
        text.replace_range(start..end, &insert);
        let at = start + caret;
        apply_edit(ctx, id, document, Edit { text, selection: at..at });
    }

    /// Handles keys the text box should not see. Returns true for Ctrl+Enter.
    fn handle_keys(&mut self, ui: &mut Ui, id: Id) -> bool {
        let ctx = ui.ctx().clone();
        let index = self.active;
        // Completion popup navigation.
        if self.popup.is_some() {
            let (up, down, accept, escape) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if let Some(popup) = &mut self.popup {
                let count = popup.items.len().min(80);
                if up {
                    popup.selected = (popup.selected + count - 1) % count;
                }
                if down {
                    popup.selected = (popup.selected + 1) % count;
                }
            }
            if escape {
                self.popup = None;
            }
            if accept {
                self.accept_completion(&ctx, id);
                return false;
            }
        }
        let Some(range) = TextEditState::load(&ctx, id).and_then(|s| s.cursor.char_range()) else { return false };
        let document = &mut self.documents[index];
        let chars = range.as_sorted_char_range();
        let selection = edit::char_to_byte(&document.text, chars.start.0)..edit::char_to_byte(&document.text, chars.end.0);
        let (apply, outdent, indent, comment, newline, complete) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::SHIFT, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::COMMAND, Key::Slash),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::Space),
            )
        });
        let edit = if outdent {
            Some(edit::outdent(&document.text, selection.clone()))
        } else if indent {
            Some(edit::indent(&document.text, selection.clone()))
        } else if comment {
            Some(edit::toggle_comment(&document.text, selection.clone()))
        } else if newline {
            Some(edit::newline(&document.text, selection.clone()))
        } else {
            None
        };
        if let Some(edit) = edit {
            apply_edit(&ctx, id, document, edit);
            self.popup = None;
        }
        if complete {
            self.update_popup(selection.end);
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) && self.search.is_some() {
            self.search = None;
        }
        apply
    }
}

/// Puts the cursor (or a selection) at byte offsets.
fn set_selection(ctx: &egui::Context, id: Id, text: &str, range: Range<usize>) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let (a, b) = (edit::byte_to_char(text, range.start), edit::byte_to_char(text, range.end));
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    state.store(ctx, id);
}

/// Replaces a document's text with an edit, keeping it undoable with Ctrl+Z.
fn apply_edit(ctx: &egui::Context, id: Id, document: &mut Document, edit: Edit) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let previous = state.cursor.char_range().unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
    let mut undoer = state.undoer();
    undoer.add_undo(&(previous, document.text.clone()));
    state.set_undoer(undoer);
    let (a, b) = (edit::byte_to_char(&edit.text, edit.selection.start), edit::byte_to_char(&edit.text, edit.selection.end));
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    state.store(ctx, id);
    document.text = edit.text;
    document.edited_at = ctx.input(|i| i.time);
    document.runtime_errors.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_track_unsaved_changes_and_syntax_errors() {
        let mut document = Document::new("scripts/a.lua", "local x = 1\n".into());
        assert!(!document.is_dirty());
        assert!(document.syntax_error.is_none());
        document.text.push_str("if x then\n");
        assert!(document.is_dirty());
        document.check();
        let error = document.syntax_error.clone().expect("unclosed if");
        assert_eq!(error.severity, Severity::Error);
        assert!(!error.message.starts_with("scripts/a.lua"), "{}", error.message);
        document.text.push_str("end\n");
        document.check();
        assert!(document.syntax_error.is_none());
        document.mark_saved();
        assert!(!document.is_dirty());
    }

    #[test]
    fn editor_opens_reveals_and_closes_files() {
        let mut editor = ScriptEditor::default();
        editor.open("scripts/a.lua", || Some("print(1)".into())).unwrap();
        editor.open("scripts/b.lua", || Some("print(2)".into())).unwrap();
        assert_eq!(editor.active, 1);
        editor.open("scripts/a.lua", || panic!("already open, not re-read")).unwrap();
        assert_eq!(editor.active, 0);
        assert!(editor.open("scripts/missing.lua", || None).is_err());
        editor.report_runtime_error("scripts/b.lua", 1, "attempt to call a nil value");
        assert_eq!(editor.documents[1].runtime_errors.len(), 1);
        editor.renamed("scripts/b.lua", "scripts/c.lua");
        assert_eq!(editor.documents[1].name(), "c.lua");
        editor.close(0);
        assert_eq!(editor.documents.len(), 1);
        assert_eq!(editor.active, 0);
    }

    #[test]
    fn outline_lists_functions() {
        let text = "local function spawn()\nend\nfunction Game.start(x)\nend\nlocal onTouch = function(hit) end\nlocal y = 2\n  function obj:method() end";
        let names: Vec<(String, usize)> = outline(text).into_iter().map(|i| (i.name, i.line)).collect();
        assert_eq!(names, vec![("spawn".into(), 0), ("Game.start".into(), 2), ("onTouch".into(), 4), ("obj:method".into(), 6)]);
    }
}

/// Drives the real widget through egui with synthetic input.
#[cfg(test)]
mod widget_tests {
    use egui::{Event, Key, Modifiers, RawInput, Rect, pos2, vec2};

    use super::*;

    struct Harness {
        ctx: egui::Context,
        editor: ScriptEditor,
    }

    impl Harness {
        fn new(text: &str) -> Self {
            let ctx = egui::Context::default();
            araria_platform::theme::install(&ctx);
            let mut editor = ScriptEditor::default();
            editor.open("scripts/test.lua", || Some(text.to_string())).unwrap();
            let mut harness = Harness { ctx, editor };
            harness.frame(Vec::new());
            // Put the cursor at the end and focus the text.
            let id = ScriptEditor::text_id("scripts/test.lua");
            let end = harness.editor.documents[0].text.len();
            set_selection(&harness.ctx, id, &harness.editor.documents[0].text, end..end);
            harness.ctx.memory_mut(|m| m.request_focus(id));
            harness.frame(Vec::new());
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<EditorAction> {
            let input = RawInput { events, screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))), ..Default::default() };
            let mut actions = Vec::new();
            let mut output = self.ctx.run_ui(input, |ui| {
                actions = self.editor.show(ui);
            });
            output.textures_delta.clear();
            actions
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) -> Vec<EditorAction> {
            self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }])
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![Event::Text(text.to_string())]);
        }

        fn text(&self) -> &str {
            &self.editor.documents[0].text
        }
    }

    #[test]
    fn typing_indents_completes_and_dedents() {
        let mut h = Harness::new("if ready then");
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "if ready then\n    ");
        h.type_text("pri");
        let labels: Vec<String> = h.editor.popup.as_ref().expect("completion popup").items.iter().map(|i| i.label.clone()).collect();
        assert!(labels.contains(&"print".to_string()), "{labels:?}");
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.text(), "if ready then\n    print()", "Enter accepts the suggestion");
        assert!(h.editor.popup.is_none());
        // The cursor sits between the parentheses.
        h.type_text("1");
        assert_eq!(h.text(), "if ready then\n    print(1)");
        h.key(Key::End, Modifiers::NONE);
        h.key(Key::Enter, Modifiers::NONE);
        h.type_text("end");
        assert_eq!(h.text(), "if ready then\n    print(1)\nend", "typing `end` dedents it");
        // Ctrl+Z undoes the automatic dedent first.
        h.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(h.text(), "if ready then\n    print(1)\n    end");
    }

    #[test]
    fn shortcuts_comment_indent_save_and_search() {
        let mut h = Harness::new("local a = 1");
        h.key(Key::Slash, Modifiers::COMMAND);
        assert_eq!(h.text(), "-- local a = 1");
        h.key(Key::Slash, Modifiers::COMMAND);
        assert_eq!(h.text(), "local a = 1");
        h.key(Key::Tab, Modifiers::NONE);
        assert_eq!(h.text(), "local a = 1 ", "Tab at column 11 pads to the next indent stop");
        h.key(Key::Home, Modifiers::NONE);
        h.key(Key::Tab, Modifiers::NONE);
        assert_eq!(h.text(), "    local a = 1 ");
        h.key(Key::Tab, Modifiers::SHIFT);
        assert_eq!(h.text(), "local a = 1 ");
        assert!(h.editor.documents[0].is_dirty());
        assert_eq!(h.key(Key::S, Modifiers::COMMAND), vec![EditorAction::Save(0)]);
        assert_eq!(h.key(Key::Enter, Modifiers::COMMAND), vec![EditorAction::Apply(0)]);
        h.key(Key::F, Modifiers::COMMAND);
        assert!(h.editor.search.is_some(), "Ctrl+F opens the search bar");
    }

    #[test]
    fn syntax_errors_are_reported_on_their_line() {
        let mut h = Harness::new("local x = 1\nlocal y = (2\n");
        h.editor.documents[0].check();
        h.frame(Vec::new());
        let error = h.editor.documents[0].syntax_error.clone().expect("syntax error");
        assert!(error.line >= 1, "{error:?}");
    }
}
