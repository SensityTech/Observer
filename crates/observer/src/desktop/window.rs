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
use super::{Action, Confirm, Outcome, Request, Shared, controls, is_running, restarting, text};

const WIDTH: f32 = 440.0;
const PADDING: i8 = 20;
const BUTTON_HEIGHT: f32 = 32.0;
const ICON: &[u8] = include_bytes!("../../assets/mark-64.rgba");

pub(super) fn spawn(shared: Arc<Shared>, requests: Receiver<Request>) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("observer-window".to_owned())
        .spawn(move || {
            while let Ok(Request::Show) = requests.recv() {
                if exit_queued(&requests) {
                    return;
                }
                let result = run(&shared);
                let mut state = shared.lock();
                state.window = None;
                if let Err(error) = result {
                    warn!(%error, "the status window could not open; Observer keeps running in this console");
                    // A confirmation asked from the tray would otherwise wait for the next window
                    // that opens, however late.
                    state.window_failed = true;
                    state.confirm = None;
                }
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
            {
                let mut state = shared.lock();
                state.window = Some(context.clone());
                state.window_failed = false;
            }
            Ok(Box::new(StatusWindow {
                shared,
                confirm: None,
                focus_cancel: false,
                height: 0.0,
                copied: false,
                unavailable: None,
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
    /// Why the control last pressed while off could not act, until the server's state changes.
    unavailable: Option<Outcome>,
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
            ui.label(RichText::new(text::TITLE).size(17.0).color(shown.palette.ink));
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
                ui.ctx()
                    .accesskit_node_builder(version.id, |node| node.set_description(commit.as_str()));
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
        facts_row(ui, &shown.palette, &facts);
    }

    fn panel(&mut self, ui: &mut Ui, shown: &Shown) {
        let off = shown.stopping.then_some(text::OBSERVER_STOPPING);
        let response = panel_row(ui, &shown.palette, &self.shared.panel, off);
        if self.acts(&response, text::OPEN_PANEL, off, &shown.status) {
            self.shared.open_panel();
        }
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
                ui.label(RichText::new(text::SETUP_TITLE).size(15.0).color(palette.ink));
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
                let response = button(ui, palette, label(Action::Restart, text::RESTART), ink, reason);
                if self.acts(&response, text::RESTART, reason, status) {
                    self.ask(Some(Confirm::Restart));
                }
            } else {
                let reason = busy.or(rules.start);
                let response = button(ui, palette, label(Action::Start, text::START), ink, reason);
                if self.acts(&response, text::START, reason, status) {
                    self.shared.act(Action::Start, false);
                }
            }
            let reason = busy.or(rules.stop);
            let response = button(ui, palette, label(Action::Stop, text::STOP), Kind::Plain, reason);
            if self.acts(&response, text::STOP, reason, status) {
                self.ask(Some(stop_confirm(status.state)));
            }
        });
        if let Some(outcome) = self.shared.outcome(status) {
            ui.label(RichText::new(outcome).color(palette.danger));
        }
        self.unavailable = self
            .unavailable
            .take()
            .filter(|unavailable| unavailable.since == status.since);
        if let Some(unavailable) = &self.unavailable {
            ui.label(RichText::new(&unavailable.text).color(palette.ink_2));
        }
    }

    /// Whether a press of a control acts. One that is off says why instead, under the controls,
    /// until the server's state changes: the panel's model.
    fn acts(&mut self, response: &Response, control: &str, off: Option<&str>, status: &ServerStatus) -> bool {
        if !response.clicked() {
            return false;
        }
        match off {
            None => {
                self.unavailable = None;
                true
            }
            Some(reason) => {
                if !reason.is_empty() {
                    self.unavailable = Some(Outcome {
                        text: text::unavailable(control, reason),
                        since: status.since,
                    });
                }
                false
            }
        }
    }

    fn footer(&mut self, ui: &mut Ui, shown: &Shown) {
        let palette = &shown.palette;
        ui.add(egui::Separator::default().spacing(4.0));
        let off = shown.stopping.then_some(text::OBSERVER_STOPPING);
        ui.horizontal(|ui| {
            let response = button(ui, palette, text::OPEN_DATA_FOLDER, Kind::Plain, off);
            if self.acts(&response, text::OPEN_DATA_FOLDER, off, &shown.status) {
                self.shared.open_data_folder();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = button(ui, palette, text::QUIT, Kind::Plain, off);
                if self.acts(&response, text::QUIT, off, &shown.status) {
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
        let question = match confirm {
            Confirm::Restart => Question {
                title: text::RESTART_TITLE,
                body: format!("{} {}", text::connected(online), text::NOTICE_FIRST),
                yes: text::CONFIRM_RESTART,
                kind: Kind::Ink,
            },
            Confirm::Stop => Question {
                title: text::STOP_TITLE,
                body: format!("{} {}", text::connected(online), text::NOTICE_FIRST),
                yes: text::CONFIRM_STOP,
                kind: Kind::Danger,
            },
            Confirm::KeepStopped => Question {
                title: text::KEEP_STOPPED_TITLE,
                body: text::KEEP_STOPPED.to_owned(),
                yes: text::CONFIRM_KEEP_STOPPED,
                kind: Kind::Plain,
            },
            Confirm::Quit if running => Question {
                title: text::QUIT_TITLE,
                body: text::quit_body(Some(online)),
                yes: text::CONFIRM_QUIT_RUNNING,
                kind: Kind::Danger,
            },
            Confirm::Quit => Question {
                title: text::QUIT_TITLE,
                body: text::quit_body(None),
                yes: text::CONFIRM_QUIT,
                kind: Kind::Ink,
            },
        };
        let Some(confirmed) = confirmation(context, palette, &question, &mut self.focus_cancel) else {
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

/// What a confirmation asks, and the button that says yes.
struct Question {
    title: &'static str,
    body: String,
    yes: &'static str,
    kind: Kind,
}

/// The confirmation's dialog, named by its title for screen readers. `Some(true)` once confirmed,
/// `Some(false)` once cancelled (Cancel, Esc, a click outside), `None` while it waits.
fn confirmation(
    context: &egui::Context,
    palette: &Palette,
    question: &Question,
    focus_cancel: &mut bool,
) -> Option<bool> {
    let mut answer = None;
    let shadow = egui::Shadow {
        offset: [0, 18],
        blur: 48,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    let id = Id::unique("confirm");
    let modal = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).accessible_name(question.title))
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
            ui.label(RichText::new(question.title).size(16.0).color(palette.ink));
            ui.label(RichText::new(&question.body).color(palette.ink_2));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, palette, question.yes, question.kind, None).clicked() {
                    answer = Some(true);
                }
                let cancel = button(ui, palette, text::CANCEL, Kind::Plain, None);
                if *focus_cancel {
                    cancel.request_focus();
                    *focus_cancel = false;
                }
                if cancel.clicked() {
                    answer = Some(false);
                }
            });
        });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// `Players 12 / 64   Build 37872   Next restart 04:00 (in 3h 12m)`, on as many lines as the width
/// needs. A fact moves to the next line whole: `30 Oct 01:00 (in 23h 6m)` never leaves part of
/// itself behind, nor its name.
fn facts_row(ui: &mut Ui, palette: &Palette, facts: &[(&str, String)]) {
    const GAP: f32 = 10.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let spacing = ui.spacing().item_spacing.x;
        for (index, (name, value)) in facts.iter().enumerate() {
            let name = whole(ui, name, palette.ink_3);
            let value = whole(ui, value, palette.ink);
            let width = name.size().x + spacing + value.size().x;
            if index > 0 {
                if ui.available_size_before_wrap().x < GAP + 2.0 * spacing + width {
                    ui.end_row();
                } else {
                    ui.add_space(GAP);
                }
            }
            ui.add(egui::Label::new(name));
            ui.add(egui::Label::new(value));
        }
    });
}

/// Text laid out on one line, whatever the width left.
fn whole(ui: &Ui, text: &str, colour: Color32) -> Arc<egui::Galley> {
    egui::WidgetText::from(RichText::new(text).color(colour)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    )
}

/// `Panel  http://localhost:40125  [Open the panel]`.
fn panel_row(ui: &mut Ui, palette: &Palette, address: &str, off: Option<&str>) -> Response {
    ui.horizontal(|ui| {
        // Each label centres on the row's height as it stands when the label is placed: the row
        // takes the button's height first.
        ui.set_min_height(BUTTON_HEIGHT);
        ui.label(RichText::new(text::PANEL).color(palette.ink_3));
        ui.add(egui::Label::new(RichText::new(address).monospace().color(palette.ink)).selectable(true));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            button(ui, palette, text::OPEN_PANEL, Kind::Plain, off)
        })
        .inner
    })
    .inner
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

