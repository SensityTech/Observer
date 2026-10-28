//! The panel's colour tokens (`web/src/styles/tokens.css`) as egui colours, in both themes.

use eframe::egui::{Color32, CornerRadius, Stroke, Theme, Visuals};
use observer_domain::server::ServerState;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub ink: Color32,
    pub ink_2: Color32,
    pub ink_3: Color32,
    pub accent: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub on_danger: Color32,
    pub btn_bg: Color32,
    pub btn_ink: Color32,
    pub overlay: Color32,
}

/// What the state's LED shows, as the panel's `STATE_LED` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Led {
    Live,
    Warning,
    Pending,
    Down,
    Off,
}

pub const CARD_RADIUS: u8 = 14;
pub const CONTROL_RADIUS: u8 = 10;

// The tokens' `rgba()` lines are colours at an alpha out of 255 over what lies beneath.
pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0B, 0x0B, 0x0B),
    surface: Color32::from_rgb(0x14, 0x14, 0x14),
    surface_2: Color32::from_rgb(0x1A, 0x1A, 0x1A),
    line: Color32::from_rgba_unmultiplied_const(0xFF, 0xFF, 0xFF, 20),
    line_strong: Color32::from_rgba_unmultiplied_const(0xFF, 0xFF, 0xFF, 36),
    ink: Color32::from_rgb(0xF5, 0xF5, 0xF5),
    ink_2: Color32::from_rgb(0xC4, 0xC4, 0xC4),
    ink_3: Color32::from_rgb(0x8A, 0x8A, 0x8A),
    accent: Color32::from_rgb(0x4D, 0x8D, 0xFF),
    warning: Color32::from_rgb(0xFB, 0xBF, 0x24),
    danger: Color32::from_rgb(0xF8, 0x71, 0x71),
    on_danger: Color32::from_rgb(0x0B, 0x0B, 0x0B),
    btn_bg: Color32::from_rgb(0xF5, 0xF5, 0xF5),
    btn_ink: Color32::from_rgb(0x0B, 0x0B, 0x0B),
    overlay: Color32::from_rgba_unmultiplied_const(0x00, 0x00, 0x00, 158),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface_2: Color32::from_rgb(0xF6, 0xF6, 0xF7),
    line: Color32::from_rgba_unmultiplied_const(0x0A, 0x0A, 0x0C, 20),
    line_strong: Color32::from_rgba_unmultiplied_const(0x0A, 0x0A, 0x0C, 36),
    ink: Color32::from_rgb(0x1C, 0x1D, 0x1F),
    ink_2: Color32::from_rgb(0x4B, 0x50, 0x58),
    ink_3: Color32::from_rgb(0x6B, 0x70, 0x79),
    accent: Color32::from_rgb(0x1F, 0x5B, 0xEA),
    warning: Color32::from_rgb(0xD9, 0x77, 0x06),
    danger: Color32::from_rgb(0xDC, 0x26, 0x26),
    on_danger: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    btn_bg: Color32::from_rgb(0x1C, 0x1D, 0x1F),
    btn_ink: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    overlay: Color32::from_rgba_unmultiplied_const(0x10, 0x11, 0x13, 82),
};

impl Palette {
    pub const fn of(theme: Theme) -> Self {
        match theme {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
        }
    }

    pub const fn led(&self, led: Led) -> Option<Color32> {
        match led {
            Led::Live => Some(self.accent),
            Led::Warning => Some(self.warning),
            Led::Down => Some(self.danger),
            Led::Pending | Led::Off => None,
        }
    }

    /// egui's own widgets drawn with the tokens: the canvas, the controls and the focus ring.
    pub fn visuals(&self, theme: Theme) -> Visuals {
        let mut visuals = match theme {
            Theme::Dark => Visuals::dark(),
            Theme::Light => Visuals::light(),
        };
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.surface;
        visuals.window_stroke = Stroke::new(1.0, self.line);
        visuals.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        visuals.extreme_bg_color = self.surface_2;
        visuals.faint_bg_color = self.surface_2;
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.3);
        visuals.selection.stroke = Stroke::new(2.0, self.accent);
        let radius = CornerRadius::same(CONTROL_RADIUS);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = radius;
            widget.fg_stroke = Stroke::new(1.0, self.ink);
        }
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.line);
        visuals.widgets.inactive.weak_bg_fill = self.surface_2;
        visuals.widgets.inactive.bg_fill = self.surface_2;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.line_strong);
        visuals.widgets.hovered.weak_bg_fill = self.surface_2;
        visuals.widgets.hovered.bg_fill = self.surface_2;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.ink_3);
        visuals.widgets.active.weak_bg_fill = self.surface_2;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.ink_2);
        visuals
    }
}

pub fn led(state: ServerState) -> Led {
    match state {
        ServerState::Online => Led::Live,
        ServerState::Degraded | ServerState::Hung => Led::Warning,
        ServerState::Starting | ServerState::Stopping => Led::Pending,
        ServerState::Crashed | ServerState::CrashLoop => Led::Down,
        ServerState::Stopped => Led::Off,
    }
}
