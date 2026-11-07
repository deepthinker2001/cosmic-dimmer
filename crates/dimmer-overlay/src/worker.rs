//! The overlay's own thread and Wayland connection.
//!
//! One calloop loop runs for the backend's lifetime with three sources: the
//! command channel from [`crate::OverlayBackend`], a timer that (re)connects,
//! and, while connected, the connection's socket. The protocol calls are the
//! ones a prototype proved on stock cosmic-comp; what to call when is
//! decided by [`crate::state::Overlay`].
//!
//! Errors at this boundary (no socket, a missing global, a protocol error, the
//! compositor going away, a failed allocation) become an `Unavailable` status
//! and a reconnect with backoff, never a panic: the applet's release profile
//! aborts on panic, which would take the applet out of the panel. Dropping
//! the connection makes the compositor remove every surface, so any failure
//! leaves the screen undimmed rather than stuck dark.

use std::{
    collections::BTreeMap, io, os::unix::net::UnixStream, path::PathBuf, sync::Arc, time::Duration,
};

use dimmer_core::{BackendError, Level, OutputId};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction,
            channel::{self, Channel},
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        client::{
            Connection, EventQueue, QueueHandle,
            backend::WaylandError,
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use tokio::sync::{oneshot, watch};

use crate::{
    connect::{self, BACKOFF_START},
    identity::{self, Resolution},
    state::{Action, Handle, OutputView, Overlay, black_pixel, buffer_bytes},
};

/// The layer-shell namespace, visible to compositor rules and debugging tools.
pub const NAMESPACE: &str = "cosmic-dimmer-overlay";

pub(crate) type Reply = oneshot::Sender<Result<(), BackendError>>;

pub(crate) enum Command {
    SetLevel {
        id: OutputId,
        level: Level,
        reply: Reply,
    },
}

/// Where the worker connects and reads EDIDs from.
#[derive(Clone, Debug)]
pub struct Config {
    /// The compositor's socket; `None` means "from `WAYLAND_DISPLAY`", read
    /// at each connection attempt.
    pub socket: Option<PathBuf>,
    /// Normally [`identity::SYSFS_DRM`].
    pub sysfs_root: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket: None,
            sysfs_root: PathBuf::from(identity::SYSFS_DRM),
        }
    }
}

/// The connection's state as callers see it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Connecting,
    Connected,
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub status: Status,
    pub outputs: Vec<OutputView>,
    /// Overlay surfaces that exist now (zero while nothing is dimmed).
    pub surfaces: usize,
}

/// Start the worker thread. The returned sender's drop (with every clone)
/// ends the thread, which closes the connection and so removes every surface.
pub(crate) fn spawn(
    config: Config,
) -> (Option<channel::Sender<Command>>, watch::Receiver<Snapshot>) {
    let (snapshot_tx, snapshot_rx) = watch::channel(Snapshot {
        status: Status::Connecting,
        outputs: Vec::new(),
        surfaces: 0,
    });
    let (command_tx, command_rx) = channel::channel();
    let snapshot = Arc::new(snapshot_tx);
    let thread_snapshot = Arc::clone(&snapshot);
    let spawned = std::thread::Builder::new()
        .name("dimmer-overlay".into())
        .spawn(move || run(config, command_rx, &thread_snapshot));
    match spawned {
        Ok(_) => (Some(command_tx), snapshot_rx),
        Err(error) => {
            snapshot.send_replace(Snapshot {
                status: Status::Unavailable(format!("could not start the overlay thread: {error}")),
                outputs: Vec::new(),
                surfaces: 0,
            });
            (None, snapshot_rx)
        }
    }
}

