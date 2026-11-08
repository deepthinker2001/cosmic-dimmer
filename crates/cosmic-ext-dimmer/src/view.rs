//! The popup's widgets: each display's sliders and lock, the auto-dimming
//! toggle, and the settings section. Pure functions of the applet's state;
//! what a display's row holds is decided by [`parts`], which the tests check
//! without a display server.

use cosmic::iced::{Alignment, Length, Rectangle};
use cosmic::widget::{column, divider, icon, row, slider, spin_button, text, toggler};
use cosmic::{Element, applet, iced};
use dimmer_core::{BackendError, Floors};

use crate::app::{Dimmer, Message};
use crate::fl;
use crate::monitor::{Channel, Monitor, Unavailable};

impl Dimmer {
    pub(crate) fn popup_content(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();
        let mut content = column::with_capacity(self.monitors.len() * 2 + 3)
            .padding([spacing.space_xxs, 0])
            .spacing(spacing.space_xxxs)
            .width(Length::Fill);
        if self.monitors.is_empty() {
            let status = if self.discovered {
                fl!("no-displays")
            } else {
                fl!("looking")
            };
            content = content.push(applet::padded_control(text::body(status)));
        }
        let mut unnamed = 0;
        for (index, monitor) in self.monitors.iter().enumerate() {
            if index > 0 {
                content = content.push(applet::padded_control(divider::horizontal::default()));
            }
            let name = if monitor.found.name.is_empty() {
                // Numbered, so two displays nothing could name stay apart.
                unnamed += 1;
                fl!("display-unnamed", number = unnamed)
            } else {
                monitor.found.name.clone()
            };
            content = content.push(monitor_view(monitor, name));
        }
        content = content.push(applet::padded_control(divider::horizontal::default()));
        content = content.push(setting_row(
            fl!("auto-dimming"),
            toggler(self.settings.auto_dimming)
                .on_toggle(Message::SetAuto)
                .into(),
        ));
        content = content.push(
            applet::menu_button(
                row::with_capacity(2)
                    .push(text::body(fl!("settings")).width(Length::Fill))
                    .push(icon::from_name(if self.show_settings {
                        "go-down-symbolic"
                    } else {
                        "go-next-symbolic"
                    }))
                    .align_y(Alignment::Center),
            )
            .on_press(Message::ToggleSettings),
        );
        if self.show_settings {
            content = content.push(self.settings_view());
        }
        content.into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let mut section = column::with_capacity(10).spacing(cosmic::theme::spacing().space_xxxs);
        if self.settings.auto_dimming {
            section = section.push(self.location_view());
        }
        for row in self.floor_rows().into_iter().chain(self.shortcut_rows()) {
            section = section.push(row);
        }
        section.into()
    }

    /// The floors. The hardware and combined floors only matter, and so only
    /// show, when a display has hardware control.
    fn floor_rows(&self) -> Vec<Element<'_, Message>> {
        let floors = self.settings.floors();
        let hard = Floors::HARD_MINIMUM;
        let floor_row = |label: String, value: u8, minimum: u8, make: fn(Floors, u8) -> Floors| {
            setting_row(
                label,
                spin_button(percent(value), value, 1, minimum, 100, move |v| {
                    Message::Floors(make(floors, v))
                })
                .into(),
            )
        };
        let hardware = shows_hardware_floors(&self.monitors);
        let mut rows = Vec::with_capacity(3);
        if hardware {
            rows.push(floor_row(
                fl!("floor-hardware"),
                floors.hardware(),
                hard.hardware(),
                |f, v| Floors::clamped(v, f.alpha(), f.effective()),
            ));
        }
        rows.push(floor_row(
            fl!("floor-software"),
            floors.alpha(),
            hard.alpha(),
            |f, v| Floors::clamped(f.hardware(), v, f.effective()),
        ));
        if hardware {
            rows.push(floor_row(
                fl!("floor-effective"),
                floors.effective(),
                hard.effective(),
                |f, v| Floors::clamped(f.hardware(), f.alpha(), v),
            ));
        }
        rows
    }
}

/// A level as the popup shows it, such as "40%".
pub(crate) fn percent(value: u8) -> String {
    fl!("level", value = value)
}

