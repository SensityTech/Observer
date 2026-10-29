//! The status window. It opens on its own thread at every start of `observer run`, and again
//! from the tray after it was closed: closing it leaves Observer running.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Painter, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Theme, Ui, Vec2, ViewportCommand,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::server::{ServerState, ServerStatus};
use tracing::warn;

use super::palette::{self, CARD_RADIUS, CONTROL_RADIUS, Led, Palette};
use super::{Action, Confirm, Request, Shared, controls, is_running, restarting, text};

const WIDTH: f32 = 440.0;
const PADDING: i8 = 20;
const ICON: &[u8] = include_bytes!("../../assets/mark-64.rgba");

pub(super) fn spawn(shared: Arc<Shared>, requests: Receiver<Request>) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("observer-window".to_owned())
        .spawn(move || {
            while let Ok(Request::Show) = requests.recv() {
                if exit_queued(&requests) {
                    return;
                }
                if let Err(error) = run(&shared) {
                    warn!(%error, "the status window could not open; Observer keeps running in this console");
                }
                shared.lock().window = None;
            }
        })
}

/// Shows asked for while the window was opening count as one; an exit wins over them.
fn exit_queued(requests: &Receiver<Request>) -> bool {
    loop {
        match requests.try_recv() {
            Ok(Request::Show) => {}
            Ok(Request::Exit) | Err(TryRecvError::Disconnected) => return true,
            Err(TryRecvError::Empty) => return false,
        }
    }
}

fn run(shared: &Arc<Shared>) -> Result<(), eframe::Error> {
    let icon = egui::IconData {
        rgba: ICON.to_vec(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(text::TITLE)
            .with_inner_size([WIDTH, 400.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(Arc::new(icon)),
        centered: true,
        run_and_return: true,
        renderer: eframe::Renderer::Wgpu,
        // Every run on this thread reuses the event loop the first one made.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        })),
        ..eframe::NativeOptions::default()
    };
    let shared = Arc::clone(shared);
    eframe::run_native(
        text::TITLE,
        options,
        Box::new(move |creation| {
            let context = &creation.egui_ctx;
            context.set_theme(egui::ThemePreference::System);
            for theme in [Theme::Dark, Theme::Light] {
                context.set_visuals_of(theme, Palette::of(theme).visuals(theme));
            }
            shared.lock().window = Some(context.clone());
            Ok(Box::new(StatusWindow {
                shared,
                confirm: None,
                focus_cancel: false,
                height: 0.0,
                copied: false,
            }))
        }),
    )
}

struct StatusWindow {
    shared: Arc<Shared>,
    confirm: Option<Confirm>,
    /// The modal just opened: its Cancel takes the focus.
    focus_cancel: bool,
    height: f32,
    copied: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ink,
    Danger,
    Plain,
}

/// What one frame shows, read once.
struct Shown {
    status: ServerStatus,
    stopping: bool,
    in_flight: Option<Action>,
    now: Timestamp,
    palette: Palette,
}

impl eframe::App for StatusWindow {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let (exiting, asked, in_flight) = {
            let mut state = self.shared.lock();
            (state.exiting, state.confirm.take(), state.in_flight)
        };
        if exiting {
            context.send_viewport_cmd(ViewportCommand::Close);
        }
        if asked.is_some() {
            self.ask(asked);
        }
        let shown = Shown {
            status: self.shared.status.borrow().clone(),
            stopping: self.shared.stopping(),
            in_flight,
            now: Timestamp::now(),
            palette: Palette::of(context.theme()),
        };

        let drawn = Frame::new()
            .fill(shown.palette.bg)
            .inner_margin(Margin::same(PADDING))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 2.0 * f32::from(PADDING));
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                self.header(ui, &shown);
                state_line(ui, &shown);
                self.facts(ui, &shown);
                self.panel(ui, &shown);
                let setup_shown = self.setup(ui, &shown);
                self.controls(ui, &shown, setup_shown);
                self.footer(ui, &shown);
            });
        let height = drawn.response.rect.height();
        if (height - self.height).abs() > 0.5 {
            self.height = height;
            context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, height)));
        }
        self.modal(&context, &shown);
        // The uptime and the time left to the next restart count on.
        context.request_repaint_after(Duration::from_secs(1));
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