/// A control in the panel's style. One that is off for a reason stays focusable, as the panel's
/// `aria-disabled` buttons do: it looks off, gives its reason on hover and to screen readers, and
/// its press only says why (`StatusWindow::acts`). An empty reason, a request on its way, turns it
/// off entirely.
fn button(ui: &mut Ui, palette: &Palette, label: &str, kind: Kind, off: Option<&str>) -> Response {
    let (fill, ink) = match kind {
        Kind::Ink => (palette.btn_bg, palette.btn_ink),
        Kind::Danger => (palette.danger, palette.on_danger),
        Kind::Plain => (palette.surface_2, palette.ink),
    };
    let reason = off.filter(|reason| !reason.is_empty());
    // The panel's `opacity-45`.
    let (fill, ink) = match reason {
        Some(_) => (fill.gamma_multiply(0.45), ink.gamma_multiply(0.45)),
        None => (fill, ink),
    };
    let widget = egui::Button::new(RichText::new(label).color(ink))
        .fill(fill)
        .corner_radius(CornerRadius::same(CONTROL_RADIUS))
        .min_size(Vec2::new(0.0, BUTTON_HEIGHT));
    let mut response = ui.add_enabled(off.is_none() || reason.is_some(), widget);
    if let Some(reason) = reason {
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_disabled();
            node.set_description(reason);
        });
        response = response
            .on_hover_text(reason)
            .on_hover_cursor(egui::CursorIcon::NotAllowed);
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

