use ::tracing::info;
use des::{prelude::*, tracing::FALLBACK_LOG_LEVEL};
use egui::{Align, Color32, ComboBox, Direction, Layout, PopupCloseBehavior, SidePanel};
use egui_graphs::{Graph, SettingsInteraction, SettingsNavigation, SettingsStyle};
use petgraph::{Undirected, prelude::StableUnGraph};
use plot::Tracer;
use std::{
    collections::HashMap,
    mem::{self, MaybeUninit, forget},
    time::{Duration, Instant},
};
use tracing_subscriber::{EnvFilter, filter::Directive, util::SubscriberInitExt};

pub mod sim;
pub mod tracing;

mod inspector;
mod plot;

use inspector::ModuleInspector;
use tracing::{MachineReadableFormat, MakeTracer};

pub fn launch_with_gui(f: impl FnOnce() -> Runtime<Sim<()>>) -> eframe::Result {
    let mut native_options = eframe::NativeOptions::default();
    native_options.viewport.maximized = Some(true);

    eframe::run_native(
        "des-gui",
        native_options,
        Box::new(|cc| Ok(Box::new(Application::new(cc, f)))),
    )
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
pub struct Application {
    // Example stuff:
    logs: MakeTracer,
    last_frame: Instant,

    rt: Rt,
    param: ExecutionParameters,

    modals: Vec<ModuleInspector>,
    traces: Vec<Vec<Box<dyn Tracer>>>,

    graph: Graph<(), (), Undirected>,
}

enum Rt {
    Runtime(Runtime<Sim<()>>),
    Finished(Sim<()>, SimTime, usize),
}

impl Rt {
    fn finish(&mut self) -> Result<(), RuntimeError> {
        match self {
            Self::Runtime(rt) => {
                unsafe {
                    let runtime = std::ptr::read(rt);
                    let replacing = runtime
                        .finish()
                        .map(|(s, t, p)| Rt::Finished(s, t, p.event_count))?;

                    let zeroed = mem::replace(self, replacing);
                    forget(zeroed);
                };
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(serde::Deserialize, serde::Serialize, Default, Debug)]
pub struct ExecutionParameters {
    limit: Option<usize>,
    pre_frame_limit: Speed,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(usize)]
enum Speed {
    #[default]
    Slow = 1,
    Medium = 10,
    Fast = 1000,
}

impl Application {
    /// Called once before the first frame.
    pub fn new(_: &eframe::CreationContext<'_>, f: impl FnOnce() -> Runtime<Sim<()>>) -> Self {
        let subscriber = tracing_subscriber::fmt().with_ansi(false);
        let subscriber = subscriber.event_format(MachineReadableFormat);
        let subscriber = subscriber.with_env_filter(
            EnvFilter::builder()
                .with_default_directive(Directive::from(FALLBACK_LOG_LEVEL))
                .from_env_lossy(),
        );
        let buf = MakeTracer::new();
        let subscriber = subscriber.with_writer(buf.clone());
        subscriber.finish().init();

        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.

        let runtime = f();
        let topo = runtime.app.topology();

        Self {
            last_frame: Instant::now(),

            param: ExecutionParameters {
                limit: Some(0),
                pre_frame_limit: Speed::Slow,
            },
            rt: Rt::Runtime(runtime),
            logs: buf,

            graph: generate_graph(topo),
            modals: Vec::new(),
            traces: vec![Vec::new()],
        }
    }

    fn update_top_bar(&mut self, ctx: &egui::Context) {
        let (time, itr, sim) = match &mut self.rt {
            Rt::Runtime(r) => (r.sim_time(), r.num_events_dispatched(), &mut r.app),
            Rt::Finished(sim, time, itr) => (*time, *itr, sim),
        };

        egui::TopBottomPanel::top("top_panel")
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

                                if self.modals.iter().any(|n| n.module == node) {
                                    continue;
                                }

                                if ui.button(node_path.as_str()).clicked() {
                                    self.modals
                                        .push(ModuleInspector::new(node, self.logs.clone()));
                                }
                            }
                        });

                    ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {
                        ui.label(format!("{:?} | {}", time, itr,));
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

                        ComboBox::from_label("Execution speed")
                            .selected_text(format!("{:?}", self.param.pre_frame_limit))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.param.pre_frame_limit,
                                    Speed::Slow,
                                    "Slow",
                                );
                                ui.selectable_value(
                                    &mut self.param.pre_frame_limit,
                                    Speed::Medium,
                                    "Medium",
                                );
                                ui.selectable_value(
                                    &mut self.param.pre_frame_limit,
                                    Speed::Fast,
                                    "Fast",
                                );
                            });
                    })
                });
            });
    }
}

