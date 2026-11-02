//! The compositor's side: one `ext_idle_notification_v1`, turned into a boolean on a channel.
//!
//! The plain notification (not the v2 "input" one) on purpose: it honours idle inhibitors, so a
//! video playing full screen counts as somebody watching, and the machine is kept smooth for it.

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::{
    self, ExtIdleNotifierV1,
};

pub enum SetupError {
    /// No compositor to talk to (yet). Worth a restart: at login the unit can start before the
    /// session has exported `WAYLAND_DISPLAY`.
    NoCompositor(anyhow::Error),
    /// The compositor is there but does not offer the protocol. Restarting will not change that.
    Unsupported(&'static str),
}

pub struct Watcher {
    queue: EventQueue<State>,
    state: State,
    // Held so the notification lives as long as the watcher.
    _notification: ExtIdleNotificationV1,
}

struct State {
    idle: watch::Sender<bool>,
}

pub fn connect(idle_after: Duration, idle: watch::Sender<bool>) -> Result<Watcher, SetupError> {
    let conn = Connection::connect_to_env()
        .context("no Wayland compositor to ask (is WAYLAND_DISPLAY set?)")
        .map_err(SetupError::NoCompositor)?;
    let (globals, queue) = registry_queue_init::<State>(&conn)
        .context("could not read the compositor's globals")
        .map_err(SetupError::NoCompositor)?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| SetupError::Unsupported("the compositor offers no seat"))?;
    let notifier: ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| SetupError::Unsupported("the compositor does not offer ext-idle-notify-v1"))?;
    let timeout_ms = u32::try_from(idle_after.as_millis()).unwrap_or(u32::MAX);
    let notification = notifier.get_idle_notification(timeout_ms, &seat, &qh, ());
    Ok(Watcher {
        queue,
        state: State { idle },
        _notification: notification,
    })
}

impl Watcher {
    /// Blocks for as long as the compositor is there. Returns only with the reason it went away.
    pub fn run(mut self) -> anyhow::Error {
        loop {
            if let Err(e) = self.queue.blocking_dispatch(&mut self.state) {
                return anyhow::Error::new(e).context("the connection to the compositor ended");
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                state.idle.send_replace(true);
            }
            ext_idle_notification_v1::Event::Resumed => {
                state.idle.send_replace(false);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
