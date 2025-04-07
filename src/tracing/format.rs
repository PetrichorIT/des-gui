use des::{net::module::try_current, time::SimTime};
use serde::{Deserialize, Serialize};
use tracing::Subscriber;
use tracing_subscriber::{
    fmt::{FormatEvent, FormatFields, FormattedFields, format::Writer},
    registry::LookupSpan,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub time: String,
    pub level: String,
    pub module: Option<String>,
    pub target: String,
    pub span: String,
    pub fields: String,
}

impl Event {
    pub fn matches(&self, query: &str) -> bool {
        self.fields.contains(query)
            | self.span.contains(query)
            | self.module.as_ref().map_or(false, |v| v.contains(query))
    }
}

#[derive(Debug)]
pub struct MachineReadableFormat;

impl<S, N> FormatEvent<S, N> for MachineReadableFormat
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
            time: SimTime::now().to_string(),
            level: event.metadata().level().to_string(),
            module: try_current().map(|v| v.path().to_string()),
            target: event.metadata().target().to_string(),
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

        writer.write_str(&serde_json::to_string(&json).unwrap())?;
        writeln!(writer)
    }
}