/// A settings row: a label on the left, its control on the right.
pub(crate) fn setting_row(label: String, control: Element<'_, Message>) -> Element<'_, Message> {
    applet::padded_control(
        row::with_capacity(2)
            .push(text::body(label).width(Length::Fill))
            .push(control)
            .spacing(cosmic::theme::spacing().space_xxs)
            .align_y(Alignment::Center),
    )
    .into()
}

/// What one display's row holds (R11). Kept apart from the widgets so the
/// layout rules are tested without a display server.
#[derive(Debug, Eq, PartialEq)]
struct Parts {
    /// The sliders shown, in order. Hardware appears only when the hardware
    /// backend can write the display: otherwise it is absent, not disabled.
    sliders: Vec<Channel>,
    /// The lock appears only when both sliders do.
    lock: bool,
    /// Small muted notes under the name. A missing hardware slider gets none:
    /// the slider being absent says enough.
    notes: Vec<String>,
}

fn parts(monitor: &Monitor) -> Parts {
    let mut sliders = Vec::with_capacity(2);
    let mut notes = Vec::new();
    for channel in [Channel::Hardware, Channel::Alpha] {
        match monitor.endpoint(channel) {
            Ok(_) => sliders.push(channel),
            Err(why) if channel == Channel::Alpha => notes.push(fl!(
                "software-unavailable",
                reason = unavailable_reason(why)
            )),
            Err(_) => {}
        }
    }
    Parts {
        lock: sliders.len() == 2,
        sliders,
        notes,
    }
}

/// The hardware and combined floors only mean something once a display has a
/// hardware slider, so like that slider they are absent, not disabled,
/// otherwise (R11). Their saved values are kept for a display that gains
/// hardware control later.
fn shows_hardware_floors(monitors: &[Monitor]) -> bool {
    monitors.iter().any(|m| m.available(Channel::Hardware))
}

/// Secondary text: the theme's text colour at reduced opacity.
fn muted(theme: &cosmic::Theme) -> iced::widget::text::Style {
    let mut color: iced::Color = theme.cosmic().on_bg_color().into();
    color.a *= 0.7;
    iced::widget::text::Style {
        color: Some(color),
        ..Default::default()
    }
}

/// One display under `name`: its notes, its sliders, then the lock.
fn monitor_view(monitor: &Monitor, name: String) -> Element<'_, Message> {
    let parts = parts(monitor);
    let mut view = column::with_capacity(3 + parts.notes.len() + parts.sliders.len())
        .spacing(cosmic::theme::spacing().space_xxxs)
        .push(applet::padded_control(text::heading(name)));
    for note in parts.notes {
        view = view.push(applet::padded_control(
            text::caption(note).class(cosmic::theme::Text::Custom(muted)),
        ));
    }
    for channel in parts.sliders {
        view = view.push(channel_view(monitor, channel));
    }
    if parts.lock {
        let key = monitor.found.key.clone();
        view = view.push(applet::padded_control(
            toggler(monitor.levels().locked())
                .label(fl!("lock"))
                .on_toggle(move |locked| Message::Lock(key.clone(), locked))
                .width(Length::Fill),
        ));
    }
    view.into()
}

/// A channel's slider with its level and write status. Only called for a
/// channel the display has (see [`parts`]).
fn channel_view(monitor: &Monitor, channel: Channel) -> Element<'_, Message> {
    let value = monitor.level(channel);
    // The range starts where the policy stops this slider now, so it cannot
    // be dragged below a floor. A range needs two ends: at 100 % the floor
    // can be 100, and a request for 99 is clamped back by the policy.
    let minimum = monitor.minimum(channel).min(99);
    let key = monitor.found.key.clone();
    let glyph = match channel {
        Channel::Hardware => "video-display-symbolic",
        Channel::Alpha => "display-brightness-symbolic",
    };
    let control = row::with_capacity(3)
        .push(icon::from_name(glyph).size(16).symbolic(true))
        .push(
            slider(minimum..=100, value, {
                let key = key.clone();
                move |percent| Message::Set(key.clone(), channel, percent)
            })
            .on_release(Message::Released(key))
            .width(Length::Fill),
        )
        .push(
            // A fixed width for "100%", so the slider does not change length
            // as the number of digits does.
            text::body(fl!("level", value = value))
                .width(Length::Fixed(44.0))
                .align_x(Alignment::End),
        )
        .spacing(cosmic::theme::spacing().space_xs)
        .align_y(Alignment::Center);
    // Only a failure adds a line: a note on every write would resize the
    // popup, and so shake it, while a slider moves.
    let writes = monitor.writes(channel);
    let status = writes.error().map(|error| {
        let reason = error_reason(error);
        match writes.confirmed() {
            Some(level) => fl!("not-applied-at", reason = reason, value = level),
            None => fl!("not-applied", reason = reason),
        }
    });
    let mut rows = column::with_capacity(2).push(applet::padded_control(control));
    if let Some(status) = status {
        rows = rows.push(applet::padded_control(text::caption(status)));
    }
    rows.into()
}

