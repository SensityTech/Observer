//! The tray icon: the state in its tooltip, the window on a click, the controls in its menu.

use std::io;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use observer_platform::{MessageLoop, Waker};
use tracing::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::window::stop_confirm;
use super::{Action, Confirm, Shared, controls, text};

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

        let rules = controls(&status);
        let busy = stopping || self.shared.lock().in_flight.is_some();
        let items = &self.items;
        items.start.set_enabled(!busy && rules.start.is_none());
        items.stop.set_enabled(!busy && rules.stop.is_none());
        items.restart.set_enabled(!busy && rules.restart.is_none());
        for item in [&items.open_panel, &items.data_folder, &items.quit] {
            item.set_enabled(!stopping);
        }
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
