//! The platform shell: navigation, top bar, dialogs and notifications
//! around the pages in [`crate::pages`].

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use crate::cards::{self, Thumbnails};
use crate::icons;
use crate::library::{GameKey, Library, Origin};
use crate::pages;
use crate::settings::Settings;
use crate::theme::{self, ACCENT, ACCENT_HI, BG, BORDER, SIDEBAR, SURFACE, SURFACE_HI, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::widgets::{self, ButtonKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Discover,
    Store,
    Create,
    Library,
    Settings,
}

impl Page {
    const ALL: [Page; 6] = [Page::Home, Page::Discover, Page::Store, Page::Create, Page::Library, Page::Settings];

    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Discover => "Discover",
            Page::Store => "Store",
            Page::Create => "Create",
            Page::Library => "Library",
            Page::Settings => "Settings",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Home => icons::HOUSE,
            Page::Discover => icons::COMPASS,
            Page::Store => icons::STOREFRONT,
            Page::Create => icons::HAMMER,
            Page::Library => icons::BOOKS,
            Page::Settings => icons::GEAR,
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::Home => "Jump back in or find something new",
            Page::Discover => "Browse every experience",
            Page::Store => "Games that ship with Araria",
            Page::Create => "Build your own game",
            Page::Library => "Everything on this device",
            Page::Settings => "Make Araria yours",
        }
    }
}

/// Something the platform asks the application to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Launch a game or project.
    Play(GameKey),
    /// Open a project in the studio.
    Edit(GameKey),
    /// Settings changed and should be applied to the engine.
    SettingsChanged,
    /// Offer a file to the user as a download (browser builds).
    Download { name: String, contents: String },
    /// Close the application (desktop builds).
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: f64,
}

pub(crate) struct NewProject {
    pub name: String,
    pub template: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sort {
    Popular,
    Newest,
    Name,
}

/// State of the platform interface.
pub struct PlatformUi {
    pub page: Page,
    pub(crate) search: String,
    pub(crate) category: Option<&'static str>,
    pub(crate) sort: Sort,
    pub(crate) detail: Option<GameKey>,
    pub(crate) new_project: Option<NewProject>,
    pub(crate) confirm_delete: Option<GameKey>,
    pub(crate) thumbs: Thumbnails,
    pub(crate) library_tab: usize,
    pub(crate) settings_tab: usize,
    /// Index of the control binding waiting for a key press.
    pub(crate) rebinding: Option<usize>,
    pub(crate) import_path: String,
    toasts: Vec<Toast>,
    /// Contents of dropped files waiting to be imported.
    imports: std::rc::Rc<std::cell::RefCell<Vec<Result<Vec<u8>, String>>>>,
    page_since: f64,
    themed: bool,
}

impl Default for PlatformUi {
    fn default() -> Self {
        PlatformUi {
            page: Page::Home,
            search: String::new(),
            category: None,
            sort: Sort::Popular,
            detail: None,
            new_project: None,
            confirm_delete: None,
            thumbs: Thumbnails::default(),
            library_tab: 0,
            settings_tab: 0,
            rebinding: None,
            import_path: String::new(),
            toasts: Vec::new(),
            imports: Default::default(),
            page_since: 0.0,
            themed: false,
        }
    }
}

/// "3 hours ago"-style relative time.
pub(crate) fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => plural(seconds / 60, "minute"),
        3600..=86_399 => plural(seconds / 3600, "hour"),
        86_400..=2_591_999 => plural(seconds / 86_400, "day"),
        _ => plural(seconds / 2_592_000, "month"),
    }
}

pub(crate) fn size_label(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", (bytes / 1024).max(1))
    }
}

