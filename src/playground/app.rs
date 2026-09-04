use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{
    build::BuildResult,
    readback::Element,
    run_menu,
    sources::{SourceSet, SourceSetKind},
};
use eframe::egui::{self, RichText, Theme};
use egui_code_editor::{CodeEditor, ColorTheme, Completer, Syntax};
use futures::task::Spawn;

#[cfg(target_family = "wasm")]
use crate::wasm_spawn::WasmSpawn;
use par_core::source::FileName;
#[cfg(not(target_family = "wasm"))]
use par_runtime::spawn::TokioSpawn;
use tokio_util::sync::CancellationToken;

pub struct Playground {
    sources: SourceSet,
    build: BuildResult,
    built_code: Arc<str>,
    editor_font_size: f32,
    show_compiled: bool,
    element: Option<Arc<Mutex<Element>>>,
    cursor_pos: (u32, u32),
    theme_mode: ThemeMode,
    #[cfg(not(target_family = "wasm"))]
    _rt: tokio::runtime::Runtime,
    spawner: Arc<dyn Spawn + Send + Sync + 'static>,
    cancel_token: Option<CancellationToken>,
    max_interactions: u32,
    #[cfg(target_family = "wasm")]
    pending_web_clipboard_paste: Arc<Mutex<Option<String>>>,
    completer: Completer,
    #[cfg(not(target_family = "wasm"))]
    open_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn is_dark(&self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::System, Self::Light, Self::Dark]
    }
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

impl Playground {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        file_path: Option<PathBuf>,
        max_interactions: u32,
    ) -> Box<Self> {
        let system_dark = cc
            .egui_ctx
            .input(|ri| ri.raw.system_theme.map(|t| t == Theme::Dark))
            .unwrap_or(false);
        let initial_is_dark = ThemeMode::default().is_dark(system_dark);

        let mut visuals = if initial_is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        apply_playground_visuals(&mut visuals);
        cc.egui_ctx.set_visuals(visuals);

        cc.egui_ctx.all_styles_mut(|style| {
            style.text_styles.extend([
                (egui::TextStyle::Monospace, egui::FontId::monospace(16.0)),
                (egui::TextStyle::Button, egui::FontId::proportional(18.0)),
                (egui::TextStyle::Body, egui::FontId::proportional(16.0)),
            ]);
            style.spacing.button_padding = egui::vec2(6.0, 3.0);
            style.visuals.code_bg_color = egui::Color32::TRANSPARENT;
            style.wrap_mode = Some(egui::TextWrapMode::Extend);
        });

        #[cfg(not(target_family = "wasm"))]
        let runtime =
            crate::tokio_factory::create_runtime().expect("Failed to create Tokio runtime");
        #[cfg(not(target_family = "wasm"))]
        let spawner = Arc::new(TokioSpawn::from_handle(runtime.handle().clone()));

        #[cfg(target_family = "wasm")]
        let spawner = Arc::new(WasmSpawn::new());

        let playground = Box::new(Self {
            sources: SourceSet::bundled_examples(),
            build: BuildResult::None,
            built_code: Arc::from(""),
            editor_font_size: 16.0,
            show_compiled: false,
            element: None,
            cursor_pos: (0, 0),
            theme_mode: ThemeMode::System,
            #[cfg(not(target_family = "wasm"))]
            _rt: runtime,
            spawner,
            cancel_token: None,
            max_interactions,
            #[cfg(target_family = "wasm")]
            pending_web_clipboard_paste: Arc::new(Mutex::new(None)),
            completer: Completer::new_with_syntax(&par_syntax()).with_auto_indent(),
            #[cfg(not(target_family = "wasm"))]
            open_error: None,
        });

        #[cfg(not(target_family = "wasm"))]
        {
            let mut playground = playground;
            if let Some(path) = file_path {
                playground.open(path);
            }
            playground
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = file_path;
            playground
        }
    }
}

impl eframe::App for Playground {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok(e) = &mut crate::CRASH_STR.try_lock() {
            **e = Some(self.sources.active_source().to_owned());
        }

        let system_dark = ui
            .input(|ri| ri.raw.system_theme.map(|t| t == egui::Theme::Dark))
            .unwrap_or(false);
        let is_dark = self.theme_mode.is_dark(system_dark);

        let mut visuals = if is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        apply_playground_visuals(&mut visuals);
        ui.set_visuals(visuals);

