use des::{net::module::RawProp, time::SimTime};
use egui::{Context, ScrollArea, SidePanel, panel::Side};
use egui_plot::{Legend, Line, Plot, PlotPoint, PlotPoints};
use serde_yml::Value;

use crate::Application;

impl Application {
    pub fn show_plot(&mut self, ctx: &Context) {
        while self.traces.len() > 1 && self.traces[self.traces.len() - 1].is_empty() {
            self.traces.pop();
        }

        SidePanel::new(Side::Right, "plot").show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                for (i, plot) in self.traces.iter().enumerate() {
                    Plot::new(format!("plot-{}", i))
                        .legend(Legend::default())
                        .view_aspect(2.0)
                        .show(ui, |ui| {
                            for trace in plot {
                                let line = Line::new(trace.points()).name(trace.name());
                                ui.line(line);
                            }
                        });

                    for (j, trace) in plot.into_iter().enumerate() {
                        if i > 0 && ui.button(format!("^ {}", trace.name())).clicked() {
                            let value = self.traces[i].remove(j);
                            self.traces[i - 1].push(value);
                            return;
                        }

                        if ui.button(format!("v {}", trace.name())).clicked() {
                            let value = self.traces[i].remove(j);
                            if (i + 1) == self.traces.len() {
                                self.traces.push(vec![value]);
                            } else {
                                self.traces[i + 1].push(value);
                            }
                            return;
                        }
                    }
                }
            })
        });
    }
}

pub trait Tracer {
    fn name(&self) -> String;
    fn update(&mut self);
    fn points(&self) -> PlotPoints<'_>;
}

pub struct PropTracer {
    key: String,
    prop: RawProp,
    values: Vec<PlotPoint>,
}

impl PropTracer {
    pub const fn new(key: String, prop: RawProp) -> Self {
        Self {
            key,
            prop,
            values: Vec::new(),
        }
    }
}

impl Tracer for PropTracer {
    fn name(&self) -> String {
        self.key.clone()
    }

    fn update(&mut self) {
        if let Some(y) = self.prop.into_value().and_then(|value| match value {
            Value::Number(n) => n.as_f64(),
            _ => None,
        }) {
            let x = SimTime::now().as_secs_f64();
            if let Some(last_y) = self.values.last().map(|p| p.y) {
                if last_y != y {
                    self.values.push(PlotPoint { x, y: last_y }); // make a stepper
                    self.values.push(PlotPoint { x, y });
                }
            } else {
                self.values.push(PlotPoint { x, y });
            }
        }
    }

    fn points(&self) -> PlotPoints<'_> {
        PlotPoints::Borrowed(&self.values)
    }
}