impl StatusWindow {
    fn ask(&mut self, confirm: Option<Confirm>) {
        self.confirm = confirm;
        self.focus_cancel = true;
    }

    fn header(&self, ui: &mut Ui, shown: &Shown) {
        let build = self.shared.build;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            paint_mark(ui.painter(), rect, &shown.palette);
            ui.label(
                RichText::new(text::TITLE)
                    .size(17.0)
                    .strong()
                    .color(shown.palette.ink),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let version = ui.label(
                    RichText::new(build.version)
                        .monospace()
                        .color(shown.palette.ink_3),
                );
                let commit = match (build.commit, build.commit_date) {
                    (Some(commit), Some(date)) => format!("{commit}, {date}"),
                    (Some(commit), None) => commit.to_owned(),
                    _ => text::DEVELOPMENT_BUILD.to_owned(),
                };
                version.on_hover_text(commit);
            });
        });
    }

    fn facts(&self, ui: &mut Ui, shown: &Shown) {
        let schedule = self.shared.schedule.borrow().clone();
        let zone = TimeZone::get(&schedule.zone).unwrap_or_else(|_| TimeZone::system());
        let status = &shown.status;
        let facts = [
            (text::PLAYERS, text::players(status.players)),
            (text::BUILD, text::build(status.build)),
            (
                text::NEXT_RESTART,
                text::next_restart(schedule.next.as_ref(), &zone, shown.now),
            ),
        ];
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (index, (name, value)) in facts.into_iter().enumerate() {
                if index > 0 {
                    ui.add_space(10.0);
                }
                ui.label(RichText::new(name).color(shown.palette.ink_3));
                ui.label(RichText::new(value).color(shown.palette.ink));
            }
        });
    }

    fn panel(&self, ui: &mut Ui, shown: &Shown) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(text::PANEL).color(shown.palette.ink_3));
            ui.add(
                egui::Label::new(
                    RichText::new(&self.shared.panel)
                        .monospace()
                        .color(shown.palette.ink),
                )
                .selectable(true),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let off = shown.stopping.then_some(text::OBSERVER_STOPPING);
                if button(ui, &shown.palette, text::OPEN_PANEL, Kind::Plain, off).clicked() {
                    self.shared.open_panel();
                }
            });
        });
    }

    /// The owner's link, while no owner exists. Says whether it showed.
    fn setup(&mut self, ui: &mut Ui, shown: &Shown) -> bool {
        let Some(token) = self.shared.setup.borrow().clone() else {
            return false;
        };
        let palette = &shown.palette;
        let url = self.shared.setup_url(&token);
        let expired = shown.now >= token.expires_at;
        Frame::new()
            .fill(palette.surface)
            .stroke(Stroke::new(1.0, palette.line))
            .corner_radius(CornerRadius::same(CARD_RADIUS))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(text::SETUP_TITLE).strong().color(palette.ink));
                ui.add(
                    egui::Label::new(RichText::new(&url).monospace().color(palette.ink_2))
                        .selectable(true)
                        .wrap(),
                );
                if expired {
                    ui.label(RichText::new(text::LINK_EXPIRED).color(palette.ink_2));
                    if button(ui, palette, text::NEW_LINK, Kind::Ink, None).clicked() {
                        self.copied = false;
                        self.shared.renew_link();
                    }
                    return;
                }
                ui.label(
                    RichText::new(text::link_expiry(token.expires_at, &TimeZone::system()))
                        .color(palette.ink_2),
                );
                ui.horizontal(|ui| {
                    if button(ui, palette, text::OPEN_SETUP, Kind::Ink, None).clicked() {
                        super::open(&url);
                    }
                    let copy = if self.copied {
                        text::LINK_COPIED
                    } else {
                        text::COPY_LINK
                    };
                    if button(ui, palette, copy, Kind::Plain, None).clicked() {
                        ui.ctx().copy_text(url.clone());
                        self.copied = true;
                    }
                });
            });
        !expired
    }

    fn controls(&mut self, ui: &mut Ui, shown: &Shown, setup_shown: bool) {
        let status = &shown.status;
        let palette = &shown.palette;
        let rules = controls(status);
        let busy = shown
            .stopping
            .then_some(text::OBSERVER_STOPPING)
            .or(shown.in_flight.map(|_| ""));
        let label = |action: Action, word: &'static str| match shown.in_flight {
            Some(flying) if flying == action => match action {
                Action::Start => text::START_SENT,
                Action::Stop => text::STOP_SENT,
                Action::Restart => text::RESTART_SENT,
            },
            _ => word,
        };
        // The setup page takes the ink while it shows: it is what to do first.
        let ink = if setup_shown { Kind::Plain } else { Kind::Ink };
        ui.horizontal(|ui| {
            // Restart stays in place, off, while the restart it made stops FXServer.
            if is_running(status.state) || restarting(status) {
                let reason = busy.or(rules.restart);
                if button(ui, palette, label(Action::Restart, text::RESTART), ink, reason).clicked() {
                    self.ask(Some(Confirm::Restart));
                }
            } else {
                let reason = busy.or(rules.start);
                if button(ui, palette, label(Action::Start, text::START), ink, reason).clicked() {
                    self.shared.act(Action::Start, false);
                }
            }
            let reason = busy.or(rules.stop);
            if button(ui, palette, label(Action::Stop, text::STOP), Kind::Plain, reason).clicked() {
                self.ask(Some(stop_confirm(status.state)));
            }
        });
        if let Some(outcome) = self.shared.outcome(status) {
            ui.label(RichText::new(outcome).color(palette.danger));
        }
    }

    fn footer(&mut self, ui: &mut Ui, shown: &Shown) {
        let palette = &shown.palette;
        ui.add(egui::Separator::default().spacing(4.0));
        let off = shown.stopping.then_some(text::OBSERVER_STOPPING);
        ui.horizontal(|ui| {
            if button(ui, palette, text::OPEN_DATA_FOLDER, Kind::Plain, off).clicked() {
                self.shared.open_data_folder();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, palette, text::QUIT, Kind::Plain, off).clicked() {
                    self.ask(Some(Confirm::Quit));
                }
            });
        });
    }

    fn modal(&mut self, context: &egui::Context, shown: &Shown) {
        let Some(confirm) = self.confirm else {
            return;
        };
        let palette = &shown.palette;
        let status = &shown.status;
        let online = status.players.map_or(0, |players| players.online);
        let running = is_running(status.state);
        let (title, body, yes, kind) = match confirm {
            Confirm::Restart => (
                text::RESTART_TITLE,
                format!("{} {}", text::connected(online), text::NOTICE_FIRST),
                text::CONFIRM_RESTART,
                Kind::Ink,
            ),
            Confirm::Stop => (
                text::STOP_TITLE,
                format!("{} {}", text::connected(online), text::NOTICE_FIRST),
                text::CONFIRM_STOP,
                Kind::Danger,
            ),
            Confirm::KeepStopped => (
                text::KEEP_STOPPED_TITLE,
                text::KEEP_STOPPED.to_owned(),
                text::CONFIRM_KEEP_STOPPED,
                Kind::Plain,
            ),
            Confirm::Quit if running => (
                text::QUIT_TITLE,
                text::quit_body(Some(online)),
                text::CONFIRM_QUIT_RUNNING,
                Kind::Danger,
            ),
            Confirm::Quit => (
                text::QUIT_TITLE,
                text::quit_body(None),
                text::CONFIRM_QUIT,
                Kind::Ink,
            ),
        };
        let mut answer = None;
        let shadow = egui::Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(90),
        };
        let modal = egui::Modal::new(Id::unique("confirm"))
            .backdrop_color(palette.overlay)
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .stroke(Stroke::new(1.0, palette.line))
                    .corner_radius(CornerRadius::same(CARD_RADIUS))
                    .inner_margin(Margin::same(18))
                    .shadow(shadow),
            )
            .show(context, |ui| {
                ui.set_width(WIDTH - 80.0);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                ui.label(RichText::new(title).size(16.0).strong().color(palette.ink));
                ui.label(RichText::new(body).color(palette.ink_2));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, palette, yes, kind, None).clicked() {
                        answer = Some(true);
                    }
                    let cancel = button(ui, palette, text::CANCEL, Kind::Plain, None);
                    if self.focus_cancel {
                        cancel.request_focus();
                        self.focus_cancel = false;
                    }
                    if cancel.clicked() {
                        answer = Some(false);
                    }
                });
            });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        let Some(confirmed) = answer else {
            return;
        };
        self.confirm = None;
        if !confirmed {
            return;
        }
        match confirm {
            Confirm::Restart => self.shared.act(Action::Restart, false),
            Confirm::Stop | Confirm::KeepStopped => self.shared.act(Action::Stop, false),
            Confirm::Quit => self.shared.stop.request(),
        }
    }
}