        #[cfg(target_family = "wasm")]
        self.handle_web_clipboard_shortcuts(ui.ctx());
        #[cfg(target_family = "wasm")]
        self.inject_pending_web_clipboard_paste(ui.ctx());

        self.sources.reload_active_if_changed();

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show_inside(ui, |ui| {
                let initial_editor_width = ui.available_width() / 2.0;
                egui::Panel::left("interaction")
                    .resizable(true)
                    .show_separator_line(true)
                    .default_size(initial_editor_width)
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| {
                        show_toolbar(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button(egui::RichText::new("-").monospace()).clicked() {
                                    self.editor_font_size = (self.editor_font_size - 1.0).max(8.0);
                                }
                                ui.label(
                                    egui::RichText::new(self.editor_font_size.to_string()).strong(),
                                );
                                if ui.button(egui::RichText::new("+").monospace()).clicked() {
                                    self.editor_font_size =
                                        (self.editor_font_size + 1.0).min(320.0);
                                }

                                ui.add_space(5.0);

                                #[cfg(not(target_family = "wasm"))]
                                self.show_file_menu(ui);

                                ui.add_space(5.0);

                                egui::containers::menu::MenuButton::from_button(egui::Button::new(
                                    egui::RichText::new("Theme").strong(),
                                ))
                                .ui(ui, |ui| {
                                    for &mode in ThemeMode::all() {
                                        if ui
                                            .radio(self.theme_mode == mode, mode.display_name())
                                            .clicked()
                                        {
                                            self.theme_mode = mode;
                                            ui.close();
                                        }
                                    }
                                });

                                ui.add_space(5.0);

                                self.show_source_menu(ui);
                            });
                        });

                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                left: 8,
                                right: 0,
                                top: 0,
                                bottom: 0,
                            })
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let editor = CodeEditor::default()
                                        .id_source("code")
                                        .with_syntax(par_syntax())
                                        .with_rows(32)
                                        .with_fontsize(self.editor_font_size)
                                        .with_theme(self.get_theme(ui))
                                        .with_numlines(true)
                                        .show_with_completer(
                                            ui,
                                            self.sources.active_source_mut(),
                                            &mut self.completer,
                                        );

                                    if let Some(cursor) = editor.cursor_range {
                                        self.cursor_pos = row_and_column(
                                            self.sources.active_source(),
                                            cursor.primary.index,
                                        );
                                    }

                                    if let (Some(checked), Some(hover_pos)) =
                                        (self.build.checked(), editor_hover_pos(&editor))
                                    {
                                        let hover_file_name = self.active_file_name();
                                        if let Some(name_info) = checked.hover_at(
                                            &hover_file_name,
                                            hover_pos.0,
                                            hover_pos.1,
                                        ) {
                                            let signature = checked.render_hover_signature_in_file(
                                                &hover_file_name,
                                                &name_info,
                                            );
                                            editor.response.response.on_hover_ui_at_pointer(|ui| {
                                                ui.label(RichText::new(signature).code());
                                                if let Some(doc) = name_info.doc() {
                                                    ui.separator();
                                                    ui.label(doc.markdown.as_str());
                                                }
                                            });
                                        }
                                    }
                                });
                            });
                    });

                self.show_interaction(ui);
            });

        #[cfg(not(target_family = "wasm"))]
        self.show_open_error_dialog(ui.ctx());
    }
}

fn row_and_column(source: &str, index: usize) -> (u32, u32) {
    let (mut row, mut col) = (0, 0);
    assert!(u32::try_from(index).is_ok(), "file size is too large");
    for c in source.chars().take(index) {
        if c == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (row, col)
}

fn editor_hover_pos(output: &egui::text_edit::TextEditOutput) -> Option<(u32, u32)> {
    let hover_pos = output.response.hover_pos()?;
    let galley_rect = egui::Rect::from_min_size(output.galley_pos, output.galley.size());
    if !galley_rect.contains(hover_pos) {
        return None;
    }

    let hover_cursor = output.galley.cursor_from_pos(hover_pos - output.galley_pos);
    Some(row_and_column(&output.galley.job.text, hover_cursor.index))
}

fn show_toolbar(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let previous_item_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;

    egui::Frame::new()
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });

    ui.spacing_mut().item_spacing = previous_item_spacing;
}

