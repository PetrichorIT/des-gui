use std::{borrow::Cow, sync::mpsc::Sender};

use des::net::ObjectPath;

use egui::{
    Button, CollapsingHeader, Color32, Frame, Label, RichText, Sense, TextEdit, TextStyle,
    collapsing_header::CollapsingState,
};
use egui_extras::{Column, TableBuilder};
use fxhash::FxHashMap;
use serde_yml::{Mapping, Value};
use tracing::Level;

use crate::{ActionReq, tracing::GuiTracingObserver};

#[derive(Debug, Clone)]
pub struct ModuleInspector {
    pub path: ObjectPath,
    pub filter: String,
    pub logs: GuiTracingObserver,
    pub remove: bool,
}

impl PartialEq for ModuleInspector {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl ModuleInspector {
    pub const fn new(module: ObjectPath, logs: GuiTracingObserver) -> Self {
        Self {
            path: module,
            filter: String::new(),
            logs,
            remove: false,
        }
    }
}

impl ModuleInspector {
    pub fn show(&mut self, ui: &mut egui::Ui, value: Value, tx: Sender<ActionReq>) {
        Frame::new().show(ui, |ui| {
            TextEdit::singleline(&mut self.filter)
                .background_color(Color32::DARK_GRAY)
                .clip_text(true)
                .hint_text("Search...")
                .show(ui);

            ui.separator();

            display_value_root(
                ui,
                &self.path,
                &tx,
                value.as_mapping().expect("must be a mapping"),
            );

            ui.separator();

            let row_height = ui.text_style_height(&TextStyle::Body);

            let stream = self.logs.streams.lock().unwrap();
            if let Some(events) = stream.get(&self.path) {
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
            mapping.insert(
                Value::String(format!("{group}.{k}").trim_matches('.').to_string()),
                v,
            );
        } else {
            mapping.insert(Value::String(group.to_string()), Value::Mapping(submapping));
        }
    }

    mapping
}

pub fn remove_empty(mapping: &mut Mapping) {
    for k in mapping.keys().cloned().collect::<Vec<_>>() {
        let value = mapping.get_mut(&k).expect("must exist");
        match value {
            Value::Mapping(map) => {
                remove_empty(map);
                if map.is_empty() {
                    mapping.remove(k);
                }
            }
            Value::Sequence(seq) if seq.is_empty() => {
                mapping.remove(k);
            }
            _ => {}
        }
    }
}

pub fn display_value_root(
    ui: &mut egui::Ui,
    module: &ObjectPath,
    tracers: &Sender<ActionReq>,
    mapping: &Mapping,
) {
    for (k, v) in mapping {
        let k = k.as_str().expect("must be a string").to_string();
        display_value(ui, module, Some(tracers), k.clone(), k.clone(), v);
    }
}

pub fn display_value(
    ui: &mut egui::Ui,
    module: &ObjectPath,
    actions: Option<&Sender<ActionReq>>,
    global_key: String,
    key: String,
    value: &Value,
) {
    match value {
        Value::Sequence(seq) => {
            CollapsingHeader::new(&key).show(ui, |ui| {
                for (i, value) in seq.iter().enumerate() {
                    display_value(
                        ui,
                        module,
                        actions,
                        format!("{global_key}.{i}"),
                        i.to_string(),
                        value,
                    );
                    if i != seq.len() - 1 {
                        ui.separator();
                    }
                }
            });
        }
        Value::Mapping(mapping) => {
            ui.horizontal(|ui| {
                let id = ui.make_persistent_id(&global_key);
                ui.vertical(|ui| {
                    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
                    let id_toggle = ui.make_persistent_id((id, "toggle"));

                    let should_toggle: bool =
                        ui.memory_mut(|m| m.data.get_temp(id_toggle).unwrap_or_default());

                    if should_toggle {
                        state.toggle(ui);
                        ui.memory_mut(|m| {
                            let should_toggle = m.data.get_temp_mut_or_default::<bool>(id_toggle);
                            *should_toggle = false;
                        });
                    }

                    state
                        .show_header(ui, |ui| {
                            let resp = ui.vertical(|ui| ui.label(key));
                            let id_interact = ui.make_persistent_id((id, "interact"));
                            if ui
                                .interact(resp.response.rect, id_interact, Sense::click())
                                .clicked()
                            {
                                ui.memory_mut(|m| {
                                    let should_toggle =
                                        m.data.get_temp_mut_or_default::<bool>(id_toggle);
                                    *should_toggle = true;
                                });
                            }

                            if let Some(actions) = actions {
                                let btn = Button::image(egui::Image::new(egui::include_image!(
                                    "../../assets/breakpoint.png"
                                )))
                                .corner_radius(5.0)
                                .frame(false);

                                if ui.add(btn).clicked() {
                                    actions
                                        .send(ActionReq::Breakpoint((
                                            module.clone(),
                                            global_key.trim_matches('.').to_string(),
                                            Some(value.clone()),
                                        )))
                                        .expect("failed to send");
                                }
                            }
                        })
                        .body(|ui| {
                            for (key, value) in mapping {
                                let key = key.as_str().unwrap().to_string();
                                display_value(
                                    ui,
                                    module,
                                    actions,
                                    format!("{global_key}.{key}"),
                                    key,
                                    value,
                                );
                            }
                        });
                });
            });
        }

        Value::Tagged(tagged) => {
            ui.horizontal(|ui| {
                display_value(
                    ui,
                    module,
                    actions,
                    global_key,
                    format!("{key} ({})", tagged.tag.string),
                    &tagged.value,
                )
            });
        }

        _ => {
            ui.horizontal(|ui| {
                ui.label(key);
                value_to_label(ui, value, module, global_key, actions);
            });
        }
    }
}

pub fn value_to_label(
    ui: &mut egui::Ui,
    value: &Value,
    module: &ObjectPath,
    global_key: String,
    actions: Option<&Sender<ActionReq>>,
) {
    ui.horizontal(|ui| {
        match value {
            Value::String(s) => ui.label(s),
            Value::Number(n) => {
                ui.label(n.to_string());
                if let Some(actions) = actions {
                    if ui.button("Observe").clicked() {
                        actions
                            .send(ActionReq::Trace((
                                module.clone(),
                                global_key.trim_matches('.').to_string(),
                            )))
                            .expect("failed to send");
                    }
                }
                ui.response()
            }
            Value::Null => ui.label("null"),
            Value::Bool(b) => ui.label(b.to_string()),
            Value::Sequence(seq) if seq.is_empty() => ui.label("[]"),
            _ => ui.label(format!("??? ({value:?})")),
        };

        if let Some(actions) = actions {
            let btn = Button::image(egui::Image::new(egui::include_image!(
                "../../assets/breakpoint.png"
            )))
            .corner_radius(5.0)
            .frame(false);

            if ui.add(btn).clicked() {
                actions
                    .send(ActionReq::Breakpoint((
                        module.clone(),
                        global_key.trim_matches('.').to_string(),
                        Some(value.clone()),
                    )))
                    .expect("failed to send");
            }
        }
    });
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
