//! The egui front end: endless practice loop with copy and translate modes.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, FontId, Key, Modifiers, RichText, TextFormat, TextureHandle, TextureOptions,
    text::LayoutJob,
};

use crate::engine::{self, Style};
use crate::generator::{Formula, Generator};
use crate::stats::{Attempt, Mode, Profile};
use crate::storage;

/// Zoom of the prompt formula relative to the live preview.
const TARGET_ZOOM: f32 = 1.6;
const GREEN: Color32 = Color32::from_rgb(80, 190, 110);
const RED: Color32 = Color32::from_rgb(230, 80, 80);
const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
/// Without any input for this long, the user counts as away and the time of
/// the current formula is excluded from the statistics.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// When to start warning that the time is about to be excluded.
const IDLE_WARNING: Duration = Duration::from_secs(20);

/// Whether a formula is on screen. The clock only runs while it is.
enum Phase {
    /// Waiting for Space; the formula is hidden.
    Ready,
    /// A formula is on screen. `idle` is set for good once the player went
    /// idle on it.
    Active { shown: Instant, last_input: Instant, idle: bool },
}

enum Feedback {
    None,
    Wrong(String),
    Correct(String),
}

/// A rendered formula texture, re-rendered when its key changes.
#[derive(Default)]
struct Rendered {
    key: Option<(String, [u8; 3], u32)>,
    result: Option<Result<TextureHandle, String>>,
}

impl Rendered {
    fn get(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        formula: &str,
        color: [u8; 3],
        px_per_pt: f32,
    ) -> &Result<TextureHandle, String> {
        let key = (formula.to_string(), color, px_per_pt.to_bits());
        if self.key.as_ref() != Some(&key) || self.result.is_none() {
            let style = Style { color, px_per_pt, lenient: false };
            let result = engine::render(formula, &style).map(|img| {
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [img.width as usize, img.height as usize],
                    &img.rgba,
                );
                ctx.load_texture(name, image, TextureOptions::LINEAR)
            });
            self.key = Some(key);
            self.result = Some(result);
        }
        self.result.as_ref().unwrap()
    }
}

#[derive(Default)]
struct Session {
    solved: u32,
    chars: usize,
    time_ms: u64,
    wrong_submits: u32,
    revealed: u32,
}

pub struct App {
    profile: Profile,
    generator: Generator,
    current: Formula,
    input: String,
    phase: Phase,
    /// Something was typed for the current formula.
    typed: bool,
    wrong_submits: u32,
    backspaces: u32,
    revealed: bool,
    feedback: Feedback,
    target: Rendered,
    preview: Rendered,
    session: Session,
    show_stats: bool,
    save_error: Option<String>,
    sync_input: String,
    sync_status: Option<Result<String, String>>,
}

impl App {
    pub fn new() -> Self {
        let profile = Profile::load();
        let mut generator = Generator::new();
        let mode = profile.config.settings.mode;
        let sync_input = profile.config.sync_dir.as_deref().map(display_path).unwrap_or_default();
        let current = generator.next(profile.level(mode), &profile.weakness(mode), "");
        Self {
            profile,
            generator,
            current,
            input: String::new(),
            phase: Phase::Ready,
            typed: false,
            wrong_submits: 0,
            backspaces: 0,
            revealed: false,
            feedback: Feedback::None,
            target: Rendered::default(),
            preview: Rendered::default(),
            session: Session::default(),
            show_stats: false,
            save_error: None,
            sync_input,
            sync_status: None,
        }
    }

    fn mode(&self) -> Mode {
        self.profile.config.settings.mode
    }

    fn next_formula(&mut self) {
        // Pick up attempts synced in from other devices.
        self.profile.reload_if_changed();
        let mode = self.mode();
        self.current = self.generator.next(
            self.profile.level(mode),
            &self.profile.weakness(mode),
            &self.current.source,
        );
        self.input.clear();
        self.phase = Phase::Ready;
        self.typed = false;
        self.wrong_submits = 0;
        self.backspaces = 0;
        self.revealed = false;
    }