fn apply_playground_visuals(visuals: &mut egui::Visuals) {
    visuals.code_bg_color = egui::Color32::TRANSPARENT;

    let button_stroke_color = if visuals.dark_mode {
        egui::Color32::from_gray(82)
    } else {
        egui::Color32::from_gray(194)
    };
    let button_stroke = egui::Stroke::new(1.0_f32, button_stroke_color);
    let button_radius = egui::CornerRadius::same(3);

    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_stroke = button_stroke;
        widget.corner_radius = button_radius;
    }
}

fn tint_button_visuals(visuals: &mut egui::Visuals, fill: egui::Color32) {
    let inactive = visuals.widgets.inactive.weak_bg_fill;
    let hovered = visuals.widgets.hovered.weak_bg_fill;
    let active = visuals.widgets.active.weak_bg_fill;
    let open = visuals.widgets.open.weak_bg_fill;

    visuals.widgets.inactive.weak_bg_fill = fill;
    visuals.widgets.hovered.weak_bg_fill = shifted_tint(fill, inactive, hovered);
    visuals.widgets.active.weak_bg_fill = shifted_tint(fill, inactive, active);
    visuals.widgets.open.weak_bg_fill = shifted_tint(fill, inactive, open);
}

fn shifted_tint(
    fill: egui::Color32,
    inactive: egui::Color32,
    target: egui::Color32,
) -> egui::Color32 {
    let delta = luminance(target) - luminance(inactive);
    if delta.abs() < 0.01 {
        return fill;
    }

    let amount = (delta.abs() * 1.5).clamp(0.08, 0.22);
    if delta.is_sign_positive() {
        fill.lerp_to_gamma(egui::Color32::WHITE, amount)
    } else {
        fill.lerp_to_gamma(egui::Color32::BLACK, amount)
    }
}

