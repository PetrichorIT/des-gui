use std::borrow::Cow;

use des::{net::module::RawProp, prelude::ModuleRef};

use egui::{CollapsingHeader, Color32, Frame, Label, RichText, TextEdit, TextStyle};
use egui_extras::{Column, TableBuilder};
use fxhash::FxHashMap;
use serde_yml::{Mapping, Value};
use tracing::Level;

use crate::{
    plot::{PropTracer, Tracer},
    tracing::GuiTracingObserver,
};

mod props;
pub use props::*;

#[derive(Debug, Clone)]
pub struct ModuleInspector {
    pub module: ModuleRef,
    pub filter: String,
    pub logs: GuiTracingObserver,
    pub remove: bool,
}

impl PartialEq for ModuleInspector {
    fn eq(&self, other: &Self) -> bool {
        self.module == other.module
    }
}

impl ModuleInspector {
    pub const fn new(module: ModuleRef, logs: GuiTracingObserver) -> Self {
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
            TextEdit::singleline(&mut self.filter)
                .background_color(Color32::DARK_GRAY)
                .clip_text(true)
                .hint_text("Search...")
                .show(ui);

            ui.separator();

            let props = self.module.props();
            let props_with_values = props
                .iter()
                .filter_map(|key| {
                    let prop = self.module.prop_raw(&key);
                    if let Some(value) = prop.into_value() {
                        Some((&key[..], Cow::<Value>::Owned(value)))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            let map = unify(&props_with_values);

            let props = self
                .module
                .props()
                .into_iter()
                .filter_map(|key| {
                    let prop = self.module.prop_raw(&key);
                    if let Some(_) = prop.into_value() {
                        Some((key, prop))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            value_to_collapsable(
                ui,
                &mut tracers[0],
                String::new(),
                String::new(),
                &Value::Mapping(map),
                &props,
            );

            ui.separator();

            let row_height = ui.text_style_height(&TextStyle::Body);

            let stream = self.logs.streams.lock().unwrap();
            if let Some(events) = stream.get(&self.module.path()) {
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
                                    RichText::new(event.time.to_string())
                                        .color(color_for_log(*event.metadata.level())),
                                );
                            });
                            row.col(|ui| {
                                ui.add(
                                    Label::new(
                                        RichText::new(event.metadata.target())
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

pub fn unify(props: &[(&str, Cow<Value>)]) -> Mapping {
    if props.len() == 1 {
        return Mapping::from_iter([(
            Value::String(props[0].0.to_string()),
            props[0].1.clone().into_owned(),
        )]);
    }

    let mut groups = FxHashMap::<&str, Vec<(&str, Cow<Value>)>>::default();
    let mut mapping = Mapping::default();
    for prop in props {
        let (selectable, rem) = prop.0.split_once('@').unwrap_or((prop.0, ""));
        if selectable.is_empty() {
            mapping.insert(Value::String(rem.to_string()), prop.1.clone().into_owned());
            continue;
        }

        let (group, split_rem) = selectable.split_once('.').unwrap_or((&selectable, ""));
        let now_rem =
            &prop.0[(group.len() + 1 - split_rem.is_empty() as usize).min(prop.0.len())..];
        groups
            .entry(group)
            .or_default()
            .push((now_rem, prop.1.clone()))
    }

    for (group, members) in groups {
        let submapping = unify(&members);
        if submapping.len() == 1 {
            let (k, v) = submapping.map.into_iter().next().expect("must exist");
            let k = k.as_str().expect("must be a string");
            mapping.insert(Value::String(format!("{group}.{k}")), v);
        } else {
            mapping.insert(Value::String(group.to_string()), Value::Mapping(submapping));
        }
    }

    mapping
}

fn simplify(mapping: &mut Mapping) {
    let keys = mapping
        .keys()
        .map(|k| {
            k.as_str()
                .unwrap()
                .split_once('.')
                .unwrap_or((k.as_str().unwrap(), ""))
        })
        .collect::<Vec<_>>();

    let mut map = FxHashMap::<String, Vec<String>>::default();
    for (mtch, rem) in &keys {
        map.entry(mtch.to_string())
            .or_default()
            .push(rem.to_string());
    }

    for (group, members) in map {
        if members.len() == 1 {
            continue;
        }

        let mut submapping = Mapping::new();
        for member in members {
            if member.is_empty() {
                let val = mapping.remove(&group).unwrap();
                let map = val.as_mapping().expect("must be a map");
                for (k, v) in map {
                    submapping.insert(k.clone(), v.clone());
                }
            } else {
                submapping.insert(
                    Value::String(member.to_string()),
                    mapping
                        .remove(Value::String(format!("{group}.{member}")))
                        .unwrap(),
                );
            }
        }

        simplify(&mut submapping);

        if submapping.len() == 1 {
            let key = submapping
                .keys()
                .next()
                .unwrap()
                .clone()
                .as_str()
                .unwrap()
                .to_string();
            let value = submapping.remove(&key).unwrap();
            mapping.insert(Value::String(format!("{group}.{key}")), value);
        } else {
            mapping.insert(Value::String(group), Value::Mapping(submapping));
        }
    }
}

fn value_to_collapsable(
    ui: &mut egui::Ui,
    tracers: &mut Vec<Box<dyn Tracer>>,
    global_key: String,
    key: String,
    value: &Value,
    props: &[(String, RawProp)],
) {
    match value {
        Value::Sequence(seq) => {
            CollapsingHeader::new(&key).show(ui, |ui| {
                for (i, value) in seq.iter().enumerate() {
                    // TODO invalidate global key
                    value_to_collapsable(
                        ui,
                        tracers,
                        format!("{global_key}.{i}"),
                        String::new(),
                        value,
                        props,
                    );
                }
            });
        }
        Value::Mapping(mapping) => {
            if !key.is_empty() {
                CollapsingHeader::new(key)
                    .id_salt(&global_key)
                    .show(ui, |ui| {
                        for (key, value) in mapping {
                            let key = key.as_str().unwrap().to_string();
                            value_to_collapsable(
                                ui,
                                tracers,
                                format!("{global_key}.{key}"),
                                key,
                                value,
                                props,
                            );
                        }
                    });
            } else {
                for (key, value) in mapping {
                    let key = key.as_str().unwrap().to_string();
                    value_to_collapsable(
                        ui,
                        tracers,
                        format!("{global_key}.{key}"),
                        key,
                        value,
                        props,
                    );
                }
            }
        }

        Value::Tagged(tagged) => {
            ui.horizontal(|ui| {
                ui.label(key);
                CollapsingHeader::new(&tagged.tag.string)
                    .id_salt(&global_key)
                    .show(ui, |ui| {
                        value_to_collapsable(
                            ui,
                            tracers,
                            global_key,
                            String::new(),
                            &tagged.value,
                            props,
                        )
                    });
            });
        }

        _ => {
            ui.horizontal(|ui| {
                ui.label(key);
                value_to_label(ui, value);
                if let Some((prop_key, prop)) = props
                    .iter()
                    .find(|v| v.0 == global_key.trim_start_matches('.'))
                {
                    if ui.button("Observe").clicked() {
                        tracers.push(Box::new(PropTracer::new(prop_key.clone(), prop.clone())));
                    }
                }
            });
        }
    }
}

fn value_to_label(ui: &mut egui::Ui, value: &Value) {
    match value {
        Value::String(s) => ui.label(s),
        Value::Number(n) => ui.label(n.to_string()),
        Value::Null => ui.label("null"),
        Value::Bool(b) => ui.label(b.to_string()),
        Value::Sequence(seq) if seq.is_empty() => ui.label("[]"),
        _ => ui.label(format!("??? ({value:?})")),
    };
}

fn color_for_log(level: Level) -> Color32 {
    match level {
        Level::TRACE => Color32::from_rgb(0, 128, 0),
        Level::DEBUG => Color32::from_rgb(0, 0, 255),
        Level::INFO => Color32::from_rgb(0, 255, 0),
        Level::WARN => Color32::from_rgb(255, 255, 0),
        Level::ERROR => Color32::from_rgb(255, 0, 0),
    }
}