    fn record(&mut self, skipped: bool) -> Attempt {
        let attempt = Attempt {
            timestamp: Attempt::now(),
            mode: self.mode(),
            source: self.current.source.clone(),
            tags: self.current.tags.iter().copied().collect(),
            chars: self.current.source.chars().count(),
            time_ms: match self.phase {
                Phase::Active { shown, .. } => shown.elapsed().as_millis() as u64,
                Phase::Ready => 0,
            },
            wrong_submits: self.wrong_submits,
            backspaces: self.backspaces,
            revealed: self.revealed,
            skipped,
            idle: matches!(self.phase, Phase::Active { idle: true, .. }),
        };
        self.save_error = self.profile.record(attempt.clone()).err();
        attempt
    }

    fn save(&mut self) {
        self.save_error = self.profile.save_settings().err();
    }

    fn submit(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        let lenient = !self.profile.config.settings.strict_din;
        match engine::equivalent(&self.current.source, &self.input, lenient) {
            Ok(true) => {
                let attempt = self.record(false);
                self.session.solved += 1;
                self.session.wrong_submits += attempt.wrong_submits;
                if attempt.revealed {
                    self.session.revealed += 1;
                }
                let mut msg = String::from("✓");
                if let Some(cpm) = attempt.cpm() {
                    self.session.chars += attempt.chars;
                    self.session.time_ms += attempt.time_ms;
                    msg += &format!(" {:.1} s · {cpm:.0} chars/min", attempt.time_ms as f32 / 1000.0);
                } else if attempt.idle {
                    msg += " not timed (idle)";
                } else {
                    msg += &format!(" {:.1} s", attempt.time_ms as f32 / 1000.0);
                }
                if attempt.wrong_submits > 0 {
                    msg += &format!(" · {} wrong", attempt.wrong_submits);
                }
                self.feedback = Feedback::Correct(msg);
                self.next_formula();
            }
            Ok(false) => {
                self.wrong_submits += 1;
                self.feedback =
                    Feedback::Wrong("✗ Renders differently – compare with the preview.".into());
            }
            Err(e) => {
                self.wrong_submits += 1;
                self.feedback = Feedback::Wrong(format!("✗ {e}"));
            }
        }
    }

    fn skip(&mut self) {
        // Only count a skip if the formula was actually attempted.
        if self.typed || self.revealed {
            self.record(true);
        }
        self.feedback = Feedback::Wrong(format!("Skipped: {}", self.current.source));
        self.next_formula();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("FasterTypst");
            ui.separator();
            let before = self.mode();
            let settings = &mut self.profile.config.settings;
            ui.selectable_value(&mut settings.mode, Mode::Translate, "Translate")
                .on_hover_text("See the rendered formula, write the Typst source.");
            ui.selectable_value(&mut settings.mode, Mode::Copy, "Copy")
                .on_hover_text("See the Typst source, type it as fast as possible.");
            ui.separator();
            let strict = ui
                .checkbox(&mut settings.strict_din, "Strict DIN")
                .on_hover_text("Require upright d (dif), j and e. Off: `d x` counts as `dif x`.");
            let preview = ui.checkbox(&mut settings.live_preview, "Live preview");
            let changed_mode = self.profile.config.settings.mode != before;
            if changed_mode {
                self.feedback = Feedback::None;
                self.next_formula();
            }
            if changed_mode || strict.changed() || preview.changed() {
                self.save();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.toggle_value(&mut self.show_stats, "📊 Statistics").clicked() && self.show_stats {
                    self.profile.reload_if_changed();
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let s = &self.session;
            let cpm = if s.time_ms > 0 { s.chars as f32 / (s.time_ms as f32 / 60_000.0) } else { 0.0 };
            ui.label(format!(
                "Session: {} solved · {cpm:.0} chars/min · {} wrong · {} revealed",
                s.solved, s.wrong_submits, s.revealed
            ));
            ui.separator();
            ui.label(format!("Level {:.1}", self.profile.level(self.mode())));
            ui.separator();
            ui.weak("Space next formula · Enter submit · Tab reveal solution · Esc skip");
            if let Some(e) = &self.save_error {
                ui.colored_label(RED, format!("Could not save: {e}"));
            }
        });
    }

