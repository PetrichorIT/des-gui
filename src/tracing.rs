use std::{
    fmt::Display,
    ops::{Deref, DerefMut},
    str::FromStr,
    sync::{Arc, Mutex},
};

use des::{
    net::{ObjectPath, module::try_current},
    time::SimTime,
};
use egui::ahash::HashMap;
use serde::{Deserialize, Serialize};
use tracing::{Metadata, Subscriber};
use tracing_subscriber::{
    fmt::{FormatEvent, FormatFields, FormattedFields, format::Writer},
    registry::LookupSpan,
};

#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct StringEncoded<T>(pub T);

impl<T> Deref for StringEncoded<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for StringEncoded<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> Serialize for StringEncoded<T>
where
    T: Display,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de, T> Deserialize<'de> for StringEncoded<T>
where
    T: FromStr,
    T::Err: Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        T::from_str(&s)
            .map(StringEncoded)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub time: SimTime,
    pub metadata: &'static Metadata<'static>,
    pub module: ObjectPath,
    pub span: String,
    pub fields: String,
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
    pub streams: Arc<Mutex<HashMap<ObjectPath, Vec<Event>>>>,
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