fn luminance(color: egui::Color32) -> f32 {
    let [r, g, b, _] = color.to_array();
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

impl Playground {
    #[cfg(target_family = "wasm")]
    fn handle_web_clipboard_shortcuts(&self, ctx: &egui::Context) {
        let mut needs_copy = false;
        let mut needs_cut = false;
        let mut needs_paste = false;

        ctx.input(|input| {
            let has_copy = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy));
            let has_cut = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Cut));
            let has_paste = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)));

            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };

                if !modifiers.command {
                    continue;
                }

                match key {
                    egui::Key::C => needs_copy = !has_copy,
                    egui::Key::X => needs_cut = !has_cut,
                    egui::Key::V => needs_paste = !has_paste,
                    _ => {}
                }
            }
        });

        if needs_copy || needs_cut {
            ctx.input_mut(|input| {
                if needs_copy {
                    input.events.push(egui::Event::Copy);
                }
                if needs_cut {
                    input.events.push(egui::Event::Cut);
                }
            });
        }

        if needs_paste {
            self.request_web_clipboard_paste(ctx);
        }
    }

    #[cfg(target_family = "wasm")]
    fn request_web_clipboard_paste(&self, ctx: &egui::Context) {
        let pending_paste = Arc::clone(&self.pending_web_clipboard_paste);
        let ctx = ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let Some(text) = read_web_clipboard_text().await else {
                return;
            };
            if text.is_empty() {
                return;
            }

            *pending_paste.lock().unwrap() = Some(text);
            ctx.request_repaint();
        });
    }

    #[cfg(target_family = "wasm")]
    fn inject_pending_web_clipboard_paste(&self, ctx: &egui::Context) {
        let Some(text) = self.pending_web_clipboard_paste.lock().unwrap().take() else {
            return;
        };

        ctx.input_mut(|input| {
            input.events.push(egui::Event::Paste(text));
        });
    }

    #[cfg(not(target_family = "wasm"))]
    fn show_file_menu(&mut self, ui: &mut egui::Ui) {
        egui::containers::menu::MenuButton::from_button(egui::Button::new(
            egui::RichText::new("File").strong(),
        ))
        .ui(ui, |ui| {
            if ui.button(egui::RichText::new("Open...").strong()).clicked() {
                self.open_file();
                ui.close();
            }

            if matches!(self.sources.kind(), SourceSetKind::DiskPackage) {
                if ui
                    .add_enabled(
                        self.sources.can_save_active(),
                        egui::Button::new(egui::RichText::new("Save").strong()),
                    )
                    .clicked()
                {
                    let _ = self.sources.save_active();
                    ui.close();
                }

                let mut do_reload = self.sources.active_reload_enabled();
                if ui
                    .checkbox(&mut do_reload, egui::RichText::new("Reload").strong())
                    .clicked()
                {
                    self.sources.set_active_reload_enabled(do_reload);
                    ui.close();
                }
            }
        });
    }

    fn show_source_menu(&mut self, ui: &mut egui::Ui) {
        let response = ui
            .scope(|ui| {
                tint_button_visuals(
                    ui.visuals_mut(),
                    blue().lerp_to_gamma(egui::Color32::WHITE, 0.55),
                );
                let (response, _) = egui::containers::menu::MenuButton::from_button(
                    egui::Button::new(
                        RichText::new(self.sources.active_label())
                            .strong()
                            .color(egui::Color32::BLACK),
                    )
                    .right_text(RichText::new("v").color(egui::Color32::TRANSPARENT)),
                )
                .ui(ui, |ui| {
                    for index in 0..self.sources.buffer_count() {
                        let label = self.sources.buffer_label(index);
                        if ui
                            .selectable_label(self.sources.is_active(index), label)
                            .clicked()
                        {
                            self.switch_to_source(index);
                            ui.close();
                        }
                    }
                });
                response
            })
            .inner;

        paint_dropdown_arrow(ui, response.rect, egui::Color32::BLACK);
    }

    fn switch_to_source(&mut self, index: usize) {
        self.sources.set_active(index);
    }

    #[cfg(not(target_family = "wasm"))]
    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            self.open(path);
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn open(&mut self, file_path: PathBuf) {
        match SourceSet::open_disk(file_path) {
            Ok(sources) => {
                self.sources = sources;
                self.clear_build_and_interaction();
                self.open_error = None;
            }
            Err(message) => {
                self.open_error = Some(message);
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn show_open_error_dialog(&mut self, ctx: &egui::Context) {
        let Some(message) = self.open_error.clone() else {
            return;
        };
        let mut close = false;

        egui::Window::new("Could not open file")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(message);
                ui.add_space(8.0);
                if ui.button(egui::RichText::new("OK").strong()).clicked() {
                    close = true;
                }
            });

        if close {
            self.open_error = None;
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn clear_build_and_interaction(&mut self) {
        self.cancel_interaction();
        self.build = BuildResult::None;
        self.built_code = Arc::from("");
    }

    #[cfg(not(target_family = "wasm"))]
    fn cancel_interaction(&mut self) {
        if let Some(cancel_token) = self.cancel_token.take() {
            cancel_token.cancel();
        }
        self.element = None;
    }

    fn get_theme(&self, ui: &egui::Ui) -> ColorTheme {
        let is_dark = self.theme_mode.is_dark(ui.visuals().dark_mode);

        if is_dark {
            fix_dark_theme(ColorTheme::GRUVBOX_DARK)
        } else {
            fix_light_theme(ColorTheme::GITHUB_LIGHT)
        }
    }

    fn active_file_name(&self) -> FileName {
        self.sources.active_file_name()
    }

    fn recompile(&mut self) {
        self.build = match self.sources.kind() {
            SourceSetKind::BundledExamples => BuildResult::from_loaded_package(
                self.sources.loaded_files(),
                SourceSet::bundled_package_id(),
                self.max_interactions,
            ),
            #[cfg(not(target_family = "wasm"))]
            SourceSetKind::DiskPackage => {
                let Some(active_path) = self.sources.active_disk_path() else {
                    self.built_code = Arc::from(self.sources.active_source());
                    return;
                };
                BuildResult::from_package_with_overrides(
                    active_path,
                    self.sources.source_overrides(),
                    self.max_interactions,
                )
            }
        };
        self.built_code = Arc::from(self.sources.active_source());
    }

    fn show_interaction(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            show_toolbar(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Compile").strong()).clicked() {
                        self.recompile();
                    }

                    if self.build.pretty().is_some() {
                        if let (Some(checked), Some(rt_compiled)) =
                            (self.build.checked(), self.build.rt_compiled())
                        {
                            let active_file = self.active_file_name();
                            let spawner = self.spawner.clone();
                            let cancel_token = &mut self.cancel_token;
                            let element = &mut self.element;
                            let name_to_ty = &rt_compiled.name_to_ty;
                            ui.scope(|ui| {
                                tint_button_visuals(
                                    ui.visuals_mut(),
                                    green().lerp_to_gamma(egui::Color32::WHITE, 0.3),
                                );
                                egui::containers::menu::MenuButton::from_button(egui::Button::new(
                                    egui::RichText::new("Run")
                                        .strong()
                                        .color(egui::Color32::BLACK),
                                ))
                                .ui(ui, |ui| {
                                    egui::ScrollArea::vertical().show(ui, |ui| {
                                        run_menu::show_run_menu(
                                            spawner.clone(),
                                            cancel_token,
                                            element,
                                            ui,
                                            &active_file,
                                            checked.clone(),
                                            rt_compiled,
                                            name_to_ty,
                                        );
                                    })
                                });
                            });
                        }

                        ui.checkbox(
                            &mut self.show_compiled,
                            egui::RichText::new("Show compiled"),
                        );
                    }
                });
            });

            egui::CentralPanel::default().show_inside(ui, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    if let Some(error) = self.build.error() {
                        ui.label(
                            egui::RichText::new(error.display(self.built_code.clone()))
                                .color(red())
                                .code(),
                        );
                    }

                    if let Some(warnings) = self.build.warnings() {
                        ui.label(
                            egui::RichText::new(warnings.display(self.built_code.clone()))
                                .color(yellow())
                                .code(),
                        );
                    }

                    let theme = self.get_theme(ui);

                    if self.show_compiled {
                        if let Some(mut pretty) =
                            self.build.pretty_for_file(&self.active_file_name())
                        {
                            CodeEditor::default()
                                .id_source("compiled")
                                .with_syntax(par_syntax())
                                .with_rows(32)
                                .with_fontsize(self.editor_font_size)
                                .with_theme(theme)
                                .with_numlines(true)
                                .show(ui, &mut pretty);
                        }
                    }

                    if !self.show_compiled {
                        if let Some(element) = &mut self.element {
                            element.lock().unwrap().show(ui);
                        }
                    }
                });
            });
        });
    }
}