    fn copy_prompt(&self, ui: &mut egui::Ui) {
        let font = FontId::monospace(22.0);
        let src: Vec<char> = self.current.source.chars().collect();
        let typed: Vec<char> = self.input.chars().collect();
        let matched = src.iter().zip(&typed).take_while(|(a, b)| a == b).count();
        let mismatch = typed.len() > matched;
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let mut job = LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        let mut push = |s: String, color: Color32, bg: Color32| {
            job.append(&s, 0.0, TextFormat { font_id: font.clone(), color, background: bg, ..Default::default() });
        };
        push(src[..matched].iter().collect(), GREEN, Color32::TRANSPARENT);
        if matched < src.len() {
            let (color, bg) = if mismatch { (Color32::WHITE, RED) } else { (text, Color32::TRANSPARENT) };
            push(src[matched].to_string(), color, bg);
            push(src[matched + 1..].iter().collect(), weak, Color32::TRANSPARENT);
        }
        ui.label(job);
    }

    fn feedback_label(&self, ui: &mut egui::Ui) {
        match &self.feedback {
            Feedback::None => {}
            Feedback::Wrong(msg) => drop(ui.colored_label(RED, msg)),
            Feedback::Correct(msg) => drop(ui.colored_label(GREEN, msg)),
        }
    }