fn state_line(ui: &mut Ui, shown: &Shown) {
    let status = &shown.status;
    let palette = &shown.palette;
    ui.horizontal(|ui| {
        let led = if shown.stopping {
            Led::Pending
        } else {
            palette::led(status.state)
        };
        paint_led(ui, palette, led);
        let word = if shown.stopping {
            text::STOPPING_OBSERVER
        } else if restarting(status) {
            text::RESTARTING
        } else {
            text::state(status.state)
        };
        ui.label(RichText::new(word).size(15.0).color(palette.ink));
        if let Some(started_at) = status.started_at.filter(|_| !shown.stopping) {
            ui.label(
                RichText::new(format!("· {}", text::uptime(started_at, shown.now)))
                    .size(15.0)
                    .color(palette.ink_2),
            );
        }
    });
    if let Some(note) = text::launch_note(status.launch).filter(|_| !shown.stopping) {
        ui.label(RichText::new(note).color(palette.ink_2));
    }
}

/// A stop of a server that is not running only keeps it from coming back.
pub(super) fn stop_confirm(state: ServerState) -> Confirm {
    if matches!(state, ServerState::Crashed | ServerState::CrashLoop) {
        Confirm::KeepStopped
    } else {
        Confirm::Stop
    }
}

/// A control in the panel's style. One that cannot act is off, its reason on hover; an empty
/// reason only turns it off.
fn button(ui: &mut Ui, palette: &Palette, label: &str, kind: Kind, off: Option<&str>) -> Response {
    let (fill, ink) = match kind {
        Kind::Ink => (palette.btn_bg, palette.btn_ink),
        Kind::Danger => (palette.danger, palette.on_danger),
        Kind::Plain => (palette.surface_2, palette.ink),
    };
    let widget = egui::Button::new(RichText::new(label).color(ink))
        .fill(fill)
        .corner_radius(CornerRadius::same(CONTROL_RADIUS))
        .min_size(Vec2::new(0.0, 32.0));
    let mut response = ui.add_enabled(off.is_none(), widget);
    if let Some(reason) = off.filter(|reason| !reason.is_empty()) {
        response = response.on_disabled_hover_text(reason);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            CornerRadius::same(CONTROL_RADIUS + 2),
            Stroke::new(2.0, palette.accent),
            StrokeKind::Outside,
        );
    }
    response
}

/// The design's mark: an ink tile, a ring the colour of the page, an accent dot. It follows the
/// theme, as the panel's logo does.
fn paint_mark(painter: &Painter, rect: Rect, palette: &Palette) {
    let centre = rect.center();
    painter.rect_filled(rect, CornerRadius::same(6), palette.ink);
    painter.circle_stroke(centre, 6.25, Stroke::new(2.0, palette.bg));
    painter.circle_filled(centre, 2.25, palette.accent);
}

fn paint_led(ui: &mut Ui, palette: &Palette, led: Led) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let centre = rect.center();
    match (led, palette.led(led)) {
        (_, Some(colour)) => {
            ui.painter().circle_filled(centre, 4.0, colour);
        }
        (Led::Pending, None) => {
            ui.painter()
                .circle_stroke(centre, 3.25, Stroke::new(1.5, palette.accent));
        }
        _ => {
            ui.painter().circle_filled(centre, 4.0, palette.line_strong);
        }
    }
}