#[cfg(target_family = "wasm")]
async fn read_web_clipboard_text() -> Option<String> {
    let window = web_sys::window()?;
    let clipboard = window.navigator().clipboard();
    let text = wasm_bindgen_futures::JsFuture::from(clipboard.read_text())
        .await
        .ok()?
        .as_string()?;
    Some(text.replace("\r\n", "\n"))
}

fn par_syntax() -> Syntax {
    Syntax {
        language: "Par",
        case_sensitive: true,
        quotes: BTreeSet::from(['"', '`']),
        comment: "//",
        comment_multiline: [r#"/*"#, r#"*/"#],
        hyperlinks: BTreeSet::from([]),
        keywords: BTreeSet::from([
            "block",
            "goto",
            "dec",
            "def",
            "type",
            "chan",
            "dual",
            "let",
            "do",
            "in",
            "case",
            "begin",
            "unfounded",
            "unbox",
            "loop",
            "module",
            "include",
            "import",
            "as",
            "export",
            "either",
            "choice",
            "recursive",
            "iterative",
            "self",
            "box",
            "catch",
            "try",
            "throw",
            "default",
            "else",
            "if",
            "is",
            "and",
            "or",
            "not",
            "poll",
            "repoll",
            "submit",
            "todo",
        ]),
        types: BTreeSet::from([]),
        special: BTreeSet::from(["<>"]),
    }
}

fn fix_dark_theme(mut theme: ColorTheme) -> ColorTheme {
    theme.bg = "1F1F1F";
    theme.functions = theme.literals;
    theme
}

fn fix_light_theme(mut theme: ColorTheme) -> ColorTheme {
    theme.bg = "F9F9F9";
    theme.functions = theme.literals;
    theme
}

#[allow(unused)]
fn red() -> egui::Color32 {
    egui::Color32::from_hex("#DE3C4B").unwrap()
}

#[allow(unused)]
fn yellow() -> egui::Color32 {
    egui::Color32::from_hex("#e09f3e").unwrap()
}

#[allow(unused)]
fn green() -> egui::Color32 {
    egui::Color32::from_hex("#7ac74f").unwrap()
}

#[allow(unused)]
fn blue() -> egui::Color32 {
    egui::Color32::from_hex("#118ab2").unwrap()
}

fn paint_dropdown_arrow(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let center = egui::pos2(rect.right() - 11.0, rect.center().y + 1.0);
    let points = vec![
        egui::pos2(center.x - 4.0, center.y - 2.5),
        egui::pos2(center.x + 4.0, center.y - 2.5),
        egui::pos2(center.x, center.y + 2.5),
    ];
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}
