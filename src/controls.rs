use egui::{Align, Color32, Context, Layout, RichText, Slider};

use crate::{Application, Rt};

impl Application {
    pub fn render_controls(&mut self, ctx: &Context) {
        let (time, itr, _, has_err) = match &self.rt {
            Rt::Runtime(r) => (r.sim_time(), r.num_events_dispatched(), &r.app, false),
            Rt::Finished(r) => (r.time, r.profiler.event_count, &r.app, r.error.is_some()),
        };

        egui::TopBottomPanel::top("controls-panel")
            .exact_height(25.0)
            .show(ctx, |ui| {
                // The top panel is often a good place for a menu bar:

                egui::menu::bar(ui, |ui| {
                    // NOTE: no File->Quit on web pages!

                    ui.horizontal(|ui| {
                        ui.toggle_value(&mut self.show_module_selection, "Modules");
                        ui.toggle_value(&mut self.show_breakpoints, "Breakpoints");
                        ui.toggle_value(&mut self.show_graph, "Graph");
                        ui.toggle_value(&mut self.show_errors, "Errors");
                    });

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

                        let slider = Slider::new(&mut self.param.per_frame_count, 1..=1_000)
                            .show_value(true)
                            .integer()
                            .suffix(" events pre frame")
                            .logarithmic(true);
                        ui.add(slider);

                        ui.label(format!("{:?} | {}", time, itr,));
                        if has_err {
                            if ui
                                .button(RichText::new("Some error has occured").color(Color32::RED))
                                .clicked()
                            {
                                self.show_errors = !self.show_errors;
                            }
                        }
                    })
                });
            });
    }
}