fn run(config: Config, commands: Channel<Command>, snapshot: &Arc<watch::Sender<Snapshot>>) {
    let fail = |reason: String| {
        tracing::error!("overlay worker stopped: {reason}");
        snapshot.send_replace(Snapshot {
            status: Status::Unavailable(reason),
            outputs: Vec::new(),
            surfaces: 0,
        });
    };
    let mut event_loop = match EventLoop::<Worker>::try_new() {
        Ok(event_loop) => event_loop,
        Err(error) => return fail(format!("event loop: {error}")),
    };
    let mut worker = Worker {
        config,
        session: None,
        parked: Overlay::default(),
        snapshot: Arc::clone(snapshot),
        handle: event_loop.handle(),
        signal: event_loop.get_signal(),
        backoff: BACKOFF_START,
        status: Status::Connecting,
    };
    let inserted = event_loop
        .handle()
        .insert_source(commands, |event, (), worker| match event {
            channel::Event::Msg(command) => worker.command(command),
            // Every OverlayBackend is gone: stop, closing the connection.
            channel::Event::Closed => worker.signal.stop(),
        });
    if let Err(error) = inserted {
        return fail(format!("command channel: {error}"));
    }
    if let Err(error) = worker.schedule_connect(Duration::ZERO) {
        return fail(error);
    }
    if let Err(error) = event_loop.run(None, &mut worker, |_| {}) {
        fail(format!("event loop: {error}"));
    }
}

struct Worker {
    config: Config,
    session: Option<Connected>,
    /// The overlay state while disconnected (levels survive reconnects).
    parked: Overlay<Reply>,
    snapshot: Arc<watch::Sender<Snapshot>>,
    handle: LoopHandle<'static, Worker>,
    signal: LoopSignal,
    backoff: Duration,
    status: Status,
}

struct Connected {
    connection: Connection,
    queue: EventQueue<Session>,
    session: Session,
}

impl Worker {
    fn publish(&self) {
        let (outputs, surfaces) = match &self.session {
            Some(connected) => (
                connected.session.overlay.outputs(),
                connected.session.surfaces.len(),
            ),
            None => (Vec::new(), 0),
        };
        self.snapshot.send_replace(Snapshot {
            status: self.status.clone(),
            outputs,
            surfaces,
        });
    }

    fn command(&mut self, command: Command) {
        let Command::SetLevel { id, level, reply } = command;
        let Some(connected) = self.session.as_mut() else {
            let reason = match &self.status {
                Status::Unavailable(reason) => reason.clone(),
                _ => "not connected to the compositor yet".into(),
            };
            let _ = reply.send(Err(BackendError::Unavailable(reason)));
            return;
        };
        let actions = connected.session.overlay.set_level(&id, level, reply);
        connected.session.apply(actions, &connected.queue.handle());
        if let Err(error) = flush(&connected.connection) {
            self.disconnect(&error);
        }
        self.publish();
    }

    fn schedule_connect(&self, after: Duration) -> Result<(), String> {
        self.handle
            .insert_source(Timer::from_duration(after), |_, (), worker| {
                match worker.connect() {
                    Ok(()) => TimeoutAction::Drop,
                    Err(reason) => {
                        tracing::warn!("overlay: cannot connect to the compositor: {reason}");
                        worker.status = Status::Unavailable(reason);
                        worker.publish();
                        let wait = worker.backoff;
                        worker.backoff = connect::next_backoff(wait);
                        TimeoutAction::ToDuration(wait)
                    }
                }
            })
            .map(|_| ())
            .map_err(|error| format!("timer: {error}"))
    }

