use std::ops::ControlFlow;

use des::net::ObjectPath;
use egui::{ComboBox, Context, RichText, ScrollArea, SidePanel};
use fxhash::FxHashMap;
use serde_yml::Value;

use crate::{Application, inspector::display_value, plot::access};

#[derive(Debug)]
pub struct Breakpoint {
    pub path: ObjectPath,
    pub key: String,
    pub kind: BreakpointKind,
    pub last: Option<Value>,
    pub triggered: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BreakpointKind {
    Disabled,
    OnValueChanged,
    OnValueAppeared,
    OnValueDisappeared,
}

impl Breakpoint {
    pub fn update(&mut self, observers: &FxHashMap<ObjectPath, Value>) -> ControlFlow<()> {
        self.triggered = false;
        self.update_inner(observers).map_break(|b| {
            self.triggered = true;
            b
        })
    }

    fn update_inner(&mut self, observers: &FxHashMap<ObjectPath, Value>) -> ControlFlow<()> {
        let value = observers
            .get(&self.path)
            .and_then(|value| access(value, &self.key));

        let ret = match self.kind {
            BreakpointKind::Disabled => ControlFlow::Continue(()),
            BreakpointKind::OnValueChanged => (self.last == value)
                .then_some(ControlFlow::Continue(()))
                .unwrap_or(ControlFlow::Break(())),
            BreakpointKind::OnValueAppeared => (self.last.is_none() && value.is_some())
                .then_some(ControlFlow::Break(()))
                .unwrap_or(ControlFlow::Continue(())),
            BreakpointKind::OnValueDisappeared => (self.last.is_some() && value.is_none())
                .then_some(ControlFlow::Break(()))
                .unwrap_or(ControlFlow::Continue(())),
        };
        self.last = value;
        ret
    }
}

impl Application {
    pub fn render_breakpoints(&mut self, ctx: &Context) {
        if self.breakpoints.is_empty() {
            return;
        }

        SidePanel::left("breakpoint-panel").show(ctx, |ui| {
            ui.label(RichText::new("Breakpoints").strong());
            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                for b in &mut self.breakpoints {
                    ui.horizontal(|ui| {
                        let bid = format!("{}", b.path);
                        ui.label(match b.triggered {
                            true => RichText::new(&bid).strong(),
                            false => RichText::new(&bid),
                        });
                        ComboBox::new((&b.path, &b.key), "")
                            .selected_text(format!("{:?}", b.kind))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut b.kind,
                                    BreakpointKind::Disabled,
                                    "Disabled",
                                );
                                ui.selectable_value(
                                    &mut b.kind,
                                    BreakpointKind::OnValueChanged,
                                    "OnValueChange",
                                );
                                ui.selectable_value(
                                    &mut b.kind,
                                    BreakpointKind::OnValueAppeared,
                                    "OnValueAppeared",
                                );
                                ui.selectable_value(
                                    &mut b.kind,
                                    BreakpointKind::OnValueDisappeared,
                                    "OnValueDisappeared",
                                );
                            });

                        // body
                        if let Some(ref last) = b.last {
                            display_value(ui, &b.path, None, b.key.clone(), b.key.clone(), last);
                        } else {
                            ui.label(&b.key);
                        }
                    });
                }
            });
        });
    }
}
