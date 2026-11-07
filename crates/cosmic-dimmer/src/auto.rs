//! Auto dimming in the applet: turning it on and off, learning the user's
//! choices, and stepping each display toward its profile on a timer. The
//! sun's cycle and the profile itself are in dimmer-core (`sun`, `profile`);
//! where the user is comes from `location.rs`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cosmic::app::Task;
use cosmic::iced::Length;
use cosmic::widget::{column, text, text_input};
use cosmic::{Element, applet};
use dimmer_core::sun::{self, Location, Phase};

use crate::app::{Dimmer, Message};
use crate::fl;
use crate::location::{Source, location, parse_coordinates};

/// How often auto dimming moves each display one percent toward its target.
pub const TICK: Duration = Duration::from_secs(2);

/// Where the sun's cycle is now.
#[must_use]
pub fn phase_now(location: Location) -> Phase {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    sun::phase(location, now)
}

impl Dimmer {
    /// Turn auto dimming on or off. Turning it on gives each display with no
    /// profile yet a first point, its current levels now, so it holds steady
    /// until the user sets levels at other times of day.
    pub(crate) fn set_auto(&mut self, on: bool) {
        self.settings.auto_dimming = on;
        self.save_settings();
        if on {
            self.seed_profiles();
        }
    }

    /// Give each display with no profile yet a first point: its current
    /// levels, now.
    pub(crate) fn seed_profiles(&mut self) {
        for index in 0..self.monitors.len() {
            if self.monitors[index].profile().is_empty() {
                self.learn(index);
                self.save(&self.monitors[index]);
            }
        }
    }

    /// While auto dimming is on, remember the display's levels as the user's
    /// choice for this time of day.
    pub(crate) fn learn(&mut self, index: usize) {
        if !self.settings.auto_dimming {
            return;
        }
        if let Some((location, _)) = &self.location {
            let phase = phase_now(*location);
            self.monitors[index].learn(phase);
        }
    }

    /// Move every display one percent toward its profile's level for now.
    /// The steps are not saved: the profile, not the level, is what persists.
    pub(crate) fn auto_tick(&mut self) -> Task<Message> {
        // A tick queued before a reset turned auto dimming off must not
        // move anything.
        if !self.settings.auto_dimming {
            return Task::none();
        }
        let Some((location, _)) = &self.location else {
            return Task::none();
        };
        let phase = phase_now(*location);
        let mut tasks = Vec::new();
        for index in 0..self.monitors.len() {
            let monitor = &mut self.monitors[index];
            if self.dragging.as_deref() == Some(monitor.found.key.as_str()) {
                continue;
            }
            if let Some(target) = monitor.profile().target(phase) {
                let writes = monitor.follow(target);
                tasks.push(self.writes(index, writes));
            }
        }
        Task::batch(tasks)
    }

    /// Use the typed location, or the time zone's when the field is empty.
    pub(crate) fn submit_location(&mut self) {
        let typed = self.location_draft.trim();
        if !typed.is_empty() && parse_coordinates(typed).is_none() {
            self.location_invalid = true;
            return;
        }
        self.location_invalid = false;
        self.settings.location = typed.to_owned();
        self.save_settings();
        self.location = location(self.settings.typed_location());
    }

    /// The location field, with where sunrise and sunset come from as its
    /// placeholder.
    pub(crate) fn location_view(&self) -> Element<'_, Message> {
        let placeholder = match &self.location {
            Some((_, Source::TimeZone(zone))) => fl!("location-time-zone", zone = zone.as_str()),
            _ => fl!("location-placeholder"),
        };
        let space = cosmic::theme::spacing().space_xxxs;
        let mut section = column::with_capacity(3)
            .spacing(space)
            .push(applet::padded_control(
                column::with_capacity(2)
                    .spacing(space)
                    .push(text::body(fl!("location")))
                    .push(
                        text_input(placeholder, &self.location_draft)
                            .on_input(Message::LocationEdited)
                            .on_submit(|_| Message::LocationSubmitted)
                            .on_unfocus(Message::LocationSubmitted)
                            .width(Length::Fill),
                    ),
            ));
        if self.location_invalid {
            section = section.push(applet::padded_control(text::caption(fl!(
                "location-invalid"
            ))));
        } else if self.location.is_none() {
            section = section.push(applet::padded_control(text::caption(fl!(
                "location-unknown"
            ))));
        }
        section.into()
    }
}
