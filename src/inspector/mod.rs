use std::{borrow::Cow, fs::File, io::BufWriter, sync::mpsc::Sender};

use des::net::ObjectPath;

use egui::{
    Button, CollapsingHeader, Color32, Frame, Label, RichText, Sense, TextEdit, TextStyle,
    collapsing_header::CollapsingState,
};
use egui_extras::{Column, TableBuilder};
use fxhash::FxHashMap;
use serde_norway::{Mapping, Value};
use tracing::Level;

use crate::{ActionReq, tracing::GuiTracingObserver};

#[derive(Debug, Clone)]
pub struct ModuleInspector {
    pub path: ObjectPath,
    pub filter: String,
    pub highlight: Option<String>,
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
            highlight: None,
            remove: false,
        }
    }
}

impl ModuleInspector {
    pub fn show(&mut self, ui: &mut egui::Ui, value: Value, tx: Sender<ActionReq>) {
        Frame::new().show(ui, |ui| {
            ui.horizontal(|ui| {
                TextEdit::singleline(&mut self.filter)
                    .background_color(Color32::from_black_alpha(0))
                    .clip_text(true)
                    .hint_text("Search...")
                    .show(ui);

                if ui.button("Export").clicked() {
                    // Export logic
                    let lock = self.logs.streams.lock().unwrap();
                    let events = lock
                        .get(&self.path)
                        .unwrap()
                        .output()
                        .into_iter()
                        .collect::<Vec<_>>();
                    // Export events to file or clipboard
                    let f = File::create(format!("{}.logs.yaml", self.path)).unwrap();
                    let f = BufWriter::new(f);
                    serde_norway::to_writer(f, &events).unwrap();
                }
            });

            ui.separator();

            // println!("{value:?}");
            ui.horizontal(|ui| {
                display(
                    ui,
                    Ctx {
                        node: &self.path,
                        actions: Some(&tx),
                    },
                    &value,
                    String::new(),
                );
            });

            ui.separator();

            let row_height = ui.text_style_height(&TextStyle::Body);

            let stream = self.logs.streams.lock().unwrap();
            if let Some(log) = stream.get(&self.path) {
                let matching_events = log
                    .output()
                    .into_iter()
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
                                let target = RichText::new(event.metadata.target())
                                    .text_style(TextStyle::Monospace)
                                    .italics();
                                if Some(event.metadata.target())
                                    == self.highlight.as_ref().map(String::as_str)
                                {
                                    let label = ui.add(
                                        Label::new(target.background_color(Color32::YELLOW))
                                            .extend(),
                                    );

                                    if label.double_clicked() {
                                        self.filter = self.highlight.clone().unwrap();
                                    } else if label.clicked() {
                                        self.highlight = None;
                                    }
                                } else {
                                    if ui.add(Label::new(target).extend()).clicked() {
                                        self.highlight = Some(event.metadata.target().to_string());
                                    }
                                };
                            });
                            row.col(|ui| {
                                let span =
                                    RichText::new(&event.span).text_style(TextStyle::Monospace);
                                if Some(&event.span) == self.highlight.as_ref() {
                                    let label = ui.label(span.background_color(Color32::YELLOW));

                                    if label.double_clicked() {
                                        self.filter = self.highlight.clone().unwrap();
                                    } else if label.clicked() {
                                        self.highlight = None;
                                    }
                                } else {
                                    if ui.label(span).clicked() {
                                        self.highlight = Some(event.span.clone());
                                    }
                                };
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
            let (k, v) = submapping.into_iter().next().expect("must exist");
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

/// For recursive variants (Seq, Map) there are two layout options:
///
/// A) key: value / index: value
/// OR
/// B) key <collaps value> / index: <collapse value>
///
/// for non-recursive subtypes use A, assume all entries have the same layout
enum LayoutConstraint {
    Shallow,
    Deep,
}

fn determine_layout_constraints(value: &Value) -> LayoutConstraint {
    match value {
        Value::Sequence(_) | Value::Mapping(_) | Value::Tagged(_) => LayoutConstraint::Deep,
        _ => LayoutConstraint::Shallow,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    pub node: &'a ObjectPath,
    pub actions: Option<&'a Sender<ActionReq>>,
}

pub fn display(ui: &mut egui::Ui, ctx: Ctx, value: &Value, key: String) {
    match value {
        Value::Mapping(map) if map.is_empty() => {
            ui.label("[:]");
        }
        Value::Mapping(map) => {
            ui.vertical(|ui| {
                for (k, v) in map {
                    let layout = determine_layout_constraints(v);
                    let k = k.as_str().unwrap();

                    match layout {
                        LayoutConstraint::Shallow => {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}:", k));
                                display(ui, ctx, &v, format!("{key}.{k}"));
                            });
                        }
                        LayoutConstraint::Deep => {
                            let id = ui.make_persistent_id((&key, k));
                            let mut state =
                                CollapsingState::load_with_default_open(&ui.ctx(), id, false);

                            let id_toggle = ui.make_persistent_id((id, "toggle"));
                            let should_toggle: bool =
                                ui.memory_mut(|m| m.data.get_temp(id_toggle).unwrap_or_default());
                            if should_toggle {
                                state.toggle(ui);
                                ui.memory_mut(|m| {
                                    let should_toggle =
                                        m.data.get_temp_mut_or_default::<bool>(id_toggle);
                                    *should_toggle = false;
                                });
                            }

                            state
                                .show_header(ui, |ui| {
                                    let resp = ui.vertical(|ui| ui.label(k));
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

                                    if let Some(actions) = ctx.actions {
                                        let btn = Button::image(egui::Image::new(
                                            egui::include_image!("../../assets/breakpoint.png"),
                                        ))
                                        .corner_radius(5.0)
                                        .frame(false);

                                        if ui.add(btn).clicked() {
                                            actions
                                                .send(ActionReq::Breakpoint((
                                                    ctx.node.clone(),
                                                    format!("{key}.{k}")
                                                        .trim_matches('.')
                                                        .to_string(),
                                                    Some(value.clone()),
                                                )))
                                                .expect("failed to send");
                                        }
                                    }
                                })
                                .body(|ui| {
                                    display(ui, ctx, v, format!("{key}.{k}"));
                                });
                        }
                    }
                }
            });

            return;
        }

        Value::Sequence(seq) if seq.is_empty() => {
            ui.label("[]");
        }
        Value::Sequence(seq) => {
            ui.vertical(|ui| {
                for (i, v) in seq.iter().enumerate() {
                    display(ui, ctx, &v, format!("{key}.{i}"));
                    if i != seq.len() - 1 {
                        ui.separator();
                    }
                }
            });
            return;
        }

        Value::Tagged(tagged) => {
            ui.horizontal(|ui| {
                CollapsingHeader::new(tagged.tag.to_string().trim_start_matches('!'))
                    .default_open(true)
                    .show(ui, |ui| display(ui, ctx, &tagged.value, key.clone()))
            });
            return;
        }

        Value::String(s) => {
            ui.label(s);
        }
        Value::Number(n) => {
            ui.label(n.to_string());
            if let Some(actions) = ctx.actions {
                if ui.button("Observe").clicked() {
                    actions
                        .send(ActionReq::Trace((
                            ctx.node.clone(),
                            key.trim_matches('.').to_string(),
                        )))
                        .expect("failed to send");
                }
            }
        }
        Value::Null => {
            ui.label("null");
        }
        Value::Bool(b) => {
            ui.label(b.to_string());
        }
    }

    if let Some(actions) = ctx.actions {
        let btn = Button::image(egui::Image::new(egui::include_image!(
            "../../assets/breakpoint.png"
        )))
        .corner_radius(5.0)
        .frame(false);

        if ui.add(btn).clicked() {
            actions
                .send(ActionReq::Breakpoint((
                    ctx.node.clone(),
                    key.trim_matches('.').to_string(),
                    Some(value.clone()),
                )))
                .expect("failed to send");
        }
    }
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