impl eframe::App for Application {
    /// Called each time the UI needs repainting, which may be many times per second.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        if let Rt::Runtime(ref mut runtime) = self.rt {
            if runtime.was_started()
                && (runtime.has_reached_limit() || runtime.num_events_remaining() == 0)
            {
                self.rt.finish().expect("failed");
                ctx.request_repaint();
                return;
            }

            let can_progress = (self.param.limit.map_or(true, |v| v > 0)
                && runtime.num_events_remaining() > 0)
                || !runtime.was_started();
            if can_progress {
                let steps = self.param.pre_frame_limit as usize;
                if !runtime.was_started() {
                    runtime.start();
                }

                info!("executing {steps} events");
                runtime.dispatch_n_events(steps);

                self.traces
                    .iter_mut()
                    .for_each(|t| t.iter_mut().for_each(|trace| trace.update()));

                if let Some(ref mut limit) = self.param.limit {
                    *limit = limit.saturating_sub(steps);
                    info!("remaining {} events after - {steps}", *limit);
                }
            }
        };

        self.update_top_bar(ctx);

        self.modals.retain(|v| !v.remove);
        for modal in &mut self.modals {
            SidePanel::new(
                egui::panel::Side::Left,
                format!("panel-{}", modal.module.path()),
            )
            .show(ctx, |ui| modal.show(ui, &mut self.traces));
        }

        if self.traces.iter().map(Vec::len).sum::<usize>() > 0 {
            self.show_plot(ctx);
        }

        let interaction_settings = &SettingsInteraction::new().with_dragging_enabled(true);
        // .with_node_clicking_enabled(true)
        // .with_node_selection_enabled(true)
        // .with_node_selection_multi_enabled(true)
        // .with_edge_clicking_enabled(true)
        // .with_edge_selection_enabled(true)
        // .with_edge_selection_multi_enabled(true);
        let style_settings = &SettingsStyle::new().with_labels_always(true);
        let navigation_settings = &SettingsNavigation::new()
            .with_fit_to_screen_enabled(true)
            .with_zoom_and_pan_enabled(false);

        egui::CentralPanel::default().show(ctx, |ui| {
            // ui.add(
            //     &mut GraphView::<_, _, _, _, _, _, LayoutStateRandom, LayoutRandom>::new(
            //         &mut self.graph,
            //     )
            //     .with_styles(style_settings)
            //     .with_interactions(interaction_settings)
            //     .with_navigations(navigation_settings),
            // );
        });

        let frame_time = Duration::from_secs(1) / 30;
        let next_frame = self.last_frame + frame_time;
        let now = Instant::now();
        let wait_time = next_frame.max(now).duration_since(now);

        ctx.request_repaint_after(wait_time);
    }
}

fn generate_graph(topo: Topology<(), ()>) -> Graph<(), (), Undirected> {
    let mut graph = Graph::from(&StableUnGraph::default());
    let mut mapping = HashMap::new();

    for node in topo.nodes() {
        let idx = graph.add_node_custom((), |gnode| {
            gnode.set_label(node.module().path().to_string());
            gnode.set_color(Color32::LIGHT_BLUE);
        });
        mapping.insert(node.module().path(), idx);
    }

    for edge in topo.edges() {
        let from = *mapping.get(&edge.from.module().path()).unwrap();
        let to = *mapping.get(&edge.to.module().path()).unwrap();

        if !graph.g.contains_edge(from, to) {
            graph.add_edge_with_label(
                from,
                to,
                (),
                format!("{} - {}", edge.from.gate().str(), edge.to.gate().str()),
            );
        }
    }

    graph
}
