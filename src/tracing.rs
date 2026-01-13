use std::sync::{Arc, Mutex};

use des::{
    net::{ObjectPath, module::try_current},
    time::SimTime,
};
use egui::ahash::HashMap;
use serde::{
    Deserialize, Serialize,
    ser::{SerializeMap, SerializeStruct},
};
use tracing::{Metadata, Subscriber};
use tracing_subscriber::{
    fmt::{FormatEvent, FormatFields, FormattedFields, format::Writer},
    registry::LookupSpan,
};

#[derive(Debug, Clone)]
pub struct Event {
    pub time: SimTime,
    pub metadata: &'static Metadata<'static>,
    pub module: ObjectPath,
    pub span: String,
    pub fields: String,
}

impl Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut struc = serializer.serialize_struct("Event", 5)?;
        struc.serialize_field("time", &self.time)?;
        struc.serialize_field("module", &self.module.as_str())?;
        struc.serialize_field("metadata", &SerFieldSet(self.metadata))?;
        struc.serialize_field("span", &self.span)?;
        struc.serialize_field("fields", &self.fields)?;

        struc.end()
    }
}

struct SerFieldSet(&'static Metadata<'static>);

impl Serialize for SerFieldSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut struc = serializer.serialize_map(None)?;
        struc.serialize_entry("target", &self.0.target())?;
        struc.serialize_entry("file", &self.0.file())?;
        struc.serialize_entry("level", &self.0.level().as_str())?;
        struc.end()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Span {
    pub name: String,
    pub fields: String,
}

impl Event {
    pub fn matches(&self, query: &str) -> bool {
        self.fields.contains(query)
            | self.span.contains(query)
            | self.module.as_str().contains(query)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GuiTracingObserver {
    pub streams: Arc<Mutex<HashMap<ObjectPath, ModuleLog>>>,
}

impl<S, N> FormatEvent<S, N> for GuiTracingObserver
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let mut json = Event {
            time: SimTime::now(),
            metadata: event.metadata(),
            module: try_current().ok_or(std::fmt::Error)?.path(),
            span: String::new(),
            fields: String::new(),
        };

        let mut txt_writer = Writer::new(&mut json.span);
        if let Some(scope) = ctx.event_scope() {
            let mut seen = false;
            for span in scope.from_root() {
                write!(&mut txt_writer, "{}", span.metadata().name())?;
                seen = true;
                let ext = span.extensions();
                if let Some(fields) = &ext.get::<FormattedFields<N>>() {
                    if !fields.is_empty() {
                        write!(&mut txt_writer, "{{{fields}}}")?;
                    }
                }
                write!(&mut txt_writer, ":")?;
            }
            if seen {
                writer.write_char(' ')?;
            }
        }

        json.span.pop();

        let mut buf_writer = Writer::new(&mut json.fields);
        ctx.format_fields(buf_writer.by_ref(), event)?;

        let mut streams = self.streams.lock().expect("failed to lock");
        streams.entry(json.module.clone()).or_default().push(json);

        Ok(())
    }
}

/// The totality of logs for a given module.
///
/// desired output:
///
/// [t0 ... t1] span, span, span
///  [t0] target message values
#[derive(Debug, Default)]
pub struct ModuleLog {
    events: Vec<Event>,
}

impl ModuleLog {
    pub fn output(&self) -> &[Event] {
        &self.events
    }

    pub fn push(&mut self, event: Event) {
        self.events.push(event.clone());
    }
}
