//! Test helper: the log events a piece of code writes, with their levels and fields.

use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// One event as it was logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) level: Level,
    pub(crate) message: String,
    /// Every other field, by name, with its value as `Display` or `Debug` writes it.
    pub(crate) fields: Vec<(String, String)>,
}

impl Record {
    /// The value of the field `name`, if the event has it.
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The level and the message of each event that `run` logs, in order, at every level.
pub(crate) fn events_of(run: impl FnOnce()) -> Vec<(Level, String)> {
    records_of(run)
        .into_iter()
        .map(|record| (record.level, record.message))
        .collect()
}

/// Each event that `run` logs, in order, at every level, with its fields.
pub(crate) fn records_of(run: impl FnOnce()) -> Vec<Record> {
    let records = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&records)));
    tracing::subscriber::with_default(subscriber, run);
    let records = records
        .lock()
        .expect("no test thread panicked while logging");
    records.clone()
}

/// Keeps every event.
struct Recorder(Arc<Mutex<Vec<Record>>>);

impl<S: Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = Fields::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .expect("no test thread panicked while logging")
            .push(Record {
                level: *event.metadata().level(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

/// The message of an event and its other fields.
#[derive(Default)]
struct Fields {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .push((field.name().to_owned(), value.to_owned()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.push((field.name().to_owned(), value));
        }
    }
}