    fn connect(&mut self) -> Result<(), String> {
        let path = match &self.config.socket {
            Some(path) => path.clone(),
            None => connect::host_socket_path(
                std::env::var_os("WAYLAND_DISPLAY").as_deref(),
                std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
            )
            .map_err(|error| error.to_string())?,
        };
        let stream = UnixStream::connect(&path).map_err(|error| format!("socket: {error}"))?;
        let connection =
            Connection::from_socket(stream).map_err(|error| format!("connection: {error}"))?;
        // Blocking round trips, on this thread only: the UI never waits here
        // except through a caller's own timeout.
        let (globals, mut queue) = registry_queue_init::<Session>(&connection)
            .map_err(|error| format!("registry: {error}"))?;
        let qh = queue.handle();
        let mut session = Session {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            compositor: CompositorState::bind(&globals, &qh)
                .map_err(|error| format!("wl_compositor: {error}"))?,
            shell: LayerShell::bind(&globals, &qh)
                .map_err(|error| format!("the compositor offers no wlr-layer-shell: {error}"))?,
            shm: Shm::bind(&globals, &qh).map_err(|error| format!("wl_shm: {error}"))?,
            overlay: std::mem::take(&mut self.parked),
            surfaces: BTreeMap::new(),
            wl_outputs: BTreeMap::new(),
            sysfs_root: self.config.sysfs_root.clone(),
            failed: None,
        };
        // Second round trip: every output's name and description arrive, so
        // callers see the outputs as soon as the status says connected.
        if let Err(error) = queue.roundtrip(&mut session) {
            self.parked = std::mem::take(&mut session.overlay);
            return Err(format!("initial round trip: {error}"));
        }
        let source = Generic::new(connection.clone(), Interest::READ, Mode::Level);
        let inserted = self
            .handle
            .insert_source(source, |_, _, worker| Ok(worker.readable()));
        if let Err(error) = inserted {
            self.parked = std::mem::take(&mut session.overlay);
            return Err(format!("watching the socket: {error}"));
        }
        tracing::info!("overlay: connected to the compositor");
        self.session = Some(Connected {
            connection,
            queue,
            session,
        });
        self.backoff = BACKOFF_START;
        self.status = Status::Connected;
        if let Some(connected) = self.session.as_mut()
            && let Err(error) = flush(&connected.connection)
        {
            self.disconnect(&error);
            return Ok(()); // a reconnect is already scheduled
        }
        self.publish();
        Ok(())
    }

    /// The socket is readable: read, dispatch, flush.
    fn readable(&mut self) -> PostAction {
        let Some(connected) = self.session.as_mut() else {
            return PostAction::Remove;
        };
        let result = dispatch(connected);
        match result {
            Ok(()) => {
                self.publish();
                PostAction::Continue
            }
            Err(error) => {
                self.disconnect(&error);
                PostAction::Remove
            }
        }
    }

    fn disconnect(&mut self, reason: &str) {
        tracing::warn!("overlay: compositor connection lost: {reason}");
        if let Some(mut connected) = self.session.take() {
            let replies = connected.session.overlay.disconnected();
            for action in replies {
                if let Action::Reply(reply, result) = action {
                    let _ = reply.send(result);
                }
            }
            self.parked = std::mem::take(&mut connected.session.overlay);
            // Dropping `connected` drops every proxy and the socket.
        }
        self.status = Status::Unavailable(format!("compositor connection lost: {reason}"));
        self.publish();
        let wait = self.backoff;
        self.backoff = connect::next_backoff(wait);
        if let Err(error) = self.schedule_connect(wait) {
            tracing::error!("overlay: cannot schedule a reconnect: {error}");
            self.signal.stop();
        }
    }
}

