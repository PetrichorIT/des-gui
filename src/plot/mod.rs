use des::{net::ObjectPath, time::SimTime};
use egui::{Context, ScrollArea, SidePanel, panel::Side};
use egui_plot::{Legend, Line, Plot, PlotPoint, PlotPoints};
use fxhash::FxHashMap;
use serde_norway::Value;

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
    fn needs_path(&self, path: &ObjectPath) -> bool;
    fn update(&mut self, values: &FxHashMap<ObjectPath, Value>);
    fn points(&self) -> PlotPoints<'_>;
}

pub struct TreeTracer {
    path: ObjectPath,
    key: String,
    values: Vec<PlotPoint>,
}

impl TreeTracer {
    pub fn new(module: ObjectPath, key: String) -> Self {
        Self {
            path: module,
            key,
            values: Vec::new(),
        }
    }
}

impl Tracer for TreeTracer {
    fn name(&self) -> String {
        format!("{} {}", self.path, self.key)
    }

    fn needs_path(&self, path: &ObjectPath) -> bool {
        self.path == *path
    }

    fn update(&mut self, values: &FxHashMap<ObjectPath, Value>) {
        let map = values.get(&self.path).expect("message not observed");

        if let Some(y) = access(map, &self.key).and_then(|v| v.as_f64()) {
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

pub fn access(value: &Value, key: &str) -> Option<Value> {
    match value {
        other if key.is_empty() => Some(other.clone()),
        Value::Mapping(map) => {
            let mut include = key.len();
            while include > 0 {
                // TODO: This shit is still buggy
                let pos = key[..include].rfind('.').unwrap_or(include);
                let subkey = &key[..pos];
                if let Some(val) = map.get(subkey) {
                    return access(val, &key[(pos + 1).min(key.len())..]);
                }
                include = pos - 1;
            }

            // try full key
            if let Some(val) = map.get(key) {
                return access(val, "");
            }

            None
        }
        Value::Sequence(seq) => {
            let (index, rem) = key.split_once('.').unwrap_or((key, ""));
            let index = index.parse::<usize>().ok()?;
            let element = seq.get(index)?;
            access(element, rem)
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_norway::{Mapping, Sequence};

    use super::*;

    #[test]
    fn access_multi_keys() {
        let value = Value::Mapping(Mapping::from_iter([(
            Value::String("inet".to_string()),
            Value::Mapping(Mapping::from_iter([(
                Value::String("v6.solicitations".to_string()),
                Value::Sequence(Sequence::from_iter([
                    Value::String("a".to_string()),
                    Value::String("b".to_string()),
                ])),
            )])),
        )]));

        let result = access(&value, "inet.v6.solicitations");
        assert_eq!(
            result,
            Some(Value::Sequence(Sequence::from_iter([
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ])))
        );
    }
}
