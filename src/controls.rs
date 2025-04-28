use egui::{Align, Color32, ComboBox, Context, Layout, PopupCloseBehavior, Slider};
use serde_yml::Value;

use crate::{Application, Rt, generate_graph, inspector::ModuleInspector, load_props_value};

impl Application {
    pub fn render_controls(&mut self, ctx: &Context) {
        let (time, itr, sim) = match &self.rt {
            Rt::Runtime(r) => (r.sim_time(), r.num_events_dispatched(), &r.app),
            Rt::Finished(sim, time, itr) => (*time, *itr, sim),
        };

        egui::TopBottomPanel::top("controls-panel")
            .exact_height(25.0)
            .show(ctx, |ui| {
                // The top panel is often a good place for a menu bar:

                egui::menu::bar(ui, |ui| {
                    // NOTE: no File->Quit on web pages!

                    ComboBox::new("combo-box-inspector-select", "")
                        .selected_text("Select a module")
                        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            for node_path in sim.nodes() {
                                let node = sim
                                    .globals()
                                    .node(node_path.clone())
                                    .expect("node must exist");

                                if self.modals.iter().any(|n| n.path == node.path()) {
                                    continue;
                                }

                                if ui.button(node_path.as_str()).clicked() {
                                    let value = load_props_value(node);
                                    self.observe
                                        .insert(node_path.clone(), Value::Mapping(value));
                                    self.modals
                                        .push(ModuleInspector::new(node_path, self.logs.clone()));
                                }
                            }
                        });

                    if ui.button("Toggle Graph").clicked() {
                        if self.enable_graph {
                            self.enable_graph = false;
                        } else {
                            generate_graph(sim, self.dir.as_path());
                            self.enable_graph = true;
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if ui
                            .add(egui::Button::new("Stop").fill(Color32::RED))
                            .clicked()
                        {
                            self.param.limit = Some(0);
                        }
                        ui.separator();

                        if ui
                            .add(egui::Button::new("Start").fill(Color32::GREEN))
                            .clicked()
                        {
                            self.param.limit = None;
                        }
                        if ui
                            .add(egui::Button::new("Step").fill(Color32::DARK_GREEN))
                            .clicked()
                        {
                            self.param.limit = Some(1);
                        }

                        let slider = Slider::new(&mut self.param.pre_frame_count, 1..=1_000)
                            .show_value(true)
                            .integer()
                            .suffix(" events pre frame")
                            .logarithmic(true);
                        ui.add(slider);

                        ui.label(format!("{:?} | {}", time, itr,));
                    })
                });
            });
    }
}
