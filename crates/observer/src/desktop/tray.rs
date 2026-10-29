//! The tray icon: the state in its tooltip, the window on a click, the controls in its menu.

use std::io;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use observer_platform::{MessageLoop, Waker};
use tracing::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::window::stop_confirm;
use super::{Action, Confirm, Controls, Shared, controls, text};

const TICK: Duration = Duration::from_secs(1);
/// The executable's own icon, from which Windows picks the size the screen's scaling needs.
const ICON_RESOURCE: u16 = 1;

pub(super) fn spawn(shared: Arc<Shared>) -> io::Result<MessageLoop> {
    MessageLoop::spawn(
        "observer-tray",
        TICK,
        move |waker| Tray::new(shared, waker),
        Tray::refresh,
    )
}

enum Event {
    Click,
    Menu(MenuId),
}

struct Items {
    open_panel: MenuItem,
    show: MenuItem,
    start: MenuItem,
    stop: MenuItem,
    restart: MenuItem,
    data_folder: MenuItem,
    quit: MenuItem,
}

struct Tray {
    shared: Arc<Shared>,
    /// Dropped on this thread as the loop ends, which takes the icon out of the notification area.
    icon: Option<TrayIcon>,
    items: Items,
    events: mpsc::Receiver<Event>,
    tooltip: String,
}

impl Tray {
    fn new(shared: Arc<Shared>, waker: Waker) -> Self {
        let item = |label| MenuItem::new(label, true, None);
        let items = Items {
            open_panel: item(text::OPEN_PANEL),
            show: item(text::SHOW_WINDOW),
            start: item(text::START),
            stop: item(text::STOP_ASKING),
            restart: item(text::RESTART_ASKING),
            data_folder: item(text::OPEN_DATA_FOLDER),
            quit: item(text::QUIT),
        };
        let menu = Menu::new();
        let built = menu.append_items(&[
            &items.open_panel,
            &items.show,
            &PredefinedMenuItem::separator(),
            &items.start,
            &items.stop,
            &items.restart,
            &PredefinedMenuItem::separator(),
            &items.data_folder,
            &PredefinedMenuItem::separator(),
            &items.quit,
        ]);
        if let Err(error) = built {
            warn!(%error, "cannot build the tray icon's menu");
        }

        // The handlers run wherever tray-icon calls them: they only pass the event on.
        let (events, received) = mpsc::channel();
        let clicks = events.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = clicks.send(Event::Click);
                waker.wake();
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = events.send(Event::Menu(event.id));
            waker.wake();
        }));

        let icon = Icon::from_resource(ICON_RESOURCE, None)
            .map_err(|error| error.to_string())
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .with_tooltip(text::TITLE)
                    .build()
                    .map_err(|error| error.to_string())
            })
            .inspect_err(|error| warn!(%error, "the tray icon could not be shown"))
            .ok();
        let mut tray = Self {
            shared,
            icon,
            items,
            events: received,
            tooltip: String::new(),
        };
        tray.refresh();
        tray
    }

    /// On every tick and every wake: what was clicked, then the tooltip and the menu as they
    /// should read now.
    fn refresh(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(&event);
        }
        let status = self.shared.status.borrow().clone();
        let stopping = self.shared.stopping();

        let tooltip = text::tooltip(
            (!stopping).then_some(status.state),
            status.players,
            self.shared.listen.port(),
        );
        if tooltip != self.tooltip {
            if let Some(icon) = &self.icon
                && let Err(error) = icon.set_tooltip(Some(&tooltip))
            {
                warn!(%error, "cannot update the tray icon's tooltip");
            }
            self.tooltip = tooltip;
        }

        let (in_flight, window_failed) = {
            let state = self.shared.lock();
            (state.in_flight.is_some(), state.window_failed)
        };
        let enabled = enabled(&controls(&status), stopping, in_flight, window_failed);
        let items = &self.items;
        items.start.set_enabled(enabled.start);
        items.stop.set_enabled(enabled.stop);
        items.restart.set_enabled(enabled.restart);
        items.open_panel.set_enabled(enabled.open);
        items.data_folder.set_enabled(enabled.open);
        items.quit.set_enabled(enabled.quit);
    }

    fn handle(&self, event: &Event) {
        let shared = &self.shared;
        let id = match event {
            Event::Click => {
                shared.show(None);
                return;
            }
            Event::Menu(id) => id,
        };
        let items = &self.items;
        let state = shared.status.borrow().state;
        if id == items.open_panel.id() {
            shared.open_panel();
        } else if id == items.show.id() {
            shared.show(None);
        } else if id == items.start.id() {
            shared.act(Action::Start, true);
        } else if id == items.stop.id() {
            shared.show(Some(stop_confirm(state)));
        } else if id == items.restart.id() {
            shared.show(Some(Confirm::Restart));
        } else if id == items.data_folder.id() {
            shared.open_data_folder();
        } else if id == items.quit.id() {
            shared.show(Some(Confirm::Quit));
        }
    }
}

/// Which items of the menu act now.
#[derive(Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "one switch per item of the menu")]
struct Enabled {
    start: bool,
    stop: bool,
    restart: bool,
    /// Open the panel, Open the data folder.
    open: bool,
    quit: bool,
}

/// Stop…, Restart… and Quit… confirm in the window: while it cannot open they stay off, rather
/// than do nothing. Show the window tries it again; Ctrl+C in the console still stops Observer.
fn enabled(rules: &Controls, stopping: bool, in_flight: bool, window_failed: bool) -> Enabled {
    let busy = stopping || in_flight;
    let asks = !window_failed;
    Enabled {
        start: !busy && rules.start.is_none(),
        stop: asks && !busy && rules.stop.is_none(),
        restart: asks && !busy && rules.restart.is_none(),
        open: !stopping,
        quit: asks && !stopping,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: Controls = Controls {
        start: Some(text::RUNNING),
        stop: None,
        restart: None,
    };

    #[test]
    fn what_asks_in_the_window_is_off_while_the_window_cannot_open() {
        assert_eq!(
            enabled(&RUNNING, false, false, false),
            Enabled {
                start: false,
                stop: true,
                restart: true,
                open: true,
                quit: true
            }
        );
        assert_eq!(
            enabled(&RUNNING, false, false, true),
            Enabled {
                start: false,
                stop: false,
                restart: false,
                open: true,
                quit: false
            }
        );
        let stopped = Controls {
            start: None,
            stop: Some(text::NOT_RUNNING),
            restart: Some(text::NOT_RUNNING),
        };
        assert!(enabled(&stopped, false, false, true).start);
        assert_eq!(
            enabled(&stopped, true, false, false),
            Enabled {
                start: false,
                stop: false,
                restart: false,
                open: false,
                quit: false
            }
        );
    }
}
