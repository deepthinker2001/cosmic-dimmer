//! The applet's session D-Bus interface: `Raise`, `Lower` and `Reset`, which
//! the brightness shortcuts and `cosmic-ext-dimmer --reset` call.

use cosmic::iced::futures::{SinkExt, channel::mpsc};
use cosmic::iced::{Subscription, stream};

use super::Key;
use crate::APP_ID;

/// The applet's D-Bus name. The interface on [`Service`] has the same name,
/// written out because `#[zbus::interface]` needs a literal; the test
/// `interface_name_is_the_bus_name` keeps the two equal.
pub const BUS_NAME: &str = APP_ID;
pub const OBJECT_PATH: &str = "/io/github/deepthinker2001/CosmicExtDimmer";

/// What the D-Bus service reports to the applet.
#[derive(Clone, Debug)]
pub enum Event {
    Ready,
    Failed(String),
    Key(Key),
}

struct Service {
    events: mpsc::Sender<Event>,
}

#[zbus::interface(name = "io.github.deepthinker2001.CosmicExtDimmer")]
impl Service {
    /// Brightness up on every display: lifts the lowest level (R12).
    fn raise(&self) {
        self.send(Key::Raise);
    }

    /// Brightness down on every display, stopping at the floors.
    fn lower(&self) {
        self.send(Key::Lower);
    }

    /// Every display back to 100 %, removing every overlay: the terminal
    /// recovery `cosmic-ext-dimmer --reset` calls this.
    fn reset(&self) {
        self.send(Key::Reset);
    }
}

impl Service {
    fn send(&self, key: Key) {
        // A full queue means the applet is far behind; dropping a key press
        // is better than blocking the bus.
        if let Err(error) = self.events.clone().try_send(Event::Key(key)) {
            tracing::warn!(%error, "dropped brightness key");
        }
    }
}

/// Serve the key interface on the session bus for as long as the applet runs.
pub fn subscription() -> Subscription<Event> {
    Subscription::run(|| {
        stream::channel(16, async |mut events: mpsc::Sender<Event>| {
            let service = Service {
                events: events.clone(),
            };
            let connection = async {
                zbus::connection::Builder::session()?
                    .name(BUS_NAME)?
                    .serve_at(OBJECT_PATH, service)?
                    .build()
                    .await
            }
            .await;
            let _connection = match connection {
                Ok(connection) => {
                    let _ = events.send(Event::Ready).await;
                    Some(connection)
                }
                Err(error) => {
                    tracing::warn!(%error, "brightness keys unavailable: D-Bus service failed");
                    let _ = events.send(Event::Failed(error.to_string())).await;
                    None
                }
            };
            // Hold the connection (and so the bus name) until the applet exits.
            std::future::pending::<()>().await;
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_name_is_the_bus_name() {
        use zbus::object_server::Interface as _;
        assert_eq!(Service::name().as_str(), BUS_NAME);
    }
}
