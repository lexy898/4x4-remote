//! Test helper: the log events a piece of code writes, with their levels.

use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// The level and the message of each event that `run` logs, in order, at every level.
pub(crate) fn events_of(run: impl FnOnce()) -> Vec<(Level, String)> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&events)));
    tracing::subscriber::with_default(subscriber, run);
    let events = events
        .lock()
        .expect("no test thread panicked while logging");
    events.clone()
}

/// Keeps the level and the message of every event.
struct Recorder(Arc<Mutex<Vec<(Level, String)>>>);

impl<S: Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .expect("no test thread panicked while logging")
            .push((*event.metadata().level(), message.0));
    }
}

/// The message of an event.
#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