    fn ready_screen(&self, ui: &mut egui::Ui) {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            self.feedback_label(ui);
            ui.add_space(24.0);
            ui.label(RichText::new("Press Space to show the next formula").size(24.0));
            ui.add_space(4.0);
            ui.weak("The clock only runs while a formula is shown.");
        });
    }

    /// Marks the current attempt as idle once there was no input for
    /// `IDLE_TIMEOUT`, so none of its time ends up in the statistics.
    fn check_idle(&mut self, ctx: &egui::Context) {
        let Phase::Active { last_input, idle, .. } = &mut self.phase else { return };
        if *idle {
            return;
        }
        let now = Instant::now();
        // Measure the gap before counting this frame's input, so the key that
        // wakes up a stalled window cannot hide the time away.
        if now - *last_input >= IDLE_TIMEOUT {
            *idle = true;
            return;
        }
        let active = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Ime(_)
                        | egui::Event::PointerMoved(_)
                        | egui::Event::PointerButton { .. }
                        | egui::Event::MouseWheel { .. }
                )
            })
        });
        if active {
            *last_input = now;
        }
        // Wake up for the warning and the timeout even without any input.
        let since = now - *last_input;
        let next = if since < IDLE_WARNING { IDLE_WARNING - since } else { Duration::from_secs(1) };
        ctx.request_repaint_after(next.min(IDLE_TIMEOUT - since));
    }

    /// In the ready phase, Space shows the next formula and starts the clock.
    fn handle_space(&mut self, ctx: &egui::Context) {
        if !matches!(self.phase, Phase::Ready) || self.editing_sync_dir(ctx) {
            return;
        }
        let pressed = ctx.input_mut(|i| {
            let pressed = i.consume_key(Modifiers::NONE, Key::Space);
            if pressed {
                // Don't let the space end up in the input field.
                i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
            }
            pressed
        });
        if pressed {
            let now = Instant::now();
            self.phase = Phase::Active { shown: now, last_input: now, idle: false };
            self.feedback = Feedback::None;
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        if matches!(self.phase, Phase::Ready) {
            self.ready_screen(ui);
            return;
        }
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let c = ui.visuals().strong_text_color();
        let color = [c.r(), c.g(), c.b()];

        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.weak(self.current.topic.label());
            for tag in &self.current.tags {
                ui.weak(format!("· {}", tag.label()));
            }
        });
        ui.add_space(12.0);

        // Prompt.
        ui.vertical_centered(|ui| {
            match self.target.get(&ctx, "target", &self.current.source, color, ppp * TARGET_ZOOM) {
                Ok(tex) => formula_image(ui, tex, ppp),
                Err(e) => {
                    ui.colored_label(RED, format!("Generator bug, please skip: {e}"));
                }
            }
            ui.add_space(12.0);
            if self.mode() == Mode::Copy {
                self.copy_prompt(ui);
            } else if self.revealed {
                ui.label(RichText::new(&self.current.source).monospace().size(18.0));
            }
        });
        ui.add_space(20.0);

        // Input.
        let editing_sync_dir = self.editing_sync_dir(&ctx);
        let (tab, esc) = if editing_sync_dir {
            (false, false)
        } else {
            ui.input_mut(|i| {
                (i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::NONE, Key::Escape))
            })
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .font(FontId::monospace(20.0))
                .desired_width(f32::INFINITY)
                .hint_text("Typst math, e.g.  underline(Z) = R + upright(j) omega L"),
        );
        if response.has_focus() && ui.input(|i| i.key_pressed(Key::Backspace)) {
            self.backspaces += 1;
        }
        if response.changed() && !self.input.is_empty() {
            self.typed = true;
        }
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        // Only when needed: `request_focus` interrupts the IME, which on
        // Wayland re-enables text input and drops keystrokes.
        if !response.has_focus() && !editing_sync_dir {
            response.request_focus();
        }
        if tab && self.mode() == Mode::Translate {
            self.revealed = true;
        }
        if esc {
            self.skip();
        }
        if enter {
            self.submit();
        }

        // Feedback.
        ui.add_space(8.0);
        self.feedback_label(ui);
        match self.phase {
            Phase::Active { idle: true, .. } => {
                ui.colored_label(
                    AMBER,
                    format!(
                        "⏸ Idle for {} s – this formula is not timed and won't affect your speed statistics.",
                        IDLE_TIMEOUT.as_secs()
                    ),
                );
            }
            Phase::Active { last_input, .. } if last_input.elapsed() >= IDLE_WARNING => {
                let left = IDLE_TIMEOUT.saturating_sub(last_input.elapsed()).as_secs_f32().ceil();
                ui.weak(format!("No input for a while – timing stops in {left:.0} s."));
            }
            _ => {}
        }

        // Live preview (also shown after a wrong submission).
        let show_preview = self.profile.config.settings.live_preview
            || (self.wrong_submits > 0 && !self.input.is_empty());
        if show_preview && !self.input.trim().is_empty() {
            ui.add_space(16.0);
            ui.weak("Your input:");
            ui.vertical_centered(|ui| match self.preview.get(&ctx, "preview", &self.input, color, ppp) {
                Ok(tex) => formula_image(ui, tex, ppp),
                Err(e) => drop(ui.weak(e.as_str())),
            });
        }
    }

    fn editing_sync_dir(&self, ctx: &egui::Context) -> bool {
        self.show_stats && ctx.memory(|m| m.has_focus(sync_field_id()))
    }

    fn stats_window(&mut self, ctx: &egui::Context) {
        let mode = self.mode();
        let profile = &self.profile;
        let sync_input = &mut self.sync_input;
        let sync_status = &self.sync_status;
        let mut sync_action: Option<Option<std::path::PathBuf>> = None;
        egui::Window::new("Statistics").open(&mut self.show_stats).default_width(520.0).show(ctx, |ui| {
            let n = profile.attempts.iter().filter(|a| a.mode == mode).count();
            ui.label(format!(
                "{mode:?} mode: {n} attempts · level {:.1} · average {:.0} chars/min",
                profile.level(mode),
                profile.average_cpm(mode).unwrap_or(0.0)
            ));
            ui.add_space(8.0);
            ui.strong("Constructs, weakest first (these get practiced more):");
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                egui::Grid::new("tags").striped(true).num_columns(5).show(ui, |ui| {
                    ui.strong("Construct");
                    ui.strong("Seen");
                    ui.strong("chars/min");
                    ui.strong("Errors");
                    ui.strong("Weakness");
                    ui.end_row();
                    for s in profile.tag_stats(mode) {
                        ui.label(s.tag.label());
                        ui.label(s.samples.to_string());
                        ui.label(s.cpm.map(|c| format!("{c:.0}")).unwrap_or("–".into()));
                        ui.label(format!("{:.2}", s.error_rate));
                        ui.add(
                            egui::ProgressBar::new((s.weakness / 2.0).min(1.0))
                                .desired_width(120.0)
                                .text(format!("{:.2}", s.weakness)),
                        );
                        ui.end_row();
                    }
                });
            });

            ui.separator();
            ui.strong("Sync between devices");
            ui.label(
                "Set every device to the same folder inside a synced folder (Nextcloud, \
                 Syncthing, …). Each device only writes its own file there, so syncing \
                 never conflicts, even offline.",
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(sync_input)
                        .id(sync_field_id())
                        .hint_text("~/Nextcloud/FasterTypst")
                        .desired_width(260.0),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.button("Use folder").clicked() || entered) && !sync_input.trim().is_empty() {
                    sync_action = Some(Some(storage::expand_home(sync_input)));
                }
                if profile.config.sync_dir.is_some() && ui.button("Stop syncing").clicked() {
                    sync_action = Some(None);
                }
            });
            ui.weak(format!("Data folder: {}", display_path(&profile.config.data_dir())));
            ui.weak(format!(
                "This device: {} · attempts per device: {}",
                profile.config.device,
                if profile.devices.is_empty() {
                    "none yet".to_string()
                } else {
                    profile.devices.iter().map(|(d, n)| format!("{d} ({n})")).collect::<Vec<_>>().join(", ")
                }
            ));
            match sync_status {
                Some(Ok(msg)) => drop(ui.colored_label(GREEN, msg.as_str())),
                Some(Err(msg)) => drop(ui.colored_label(RED, msg.as_str())),
                None => {}
            }
        });
        if let Some(dir) = sync_action {
            let target = dir.as_deref().map(display_path);
            self.sync_status = Some(match (self.profile.set_sync_dir(dir), target) {
                (Ok(()), Some(target)) => Ok(format!("Syncing via {target}")),
                (Ok(()), None) => {
                    self.sync_input.clear();
                    Ok("Stopped syncing; this device's attempts are stored locally again.".into())
                }
                (Err(e), _) => Err(format!("Could not switch folder: {e}")),
            });
            self.next_formula_if_ready();
        }
    }

    /// Regenerates the hidden next formula, e.g. after the level changed.
    fn next_formula_if_ready(&mut self) {
        if matches!(self.phase, Phase::Ready) {
            self.next_formula();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_space(&ctx);
        // After `handle_space`, so the idle wake-up is scheduled from the
        // frame the formula appears in.
        self.check_idle(&ctx);
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(4.0);
            self.bottom_bar(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.central(ui));
        });
        if self.show_stats {
            let ctx = ui.ctx().clone();
            self.stats_window(&ctx);
        }
    }
}

/// Shows a rendered formula at its natural size, shrunk if wider than the
/// available space.
fn formula_image(ui: &mut egui::Ui, tex: &TextureHandle, ppp: f32) {
    ui.add(
        egui::Image::from_texture((tex.id(), tex.size_vec2() / ppp))
            .fit_to_original_size(1.0)
            .max_width(ui.available_width()),
    );
}

fn sync_field_id() -> egui::Id {
    egui::Id::new("sync_dir_field")
}

/// Shows paths under the home directory as `~/...`.
fn display_path(path: &std::path::Path) -> String {
    match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(|p| p.to_path_buf())) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
