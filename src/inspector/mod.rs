use des::prelude::ModuleRef;

use egui::{Align, Color32, Frame, Grid, Label, Layout, RichText, TextEdit, TextStyle};
use egui_extras::{Column, TableBuilder};

use crate::{
    plot::{PropTracer, Tracer},
    tracing::MakeTracer,
};

mod props;
pub use props::*;

#[derive(Debug, Clone)]
pub struct ModuleInspector {
    pub module: ModuleRef,
    pub filter: String,
    pub logs: MakeTracer,
    pub remove: bool,
}

impl PartialEq for ModuleInspector {
    fn eq(&self, other: &Self) -> bool {
        self.module == other.module
    }
}

impl ModuleInspector {
    pub const fn new(module: ModuleRef, logs: MakeTracer) -> Self {
        Self {
            module,
            filter: String::new(),
            logs,
            remove: false,
        }
    }
}

impl ModuleInspector {
    pub fn show(&mut self, ui: &mut egui::Ui, tracers: &mut Vec<Vec<Box<dyn Tracer>>>) {
        Frame::new().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new(format!("Inspector: {}", self.module.path())).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(RichText::new("X").strong().color(Color32::RED))
                        .clicked()
                    {
                        self.remove = true;
                    }
                });
            });

            ui.separator();

            Grid::new(format!("{}-inspector-grid", self.module.path())).show(ui, |ui| {
                let props = self.module.props();
                for prop in props {
                    ui.label(&prop);

                    let mut reader = PropReader::from_key(&prop, &self.module);

                    // Value
                    if let Some(ref mut reader) = reader {
                        ui.label(reader.get_str());
                    } else {
                        ui.label("???");
                    };

                    if let Some(reader) = reader {
                        if ui.button("O").clicked() {
                            tracers[0].push(Box::new(PropTracer::new(
                                format!("{}.{}", self.module.path(), prop),
                                reader,
                            )));
                        }
                    }

                    ui.end_row();
                }
            });

            ui.separator();

            TextEdit::singleline(&mut self.filter)
                .background_color(Color32::DARK_GRAY)
                .show(ui);

            ui.separator();

            let row_height = ui.text_style_height(&TextStyle::Body);

            let stream = self.logs.streams.lock().unwrap();
            if let Some(events) = stream.get(&self.module.path().to_string()) {
                let matching_events = events
                    .iter()
                    .filter(|v| v.matches(&self.filter))
                    .collect::<Vec<_>>();

                TableBuilder::new(ui)
                    .column(Column::initial(100.0).clip(true).resizable(true))
                    .column(Column::initial(100.0).clip(true).resizable(true))
                    .column(Column::initial(100.0).clip(true).resizable(true))
                    .column(Column::remainder().at_least(50.0))
                    .stick_to_bottom(true)
                    .body(|body| {
                        body.rows(row_height, matching_events.len(), |mut row| {
                            let event = matching_events[row.index()];
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(&event.time).color(color_for_log(&event.level)),
                                );
                            });

                            row.col(|ui| {
                                ui.add(
                                    Label::new(
                                        RichText::new(&event.target)
                                            .text_style(TextStyle::Monospace)
                                            .italics(),
                                    )
                                    .extend(),
                                );
                            });
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(&event.span).text_style(TextStyle::Monospace),
                                );
                            });
                            row.col(|ui| {
                                ui.add(
                                    Label::new(
                                        RichText::new(&event.fields)
                                            .text_style(TextStyle::Monospace),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    });
            }
        });
    }
}

fn color_for_log(level: &str) -> Color32 {
    match level {
        "TRACE" => Color32::from_rgb(0, 128, 0),
        "DEBUG" => Color32::from_rgb(0, 0, 255),
        "INFO" => Color32::from_rgb(0, 255, 0),
        "WARN" => Color32::from_rgb(255, 255, 0),
        "ERROR" => Color32::from_rgb(255, 0, 0),
        _ => Color32::WHITE,
    }
}
