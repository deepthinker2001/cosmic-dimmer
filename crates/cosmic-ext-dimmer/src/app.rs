//! The applet itself: its state, its messages and how it handles them.
//!
//! Nothing here waits for a display: every backend call runs as a task on the
//! executor, bounded by the configured timeout, and its result comes back as a
//! message (R10). The popup's widgets are in `view.rs`, the brightness
//! shortcuts in `shortcuts.rs` and auto dimming in `auto.rs`.

use cosmic::app::{Core, Task};
use cosmic::cosmic_config;
use cosmic::iced::window::Id;
use cosmic::iced::{Rectangle, Subscription};
use cosmic::surface::action::{LiveSettings, app_popup, destroy_popup};
use cosmic::widget::text;
use cosmic::{Element, applet, iced};
use dimmer_core::store::{CONFIG_VERSION, MonitorStore};
use dimmer_core::{BackendError, Floors, Level, with_timeout};

use crate::backend::Backends;
use crate::monitor::{self, Channel, Found, Monitor, Write};
use crate::settings::Settings;
use crate::shortcuts::{self, KeyService, ShortcutMessage, Shortcuts};
use crate::{APP_ID, ICON, auto, fl, keys, location, view};

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent UI flags, not a state machine"
)]
pub struct Dimmer {
    pub(crate) core: Core,
    pub(crate) popup: Option<Id>,
    pub(crate) backends: Backends,
    /// The applet's own config namespace; `None` if it cannot be opened, in
    /// which case nothing is saved and defaults are used.
    pub(crate) config: Option<cosmic_config::Config>,
    pub(crate) store: Option<MonitorStore>,
    pub(crate) settings: Settings,
    pub(crate) monitors: Vec<Monitor>,
    pub(crate) discovering: bool,
    pub(crate) discovered: bool,
    pub(crate) show_settings: bool,
    /// The brightness shortcuts and their key service.
    pub(crate) shortcuts: Shortcuts,
    /// Where auto dimming takes sunrise and sunset from; `None` when neither
    /// typed coordinates nor the time zone give a location.
    pub(crate) location: Option<(dimmer_core::sun::Location, location::Source)>,
    /// The location field as typed, until it is submitted.
    pub(crate) location_draft: String,
    /// Whether the submitted location could not be read.
    pub(crate) location_invalid: bool,
    /// The display whose slider is being dragged: auto dimming leaves it
    /// alone until the slider is let go.
    pub(crate) dragging: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup(Rectangle<i32>),
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    Discovered(Vec<Found>),
    /// A slider moved: the display's key ([`Found::key`]), the channel, the
    /// level.
    Set(String, Channel, u8),
    /// A slider was let go: the display's key. Its levels are saved.
    Released(String),
    /// The lock was switched: the display's key, and whether it is on.
    Lock(String, bool),
    Written {
        /// The display's key ([`Found::key`]).
        key: String,
        channel: Channel,
        level: u8,
        result: Result<(), BackendError>,
    },
    ToggleSettings,
    Floors(Floors),
    KeyStep(u8),
    /// From the brightness-key D-Bus service.
    Keys(keys::Event),
    Shortcuts(ShortcutMessage),
    /// The settings changed on disk: by this applet, or by `--reset` or a
    /// hand edit.
    SettingsChanged,
    /// Turn auto dimming on or off.
    SetAuto(bool),
    /// Time for auto dimming to take its next 1 % step.
    AutoTick,
    LocationEdited(String),
    LocationSubmitted,
}