fn unavailable_reason(why: &Unavailable) -> String {
    match why {
        Unavailable::NotFound => fl!("reason-unsupported"),
        Unavailable::ReadOnly => fl!("reason-read-only"),
        Unavailable::Backend(error) => error_reason(error),
    }
}

/// A failure as the user reads it. The backends' own details are English
/// and technical, so they go to the log (where the write's failure is
/// recorded) and the popup gets a translated reason.
pub(crate) fn error_reason(error: &BackendError) -> String {
    match error {
        BackendError::Unsupported => fl!("reason-unsupported"),
        BackendError::TimedOut => fl!("reason-timed-out"),
        BackendError::UnknownOutput => fl!("reason-not-found"),
        BackendError::InvalidLevel => fl!("reason-invalid-level"),
        BackendError::Unavailable(_) => fl!("reason-unavailable"),
        BackendError::Superseded => fl!("reason-superseded"),
    }
}

/// Wayland positioner rectangles are integer surface coordinates.
#[expect(
    clippy::cast_possible_truncation,
    reason = "panel button bounds are small, finite surface coordinates"
)]
pub(crate) fn anchor_rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle<i32> {
    Rectangle {
        x: x.round() as i32,
        y: y.round() as i32,
        width: width.round() as i32,
        height: height.round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::Found;

    #[test]
    fn reasons_are_short_and_specific() {
        assert_eq!(
            unavailable_reason(&Unavailable::NotFound),
            fl!("reason-unsupported")
        );
        assert_eq!(
            unavailable_reason(&Unavailable::Backend(BackendError::TimedOut)),
            fl!("reason-timed-out")
        );
        assert_eq!(
            error_reason(&BackendError::TimedOut),
            fl!("reason-timed-out")
        );
        let reason = error_reason(&BackendError::Unavailable("bus busy".into()));
        assert_eq!(reason, fl!("reason-unavailable"));
        assert!(
            !reason.contains("bus busy"),
            "backend details stay in the log"
        );
    }

    fn display(hardware: bool) -> Monitor {
        let endpoint = |ok: bool| {
            if ok {
                Ok(dimmer_core::OutputId("TST-00000001-00000001".into()))
            } else {
                Err(Unavailable::NotFound)
            }
        };
        Monitor::new(
            Found {
                key: "TST-00000001-00000001".into(),
                name: "Panel (DP-1)".into(),
                hardware: endpoint(hardware),
                alpha: endpoint(true),
            },
            dimmer_core::MonitorLevels::default(),
            Floors::default(),
        )
    }

    #[test]
    fn a_display_with_hardware_gets_both_sliders_and_the_lock() {
        assert_eq!(
            parts(&display(true)),
            Parts {
                sliders: vec![Channel::Hardware, Channel::Alpha],
                lock: true,
                notes: Vec::new(),
            }
        );
    }

    #[test]
    fn a_display_without_hardware_has_no_hardware_slider_and_no_lock() {
        let monitor = display(false);
        let parts = parts(&monitor);
        assert_eq!(parts.sliders, [Channel::Alpha], "absent, not disabled");
        assert!(!parts.lock);
        assert!(parts.notes.is_empty(), "no note about missing hardware");
        assert_eq!(
            monitor.level(Channel::Hardware),
            100,
            "hardware held at 100 in the model"
        );
    }

    #[test]
    fn the_hardware_floor_settings_need_a_display_with_hardware() {
        assert!(!shows_hardware_floors(&[]));
        assert!(!shows_hardware_floors(&[display(false)]));
        assert!(shows_hardware_floors(&[display(false), display(true)]));
    }

    #[test]
    fn anchor_rect_rounds_to_surface_coordinates() {
        let rect = anchor_rect(10.4, 2.6, 32.0, 31.5);
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (10, 3, 32, 32));
    }
}