fn flush(connection: &Connection) -> Result<(), String> {
    match connection.flush() {
        Ok(()) => Ok(()),
        // The compositor is slow to read; the next flush sends the rest.
        Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn dispatch(connected: &mut Connected) -> Result<(), String> {
    if let Some(guard) = connected.queue.prepare_read() {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    connected
        .queue
        .dispatch_pending(&mut connected.session)
        .map_err(|error| error.to_string())?;
    if let Some(error) = connected.session.failed.take() {
        return Err(error);
    }
    flush(&connected.connection)
}

/// One `wl_output`'s overlay surface and the buffer it shows. The buffer and
/// its pool are kept until replaced or destroyed, so the compositor never
/// reads memory the client has let go of.
struct OutputSurface {
    layer: LayerSurface,
    pool: Option<SlotPool>,
    buffer: Option<Buffer>,
}

/// The Wayland dispatch state for one connection.
struct Session {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    shell: LayerShell,
    shm: Shm,
    overlay: Overlay<Reply>,
    surfaces: BTreeMap<Handle, OutputSurface>,
    wl_outputs: BTreeMap<Handle, wl_output::WlOutput>,
    sysfs_root: PathBuf,
    /// Set by a handler that hit an error the connection cannot survive.
    failed: Option<String>,
}

impl Session {
    fn identify(&self, output: &wl_output::WlOutput) -> Option<(Handle, OutputId, String)> {
        let info = self.outputs.info(output)?;
        let connector = info.name.clone().unwrap_or_default();
        let id = match identity::resolve_sysfs(&self.sysfs_root, &connector) {
            Resolution::Edid(edid) => identity::edid_output_id(&edid),
            other => {
                // Logged by connector only: EDID serials stay out of logs.
                tracing::info!("overlay: output {connector:?} has no usable EDID ({other:?})");
                if connector.is_empty() {
                    OutputId(format!("wl-output:{}", info.id))
                } else {
                    identity::connector_output_id(&connector)
                }
            }
        };
        let name = match (info.model.as_str(), connector.as_str()) {
            // Nothing to name it by: the applet shows its own, translated,
            // word for an unnamed display.
            ("" | "Unknown", "") => String::new(),
            ("" | "Unknown", connector) => connector.to_owned(),
            (model, "") => model.to_owned(),
            (model, connector) => format!("{model} ({connector})"),
        };
        Some((info.id, id, name))
    }

    fn handle_of(&self, layer: &LayerSurface) -> Option<Handle> {
        self.surfaces
            .iter()
            .find(|(_, surface)| &surface.layer == layer)
            .map(|(handle, _)| *handle)
    }

    /// Execute the state machine's actions. Failures feed back into it as a
    /// close, so they are bounded the same way.
    fn apply(&mut self, actions: Vec<Action<Reply>>, qh: &QueueHandle<Self>) {
        let mut queue = std::collections::VecDeque::from(actions);
        while let Some(action) = queue.pop_front() {
            let outcome = match action {
                Action::Create(handle) => self.create(handle, qh),
                Action::Draw {
                    output,
                    size,
                    alpha,
                } => self.draw(output, size, alpha),
                Action::Commit(handle) => {
                    if let Some(surface) = self.surfaces.get(&handle) {
                        surface.layer.commit();
                    }
                    Ok(())
                }
                Action::Destroy(handle) => {
                    // Dropping the LayerSurface destroys the layer surface and
                    // its wl_surface; the buffer goes once the compositor
                    // releases it.
                    self.surfaces.remove(&handle);
                    Ok(())
                }
                Action::Reply(reply, result) => {
                    let _ = reply.send(result);
                    Ok(())
                }
            };
            if let Err((handle, reason)) = outcome {
                tracing::warn!("overlay: dropping a surface: {reason}");
                queue.extend(self.overlay.closed(handle));
            }
        }
    }

    fn create(&mut self, handle: Handle, qh: &QueueHandle<Self>) -> Result<(), (Handle, String)> {
        let output = self
            .wl_outputs
            .get(&handle)
            .ok_or((handle, "unknown output".to_owned()))?;
        // An empty region, NOT None: None restores the default infinite input
        // region and the overlay would swallow every click. Set before the
        // first commit.
        let region = Region::new(&self.compositor)
            .map_err(|error| (handle, format!("input region: {error}")))?;
        let surface = self.compositor.create_surface(qh);
        surface.set_input_region(Some(region.wl_region()));
        drop(region);
        let layer = self.shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            Some(output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // No buffer until configure; SCTK acknowledges the configure serial.
        layer.commit();
        self.surfaces.insert(
            handle,
            OutputSurface {
                layer,
                pool: None,
                buffer: None,
            },
        );
        Ok(())
    }

    fn draw(
        &mut self,
        handle: Handle,
        size: (u32, u32),
        alpha: u8,
    ) -> Result<(), (Handle, String)> {
        let fail = |reason: &str| (handle, reason.to_owned());
        let surface = self
            .surfaces
            .get_mut(&handle)
            .ok_or_else(|| fail("no surface to draw on"))?;
        let bytes = buffer_bytes(size.0, size.1).ok_or_else(|| fail("invalid size"))?;
        let width = i32::try_from(size.0).map_err(|_| fail("width"))?;
        let height = i32::try_from(size.1).map_err(|_| fail("height"))?;
        let stride = width.checked_mul(4).ok_or_else(|| fail("stride"))?;
        if surface.pool.is_none() {
            surface.pool =
                Some(SlotPool::new(bytes, &self.shm).map_err(|error| fail(&error.to_string()))?);
        }
        let pool = surface.pool.as_mut().ok_or_else(|| fail("no pool"))?;
        // A new buffer every time: the previous one may still be in the
        // compositor's hands, so it is never written again.
        let (buffer, canvas) = pool
            .create_buffer(width, height, stride, wl_shm::Format::Argb8888)
            .map_err(|error| fail(&error.to_string()))?;
        let pixel = black_pixel(alpha);
        for chunk in canvas.as_chunks_mut::<4>().0 {
            *chunk = pixel;
        }
        // Buffer scale stays 1: configure sizes are logical, and a constant
        // colour needs no HiDPI detail. No frame callbacks, no redraw loop.
        let wl_surface = surface.layer.wl_surface();
        wl_surface.damage_buffer(0, 0, width, height);
        buffer
            .attach_to(wl_surface)
            .map_err(|error| fail(&format!("attach: {error:?}")))?;
        surface.layer.commit();
        surface.buffer = Some(buffer);
        tracing::debug!("overlay: drew {}x{} at alpha {alpha}", size.0, size.1);
        Ok(())
    }
}

impl LayerShellHandler for Session {
    fn closed(&mut self, _: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface) {
        // Respect the compositor: drop it; only a new user level recreates it.
        if let Some(handle) = self.handle_of(layer) {
            tracing::info!("overlay: the compositor closed a surface");
            let actions = self.overlay.closed(handle);
            self.apply(actions, qh);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(handle) = self.handle_of(layer) else {
            return;
        };
        let (width, height) = configure.new_size;
        let actions = self.overlay.configure(handle, width, height);
        self.apply(actions, qh);
    }
}

impl OutputHandler for Session {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some((handle, id, name)) = self.identify(&output) else {
            return;
        };
        tracing::info!("overlay: output added: {name}");
        self.wl_outputs.insert(handle, output);
        if let Some(actions) = self.overlay.output_added(handle, id, name) {
            self.apply(actions, qh);
        } else {
            self.wl_outputs.remove(&handle);
            tracing::warn!("overlay: output limit reached; this output stays undimmed");
        }
    }

    fn update_output(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Size changes arrive as layer configures. Only the identity is
        // rechecked here, in case the EDID was unreadable when it appeared.
        let Some((handle, id, _)) = self.identify(&output) else {
            return;
        };
        let actions = self.overlay.output_identified(handle, id);
        self.apply(actions, qh);
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let handle = self
            .wl_outputs
            .iter()
            .find(|(_, known)| **known == output)
            .map(|(handle, _)| *handle);
        if let Some(handle) = handle {
            tracing::info!("overlay: output removed");
            let actions = self.overlay.output_removed(handle);
            self.apply(actions, qh);
            self.wl_outputs.remove(&handle);
        }
    }
}

impl CompositorHandler for Session {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for Session {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Session {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Session);
delegate_output!(Session);
delegate_layer!(Session);
delegate_shm!(Session);
delegate_registry!(Session);