impl PlatformUi {
    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), kind, born: -1.0 });
    }

    pub(crate) fn go(&mut self, page: Page, now: f64) {
        if self.page != page {
            self.page = page;
            self.page_since = now;
        }
    }

    /// Applies fonts, colors, interface scale and motion preferences.
    /// Returns false on the frame the fonts are installed: they only become
    /// usable on the next frame, so nothing should be drawn yet.
    pub fn apply_style(&mut self, ctx: &egui::Context, settings: &Settings) -> bool {
        if !self.themed {
            theme::install(ctx);
            self.themed = true;
            ctx.request_repaint();
            return false;
        }
        let animation = if settings.accessibility.reduce_motion { 0.0 } else { 0.14 };
        ctx.all_styles_mut(|style| style.animation_time = animation);
        let zoom = settings.accessibility.ui_scale.clamp(0.75, 1.75);
        if (ctx.zoom_factor() - zoom).abs() > 0.001 {
            ctx.set_zoom_factor(zoom);
        }
        true
    }

    /// Draws the whole platform into `ui` and returns what the application
    /// should do in response.
    pub fn show(&mut self, ui: &mut Ui, library: &mut Library, settings: &mut Settings) -> Vec<Action> {
        let mut actions = Vec::new();
        if !self.apply_style(ui.ctx(), settings) {
            return actions;
        }
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BG);

        if !library.storage().is_ready() {
            widgets::spinner(ui, full.center() - vec2(0.0, 20.0), 22.0);
            ui.painter().text(full.center() + vec2(0.0, 30.0), Align2::CENTER_CENTER, "Loading your library…", theme::medium(15.0), TEXT_DIM);
            return actions;
        }

        self.import_dropped_files(ui, library);
        let now = ui.input(|i| i.time);
        let compact = full.width() < 980.0;
        let sidebar_width = if compact { 76.0 } else { 236.0 };
        let sidebar = Rect::from_min_max(full.min, pos2(full.left() + sidebar_width, full.bottom()));
        let main = Rect::from_min_max(pos2(sidebar.right(), full.top()), full.max);
        self.sidebar(ui, sidebar, compact, library, now, &mut actions);

        let top = Rect::from_min_max(main.min, pos2(main.right(), main.top() + 76.0));
        self.top_bar(ui, top, now);

        let content = Rect::from_min_max(pos2(main.left(), top.bottom()), main.max);
        // Pages fade and slide in when switching.
        let reveal = if ui.style().animation_time > 0.0 { ((now - self.page_since) / 0.2).clamp(0.0, 1.0) as f32 } else { 1.0 };
        if reveal < 1.0 {
            ui.ctx().request_repaint();
        }
        let eased = 1.0 - (1.0 - reveal).powi(3);
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content);
            ui.multiply_opacity(eased);
            egui::ScrollArea::vertical().id_salt(("page", self.page.label())).auto_shrink([false, false]).show(ui, |ui| {
                let width = (ui.available_width() - 72.0).clamp(320.0, 1280.0);
                ui.add_space(8.0 + 14.0 * (1.0 - eased));
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        match self.page {
                            Page::Home => pages::home(ui, self, library, &mut actions),
                            Page::Discover => pages::discover(ui, self, library, &mut actions),
                            Page::Store => pages::store(ui, self, library, &mut actions),
                            Page::Create => pages::create(ui, self, library, &mut actions),
                            Page::Library => pages::library(ui, self, library, &mut actions),
                            Page::Settings => pages::settings(ui, self, library, settings, &mut actions),
                        }
                        ui.add_space(48.0);
                    });
                });
            });
        });

        self.dialogs(ui.ctx(), library, &mut actions);
        self.draw_toasts(ui, full, now);
        actions
    }

    fn sidebar(&mut self, ui: &mut Ui, rect: Rect, compact: bool, library: &Library, now: f64, actions: &mut Vec<Action>) {
        ui.painter().rect_filled(rect, 0.0, SIDEBAR);
        ui.painter().line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, BORDER));

        // Logo: a gem mark and the wordmark.
        let logo = pos2(rect.left() + if compact { 38.0 } else { 40.0 }, rect.top() + 40.0);
        let gem = [logo + vec2(0.0, -15.0), logo + vec2(14.0, -3.0), logo + vec2(0.0, 15.0), logo + vec2(-14.0, -3.0)];
        ui.painter().add(egui::Shape::convex_polygon(gem.to_vec(), ACCENT, Stroke::NONE));
        ui.painter().add(egui::Shape::convex_polygon(vec![gem[0], gem[1], logo + vec2(0.0, -3.0)], ACCENT_HI, Stroke::NONE));
        ui.painter().add(egui::Shape::convex_polygon(vec![gem[3], logo + vec2(0.0, -3.0), gem[2]], theme::mix(ACCENT, theme::ACCENT_2, 0.55), Stroke::NONE));
        if !compact {
            ui.painter().text(logo + vec2(26.0, 0.0), Align2::LEFT_CENTER, "Araria", theme::bold(24.0), TEXT);
        }

        let mut y = rect.top() + 96.0;
        for page in Page::ALL {
            if page == Page::Settings {
                y += 14.0;
                ui.painter().line_segment([pos2(rect.left() + 20.0, y - 7.0), pos2(rect.right() - 20.0, y - 7.0)], Stroke::new(1.0, BORDER));
            }
            let item = Rect::from_min_size(pos2(rect.left() + 14.0, y), vec2(rect.width() - 28.0, 46.0));
            let response = ui.interact(item, Id::new(("nav", page.label())), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let response = if compact { response.on_hover_text(page.label()) } else { response };
            let active = ui.ctx().animate_bool_with_time(response.id.with("active"), self.page == page, widgets::anim_time(ui));
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), widgets::anim_time(ui));
            let fill = theme::mix(theme::with_alpha(SURFACE_HI, hover), theme::with_alpha(ACCENT, 0.2), active);
            ui.painter().rect_filled(item, CornerRadius::same(12), fill);
            if active > 0.0 {
                let bar = Rect::from_center_size(pos2(item.left() + 2.0, item.center().y), vec2(4.0, 22.0 * active));
                ui.painter().rect_filled(bar, CornerRadius::same(2), ACCENT_HI);
            }
            let tint = theme::mix(theme::mix(TEXT_DIM, TEXT, hover), Color32::WHITE, active);
            let icon_x = if compact { item.center().x } else { item.left() + 26.0 };
            ui.painter().text(pos2(icon_x, item.center().y), Align2::CENTER_CENTER, page.icon(), theme::regular(21.0), theme::mix(tint, ACCENT_HI, active * 0.6));
            if !compact {
                let font = if self.page == page { theme::semibold(15.0) } else { theme::medium(15.0) };
                ui.painter().text(pos2(item.left() + 52.0, item.center().y), Align2::LEFT_CENTER, page.label(), font, tint);
            }
            if response.clicked() {
                self.go(page, now);
            }
            y += 52.0;
        }

        // Footer: storage status, and Exit on desktop.
        let footer = Rect::from_min_max(pos2(rect.left() + 14.0, rect.bottom() - 132.0), pos2(rect.right() - 14.0, rect.bottom() - 16.0));
        if !compact {
            let note = Rect::from_min_size(footer.min, vec2(footer.width(), 64.0));
            ui.painter().rect(note, CornerRadius::same(12), SURFACE, Stroke::new(1.0, BORDER), StrokeKind::Inside);
            let (icon, color, title, body) = match library.storage().warning() {
                Some(_) => (icons::WARNING, theme::WARNING, "Not saving", "Storage is unavailable"),
                None => (icons::LOCK_OPEN, theme::SUCCESS, "No account needed", "Your data stays on this device"),
            };
            ui.painter().text(note.left_center() + vec2(22.0, 0.0), Align2::CENTER_CENTER, icon, theme::regular(20.0), color);
            ui.painter().text(note.left_top() + vec2(42.0, 13.0), Align2::LEFT_TOP, title, theme::semibold(12.5), TEXT);
            ui.painter().text(note.left_top() + vec2(42.0, 33.0), Align2::LEFT_TOP, body, theme::regular(11.5), TEXT_DIM);
        }
        if cfg!(not(target_arch = "wasm32")) {
            let exit = Rect::from_min_size(pos2(footer.left(), footer.bottom() - 40.0), vec2(footer.width(), 40.0));
            ui.scope_builder(UiBuilder::new().max_rect(exit).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                if widgets::button(ui, ButtonKind::Ghost, icons::SIGN_OUT, if compact { "" } else { "Exit" }).clicked() {
                    actions.push(Action::Exit);
                }
            });
        }
        if !compact {
            ui.painter().text(pos2(rect.right() - 18.0, rect.bottom() - 36.0), Align2::RIGHT_CENTER, concat!("v", env!("CARGO_PKG_VERSION")), theme::regular(11.0), TEXT_FAINT);
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        ui.painter().text(rect.left_top() + vec2(36.0, 20.0), Align2::LEFT_TOP, self.page.label(), theme::bold(26.0), TEXT);
        ui.painter().text(rect.left_top() + vec2(37.0, 53.0), Align2::LEFT_TOP, self.page.subtitle(), theme::regular(13.0), TEXT_DIM);
        let wide = rect.width() > 760.0;
        let search_width = if wide { 300.0 } else { 190.0 };
        let right = Rect::from_min_max(pos2(rect.right() - search_width - 190.0, rect.top() + 18.0), pos2(rect.right() - 32.0, rect.top() + 60.0));
        ui.scope_builder(UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            if widgets::button(ui, ButtonKind::Primary, icons::PLUS, "Create").clicked() {
                self.go(Page::Create, now);
                self.new_project = Some(NewProject { name: String::new(), template: None });
            }
            ui.add_space(6.0);
            let before = self.search.clone();
            let response = widgets::search_box(ui, &mut self.search, "Search games", search_width);
            // Typing a query anywhere takes you to the search results.
            if self.search != before && !self.search.is_empty() && self.page != Page::Discover {
                self.go(Page::Discover, now);
                response.request_focus();
            }
        });
    }

    fn import_dropped_files(&mut self, ui: &Ui, library: &mut Library) {
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let queue = self.imports.clone();
            #[cfg(not(target_arch = "wasm32"))]
            queue.borrow_mut().push(file.bytes());
            // Browsers only expose dropped files asynchronously.
            #[cfg(target_arch = "wasm32")]
            wasm_bindgen_futures::spawn_local(async move {
                let bytes = file.bytes_async().await;
                queue.borrow_mut().push(bytes);
            });
        }
        let ready: Vec<Result<Vec<u8>, String>> = self.imports.borrow_mut().drain(..).collect();
        for bytes in ready {
            let text = bytes.ok().and_then(|bytes| String::from_utf8(bytes).ok());
            match text.ok_or_else(|| "the file could not be read".to_string()).and_then(|t| library.import_bundle(&t)) {
                Ok(key) => {
                    let name = library.find(&key).map_or_else(|| key.id.clone(), |e| e.manifest.name.clone());
                    self.toast(ToastKind::Success, format!("Imported “{name}”"));
                    self.page = Page::Library;
                    self.library_tab = 1;
                }
                Err(e) => self.toast(ToastKind::Error, format!("Import failed: {e}")),
            }
        }
    }

    /// Imports a bundle from a file path typed by the user (desktop).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_from_path(&mut self, library: &mut Library) {
        let path = self.import_path.trim().trim_matches('"').to_string();
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| library.import_bundle(&t)) {
            Ok(key) => {
                let name = library.find(&key).map_or_else(|| key.id.clone(), |e| e.manifest.name.clone());
                self.toast(ToastKind::Success, format!("Imported “{name}”"));
                self.import_path.clear();
            }
            Err(e) => self.toast(ToastKind::Error, format!("Import failed: {e}")),
        }
    }

    /// Exports a project: a download in the browser, a file in the data
    /// folder's `exports` directory on desktop.
    pub(crate) fn export(&mut self, library: &Library, key: &GameKey, actions: &mut Vec<Action>) {
        match library.export_project(key) {
            Ok(contents) => {
                let name = format!("{}.araria", key.id);
                if cfg!(target_arch = "wasm32") {
                    actions.push(Action::Download { name: name.clone(), contents });
                    self.toast(ToastKind::Success, format!("Downloading {name}"));
                } else {
                    match library.storage().write(&format!("exports/{name}"), contents.as_bytes()) {
                        Ok(()) => self.toast(ToastKind::Success, format!("Exported to {}/exports/{name}", library.storage().location())),
                        Err(e) => self.toast(ToastKind::Error, format!("Export failed: {e}")),
                    }
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Export failed: {e}")),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, library: &mut Library, actions: &mut Vec<Action>) {
        let frame = egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0, BORDER)).corner_radius(CornerRadius::same(18)).inner_margin(egui::Margin::same(0));
        let backdrop = theme::with_alpha(Color32::from_rgb(5, 6, 10), 0.72);

        if let Some(key) = self.detail.clone() {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("game-detail")).frame(frame).backdrop_color(backdrop).show(ctx, |ui| {
                close = pages::detail(ui, self, library, &key, actions);
            });
            if close || modal.should_close() {
                self.detail = None;
            }
        }

        if self.new_project.is_some() {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("new-project")).frame(frame.inner_margin(egui::Margin::same(28))).backdrop_color(backdrop).show(ctx, |ui| {
                close = pages::new_project_dialog(ui, self, library, actions);
            });
            if close || modal.should_close() {
                self.new_project = None;
            }
        }

        if let Some(key) = self.confirm_delete.clone() {
            let name = library.find(&key).map_or_else(|| key.id.clone(), |e| e.manifest.name.clone());
            let mut close = false;
            let modal = egui::Modal::new(Id::new("confirm-delete")).frame(frame.inner_margin(egui::Margin::same(28))).backdrop_color(backdrop).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(egui::RichText::new(if key.origin == Origin::Project { "Delete this project?" } else { "Uninstall this game?" }).font(theme::semibold(19.0)));
                let body = if key.origin == Origin::Project {
                    format!("“{name}” and all of its scenes, scripts and assets will be permanently removed from this device.")
                } else {
                    format!("“{name}” will be removed from this device. You can install it again from the Store at any time.")
                };
                ui.label(egui::RichText::new(body).color(TEXT_DIM));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let label = if key.origin == Origin::Project { "Delete" } else { "Uninstall" };
                    if widgets::button(ui, ButtonKind::Danger, icons::TRASH, label).clicked() {
                        if key.origin == Origin::Project {
                            library.delete_project(&key.id);
                        } else {
                            library.uninstall(&key.id);
                        }
                        self.thumbs.invalidate(&key);
                        self.toast(ToastKind::Info, format!("Removed “{name}”"));
                        self.detail = None;
                        close = true;
                    }
                    if widgets::button(ui, ButtonKind::Secondary, "", "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.confirm_delete = None;
            }
        }
    }

    fn draw_toasts(&mut self, ui: &Ui, full: Rect, now: f64) {
        const LIFETIME: f64 = 4.5;
        for toast in &mut self.toasts {
            if toast.born < 0.0 {
                toast.born = now;
            }
        }
        self.toasts.retain(|t| now - t.born < LIFETIME);
        if self.toasts.is_empty() {
            return;
        }
        ui.ctx().request_repaint();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("toasts")));
        let mut y = full.bottom() - 28.0;
        for toast in self.toasts.iter().rev() {
            let age = (now - toast.born) as f32;
            let fade = (age / 0.2).min(1.0) * ((LIFETIME as f32 - age) / 0.4).clamp(0.0, 1.0);
            let (icon, color) = match toast.kind {
                ToastKind::Info => (icons::INFO, ACCENT_HI),
                ToastKind::Success => (icons::CHECK, theme::SUCCESS),
                ToastKind::Error => (icons::WARNING, theme::DANGER),
            };
            let galley = painter.layout(toast.text.clone(), theme::medium(13.5), TEXT, 360.0);
            let size = galley.size() + vec2(64.0, 26.0);
            let rect = Rect::from_min_size(pos2(full.right() - size.x - 28.0 + 24.0 * (1.0 - fade), y - size.y), size);
            painter.rect(rect, CornerRadius::same(12), theme::with_alpha(SURFACE_HI, fade), Stroke::new(1.0, theme::with_alpha(color, 0.6 * fade)), StrokeKind::Inside);
            painter.text(rect.left_center() + vec2(24.0, 0.0), Align2::CENTER_CENTER, icon, theme::regular(19.0), theme::with_alpha(color, fade));
            painter.galley(rect.min + vec2(46.0, 13.0), galley, theme::with_alpha(TEXT, fade));
            y -= size.y + 10.0;
        }
    }
}

/// Small cover used in lists.
pub(crate) fn mini_cover(ui: &mut Ui, st: &mut PlatformUi, library: &Library, key: &GameKey, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if let Some(entry) = library.find(key) {
        cards::cover(ui, &mut st.thumbs, library, entry, rect, CornerRadius::same(theme::RADIUS_SMALL));
    }
}
