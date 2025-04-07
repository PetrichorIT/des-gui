mod format;

use std::{
    io,
    sync::{Arc, Mutex},
};

use egui::ahash::HashMap;
pub use format::*;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Debug, Clone)]
pub struct MakeTracer {
    pub all: Arc<Mutex<String>>,
    pub streams: Arc<Mutex<HashMap<String, Vec<Event>>>>,
}

impl MakeTracer {
    pub fn new() -> Self {
        Self {
            all: Arc::new(Mutex::new(String::new())),
            streams: Arc::new(Mutex::new(HashMap::default())),
        }
    }
}

impl<'a> MakeWriter<'a> for MakeTracer {
    type Writer = Tracer;
    fn make_writer(&'a self) -> Self::Writer {
        Tracer {
            all: self.all.clone(),
            streams: self.streams.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tracer {
    all: Arc<Mutex<String>>,
    streams: Arc<Mutex<HashMap<String, Vec<Event>>>>,
}

impl io::Write for Tracer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(event) = serde_json::from_slice::<Event>(buf) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid JSON"));
        };

        self.all
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(buf)); // needs formating

        let Some(ref module) = event.module else {
            return Ok(buf.len());
        };

        let mut streams = self.streams.lock().unwrap();

        let stream = streams.entry(module.clone()).or_default();
        stream.push(event);

        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