impl cosmic::Application for Dimmer {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        let config = cosmic_config::Config::new(APP_ID, CONFIG_VERSION)
            .inspect_err(|error| tracing::warn!(%error, "settings will not be saved"))
            .ok();
        let settings = config.as_ref().map(Settings::load).unwrap_or_default();
        let mut dimmer = Self {
            core,
            popup: None,
            backends: Backends::from_env(),
            store: config.clone().map(MonitorStore::new),
            config,
            settings,
            monitors: Vec::new(),
            discovering: false,
            discovered: false,
            show_settings: false,
            shortcuts: Shortcuts::default(),
            location: None,
            location_draft: String::new(),
            location_invalid: false,
            dragging: None,
        };
        dimmer.location_draft.clone_from(&dimmer.settings.location);
        dimmer.location = location::location(dimmer.settings.typed_location());
        // Discover at start, so saved levels are restored before the popup opens.
        let task = dimmer.discover();
        (dimmer, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![keys::subscription().map(Message::Keys)];
        if self.shortcuts.capturing.is_some() {
            subscriptions.push(shortcuts::captured_keys());
        }
        subscriptions.push(self.core.watch_config::<Settings>(APP_ID).map(|update| {
            // A missing key is normal; only unreadable ones are worth a line.
            for error in update.errors.iter().filter(|error| error.is_err()) {
                tracing::warn!(%error, "a saved setting is unreadable");
            }
            // The update's own copy can mix old and new values while a save
            // is half written, so the handler reads the files instead.
            Message::SettingsChanged
        }));
        if self.settings.auto_dimming && self.location.is_some() {
            subscriptions.push(iced::time::every(auto::TICK).map(|_| Message::AutoTick));
        }
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePopup(anchor) => return self.toggle_popup(anchor),
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    return self.end_capture();
                }
            }
            Message::Surface(action) => return cosmic::surface::surface_task(action),
            Message::Discovered(found) => return self.discovered(found),
            Message::Set(key, channel, percent) => {
                self.dragging = Some(key.clone());
                return self.with_monitor(&key, false, |m| m.set(channel, percent));
            }
            Message::Released(key) => {
                self.dragging = None;
                if let Some(index) = self.monitors.iter().position(|m| m.found.key == key) {
                    self.learn(index);
                    self.save(&self.monitors[index]);
                }
            }
            Message::Lock(key, locked) => {
                return self.with_monitor(&key, true, |m| {
                    m.set_locked(locked);
                    Vec::new()
                });
            }
            Message::Written {
                key,
                channel,
                level,
                result,
            } => {
                let Some(monitor) = self.monitors.iter_mut().find(|m| m.found.key == key) else {
                    return Task::none();
                };
                // Logged by the backend's display name: keys carry EDID serials.
                if let Err(error) = &result
                    && *error != BackendError::Superseded
                {
                    let label = monitor.found.name.as_str();
                    tracing::warn!(display = label, ?channel, level, %error, "write failed");
                }
                let next = monitor.finished(channel, level, &result);
                if let Some(write) = next {
                    return self.write(&key, write);
                }
            }
            Message::ToggleSettings => self.show_settings = !self.show_settings,
            Message::Floors(floors) => {
                self.settings.set_floors(floors);
                self.save_settings();
                return self.apply_floors(floors);
            }
            Message::SettingsChanged => return self.settings_changed(),
            Message::KeyStep(step) => {
                self.settings.key_step = step.into();
                self.save_settings();
            }
            Message::Keys(event) => return self.key_event(event),
            Message::SetAuto(on) => self.set_auto(on),
            Message::AutoTick => return self.auto_tick(),
            Message::LocationEdited(text) => {
                self.location_draft = text;
                self.location_invalid = false;
            }
            Message::LocationSubmitted => self.submit_location(),
            Message::Shortcuts(message) => return self.shortcut_message(message),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let button =
            self.core
                .applet
                .icon_button(ICON)
                .on_press_with_rectangle(|offset, bounds| {
                    Message::TogglePopup(view::anchor_rect(
                        bounds.x - offset.x,
                        bounds.y - offset.y,
                        bounds.width,
                        bounds.height,
                    ))
                });

        Element::from(self.core.applet.applet_tooltip::<Message>(
            button,
            fl!("applet-name"),
            self.popup.is_some(),
            Message::Surface,
            None,
        ))
    }

    fn view_window(&self, id: Id) -> Element<'_, Message> {
        if self.popup == Some(id) {
            self.core
                .applet
                .popup_container(self.popup_content())
                .into()
        } else {
            text::body("").into()
        }
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(applet::style())
    }
}

impl Dimmer {
    fn toggle_popup(&mut self, anchor: Rectangle<i32>) -> Task<Message> {
        if let Some(id) = self.popup {
            return cosmic::surface::surface_task(destroy_popup(id));
        }
        let open = cosmic::surface::surface_task(app_popup::<Dimmer>(
            |_| LiveSettings::default(),
            move |state: &mut Dimmer| {
                let new_id = Id::unique();
                state.popup = Some(new_id);
                let mut settings = state.core.applet.get_popup_settings(
                    // libcosmic gives every applet's main window
                    // Id::RESERVED; same fallback, without a panic.
                    state.core.main_window_id().unwrap_or(Id::RESERVED),
                    new_id,
                    None,
                    None,
                    None,
                );
                settings.positioner.anchor_rect = anchor;
                settings
            },
            // The content comes from `view_window`, as in the stock applets.
            None,
        ));
        // Look again each time the popup opens: displays come and go, and
        // the shortcuts can change in COSMIC Settings.
        Task::batch([open, self.discover(), shortcuts::read()])
    }

    fn discover(&mut self) -> Task<Message> {
        if self.discovering {
            return Task::none();
        }
        self.discovering = true;
        let backends = self.backends.clone();
        let timeout = self.settings.timeout();
        cosmic::task::future(async move {
            Message::Discovered(
                monitor::discover(&*backends.hardware, &*backends.alpha, timeout).await,
            )
        })
    }

    /// Merge a discovery: known displays keep their state, new ones load
    /// their saved levels and restore them, gone ones are dropped.
    fn discovered(&mut self, found: Vec<Found>) -> Task<Message> {
        self.discovering = false;
        self.discovered = true;
        let floors = self.settings.floors();
        let mut previous = std::mem::take(&mut self.monitors);
        let mut restore = Vec::new();
        for found in found {
            if let Some(index) = previous.iter().position(|m| m.found.key == found.key) {
                let mut monitor = previous.swap_remove(index);
                let writes = monitor.rediscovered(found, floors);
                restore.push((self.monitors.len(), writes));
                self.monitors.push(monitor);
                continue;
            }
            let loaded = match (&self.store, found.edid()) {
                (Some(store), Some(edid)) => {
                    let loaded = store.load(&edid);
                    for issue in &loaded.issues {
                        tracing::warn!(display = found.name, %issue, "saved levels");
                    }
                    Some(loaded)
                }
                _ => None,
            };
            let (levels, profile) =
                loaded.map_or_else(Default::default, |loaded| (loaded.levels, loaded.profile));
            let mut monitor = Monitor::new(found, levels, floors).with_profile(profile);
            restore.push((self.monitors.len(), monitor.restore()));
            self.monitors.push(monitor);
        }
        Task::batch(
            restore
                .into_iter()
                .map(|(index, writes)| self.writes(index, writes)),
        )
    }