#[cfg(test)]
mod tests {
    use eframe::egui::accesskit::{Node, Role};
    use eframe::egui::{Event, Key, Modifiers, Pos2, RawInput};

    use super::*;
    use crate::desktop::palette::DARK;

    /// `draw` inside the window's padding, and the accessibility tree it built. Two passes: an
    /// area such as a dialog only shows from its second.
    fn drawn(draw: impl FnMut(&mut Ui)) -> Vec<Node> {
        pressing_tab(draw, 0).0
    }

    /// `draw` over a pass, a pass for each press of Tab, and a last pass: that pass's tree, and
    /// the label of what holds the keyboard's focus.
    fn pressing_tab(mut draw: impl FnMut(&mut Ui), presses: usize) -> (Vec<Node>, Option<String>) {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut drawn = (Vec::new(), None);
        for pass in 0..presses + 2 {
            let tab = Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH, 600.0))),
                events: (1..=presses).contains(&pass).then_some(tab).into_iter().collect(),
                ..RawInput::default()
            };
            let mut output = context.run_ui(input, |ui| {
                Frame::new().inner_margin(Margin::same(PADDING)).show(ui, |ui| {
                    ui.set_width(WIDTH - 2.0 * f32::from(PADDING));
                    draw(ui);
                });
            });
            // Nothing renders: the font atlas the pass made goes nowhere.
            output.textures_delta.clear();
            if let Some(update) = output.platform_output.accesskit_update {
                let focus = update
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == update.focus)
                    .and_then(|(_, node)| node.label().map(str::to_owned));
                drawn = (update.nodes.into_iter().map(|(_, node)| node).collect(), focus);
            }
        }
        drawn
    }

    /// The node a screen reader reads as `text`.
    fn read<'a>(nodes: &'a [Node], text: &str) -> &'a Node {
        nodes
            .iter()
            .find(|node| node.label() == Some(text) || node.value() == Some(text))
            .unwrap_or_else(|| panic!("nothing reads {text:?}"))
    }

    fn middle(node: &Node) -> f64 {
        let bounds = node.bounds().expect("a drawn node has bounds");
        f64::midpoint(bounds.y0, bounds.y1)
    }

    #[test]
    fn a_fact_moves_to_the_next_line_whole() {
        for players in ["12 / 64", "2,048 / 2,048"] {
            let facts = [
                (text::PLAYERS, players.to_owned()),
                (text::BUILD, "35245".to_owned()),
                (text::NEXT_RESTART, "30 Oct 01:00 (in 23h 6m)".to_owned()),
            ];
            let nodes = drawn(|ui| facts_row(ui, &DARK, &facts));

            for (name, value) in &facts {
                let name = read(&nodes, name).bounds().unwrap();
                let value = read(&nodes, value).bounds().unwrap();
                assert!(
                    (value.y0 - name.y0).abs() < 0.5 && (value.height() - name.height()).abs() < 0.5,
                    "{value:?} is not on one line beside its name {name:?} ({players} players)"
                );
            }
        }
    }

    #[test]
    fn a_confirmation_is_a_dialog_its_title_names() {
        let question = Question {
            title: text::STOP_TITLE,
            body: format!("{} {}", text::connected(12), text::NOTICE_FIRST),
            yes: text::CONFIRM_STOP,
            kind: Kind::Danger,
        };
        let mut focus_cancel = true;

        let nodes = drawn(|ui| {
            confirmation(ui.ctx(), &DARK, &question, &mut focus_cancel);
        });

        let dialog = nodes
            .iter()
            .find(|node| node.role() == Role::Dialog)
            .expect("the confirmation is a dialog");
        assert_eq!(dialog.label(), Some(text::STOP_TITLE));
    }

    #[test]
    fn a_control_that_is_off_is_reached_by_tab_and_read_with_its_reason() {
        let controls = |ui: &mut Ui| {
            button(ui, &DARK, text::START, Kind::Ink, Some(text::NOT_CONFIGURED));
            // A request on its way: nothing to say, nothing to reach.
            button(ui, &DARK, text::STOP, Kind::Plain, Some(""));
            button(ui, &DARK, text::QUIT, Kind::Plain, None);
        };

        let (nodes, first) = pressing_tab(controls, 1);
        let (_, second) = pressing_tab(controls, 2);

        assert_eq!(first.as_deref(), Some(text::START));
        assert_eq!(second.as_deref(), Some(text::QUIT));
        let start = read(&nodes, text::START);
        assert!(start.is_disabled());
        assert_eq!(start.description(), Some(text::NOT_CONFIGURED));
    }

    #[test]
    fn the_panel_rows_words_centre_on_its_button() {
        let nodes = drawn(|ui| {
            panel_row(ui, &DARK, "http://localhost:40125", None);
        });

        let button = middle(read(&nodes, text::OPEN_PANEL));
        for words in [text::PANEL, "http://localhost:40125"] {
            let offset = middle(read(&nodes, words)) - button;
            assert!(
                offset.abs() < 1.0,
                "{words:?} sits {offset} px off the button's middle"
            );
        }
    }
}