    /// Apply `operation` to the display with this key and start its writes.
    fn with_monitor(
        &mut self,
        key: &str,
        save: bool,
        operation: impl FnOnce(&mut Monitor) -> Vec<Write>,
    ) -> Task<Message> {
        let Some(index) = self.monitors.iter().position(|m| m.found.key == key) else {
            return Task::none();
        };
        let writes = operation(&mut self.monitors[index]);
        if save {
            self.save(&self.monitors[index]);
        }
        self.writes(index, writes)
    }

    fn key_event(&mut self, event: keys::Event) -> Task<Message> {
        match event {
            keys::Event::Ready => self.shortcuts.service = KeyService::Ready,
            keys::Event::Failed(error) => self.shortcuts.service = KeyService::Failed(error),
            keys::Event::Key(key) => {
                let step = self.settings.key_step();
                if key == keys::Key::Reset {
                    // A reset is for a screen too dark to use: auto dimming
                    // must not dim it again.
                    self.set_auto(false);
                }
                let mut tasks = Vec::new();
                for index in 0..self.monitors.len() {
                    let monitor = &mut self.monitors[index];
                    let writes = match key {
                        keys::Key::Raise => monitor.raise(step),
                        keys::Key::Lower => monitor.lower(step),
                        keys::Key::Reset => monitor.force_reset(),
                    };
                    if key != keys::Key::Reset {
                        self.learn(index);
                    }
                    self.save(&self.monitors[index]);
                    tasks.push(self.writes(index, writes));
                }
                return Task::batch(tasks);
            }
        }
        Task::none()
    }

    pub(crate) fn writes(&self, index: usize, writes: Vec<Write>) -> Task<Message> {
        let name = &self.monitors[index].found.key;
        Task::batch(writes.into_iter().map(|write| self.write(name, write)))
    }

    /// Start one backend write; its result arrives as [`Message::Written`].
    fn write(&self, key: &str, write: Write) -> Task<Message> {
        let backend = match write.channel {
            Channel::Hardware => self.backends.hardware.clone(),
            Channel::Alpha => self.backends.alpha.clone(),
        };
        let timeout = self.settings.timeout();
        let key = key.to_owned();
        cosmic::task::future(async move {
            let result = match Level::new(write.level) {
                Ok(level) => {
                    use dimmer_core::Backend as _;
                    with_timeout(timeout, backend.set_level(&write.output, level)).await
                }
                Err(error) => Err(error),
            };
            Message::Written {
                key,
                channel: write.channel,
                level: write.level,
                result,
            }
        })
    }

    pub(crate) fn save(&self, monitor: &Monitor) {
        if let (Some(store), Some(edid)) = (&self.store, monitor.edid())
            && let Err(error) = store.save(&edid, monitor.levels(), monitor.profile())
        {
            // The error's entry name holds the display's serial: log the cause only.
            tracing::warn!(error = %error.source, "levels not saved");
        }
    }

    pub(crate) fn save_settings(&self) {
        if let Some(config) = &self.config
            && let Err(error) = self.settings.save(config)
        {
            tracing::warn!(%error, "settings not saved");
        }
    }

    /// Apply new floors to every display, saving and writing what moved.
    fn apply_floors(&mut self, floors: Floors) -> Task<Message> {
        let mut tasks = Vec::new();
        for index in 0..self.monitors.len() {
            let writes = self.monitors[index].set_floors(floors);
            self.save(&self.monitors[index]);
            tasks.push(self.writes(index, writes));
        }
        Task::batch(tasks)
    }

    /// The settings files changed. They are read again, whole, rather than
    /// trusting the watcher's copy, which can be stale mid-save. The applet's
    /// own saves read back unchanged and do nothing; a change made elsewhere
    /// is applied as if it had been made in the popup.
    fn settings_changed(&mut self) -> Task<Message> {
        let Some(config) = &self.config else {
            return Task::none();
        };
        let settings = Settings::read(config);
        if settings == self.settings {
            return Task::none();
        }
        let previous = std::mem::replace(&mut self.settings, settings);
        if self.settings.location != previous.location {
            self.location_draft.clone_from(&self.settings.location);
            self.location_invalid = false;
            self.location = location::location(self.settings.typed_location());
        }
        if self.settings.auto_dimming && !previous.auto_dimming {
            self.seed_profiles();
        }
        if self.settings.floors() == previous.floors() {
            Task::none()
        } else {
            self.apply_floors(self.settings.floors())
        }
    }
}
