use breakpoint::{Breakpoint, BreakpointKind};
use des::{prelude::*, runtime::RuntimeResult, tracing::FALLBACK_LOG_LEVEL};
use egui::{
    CentralPanel, CollapsingHeader, Id, Image, RichText, ScrollArea, SidePanel, ViewportBuilder,
};
use fxhash::FxHashMap;
use petgraph::dot::{Config, Dot};
use plot::{Tracer, TreeTracer};
use serde_norway::{Mapping, Value};
use std::{
    borrow::Cow,
    env::{self, temp_dir, var},
    fs::{self, File},
    io::Write,
    mem::{self, forget},
    ops::{ControlFlow, Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};
use tracing_error::ErrorLayer;
use tracing_subscriber::{EnvFilter, filter::Directive, fmt::Layer, layer::SubscriberExt};

pub mod sim;
pub mod tracing;

mod breakpoint;
mod controls;
mod inspector;
mod plot;

use inspector::{ModuleInspector, remove_empty, unify};
use tracing::GuiTracingObserver;

pub fn launch_with_gui(f: impl FnOnce() -> Runtime<Sim<()>>) -> eframe::Result {
    let mut native_options = eframe::NativeOptions::default();
    native_options.viewport.maximized = Some(true);

    let supress = var("DES_NOGUI").is_ok_and(|v| v == "1");
    if supress {
        let _ = f().run().assert_no_err();
        return Ok(());
    }

    eframe::run_native(
        "des-gui",
        native_options,
        Box::new(|cc| Ok(Box::new(Application::new(cc, f)))),
    )
}

pub enum ActionReq {
    Breakpoint(BreakpointReq),
    Trace(TreeTraceReq),
}

pub type TreeTraceReq = (ObjectPath, String);
pub type BreakpointReq = (ObjectPath, String, Option<Value>);

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
pub struct Application {
    // Example stuff:
    logs: GuiTracingObserver,
    last_frame: Instant,

    rt: Rt,
    param: ExecutionParameters,

    dir: PathBuf,

    // Value observers
    observe: Observer,
    breakpoints: Vec<Breakpoint>,

    // presenters
    modals: Vec<ModuleInspector>,
    traces: Vec<Vec<Box<dyn Tracer>>>,

    // helpers
    tx_rx: (Sender<ActionReq>, Receiver<ActionReq>),

    frame_time: Duration,

    show_module_selection: bool,
    show_breakpoints: bool,
    show_graph: bool,
    show_errors: bool,
}

#[derive(Debug, Default)]
struct Observer {
    map: FxHashMap<ObjectPath, Value>,
}

impl Observer {
    fn update(&mut self, sim: &Sim<()>) {
        for (path, value) in &mut self.map {
            let Some(module) = sim.globals().get(&path) else {
                continue;
            };

            let map = load_props_value(module);
            *value = Value::Mapping(map);
        }
    }
}

impl Deref for Observer {
    type Target = FxHashMap<ObjectPath, Value>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl DerefMut for Observer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

enum Rt {
    Runtime(Runtime<Sim<()>>),
    Finished(RuntimeResult<Sim<()>>),
}

impl Rt {
    fn sim(&self) -> &Sim<()> {
        match self {
            Self::Runtime(rt) => &rt.app,
            Self::Finished(res) => &res.app,
        }
    }

    fn finish(&mut self) -> Result<(), des::net::Error> {
        match self {
            Self::Runtime(rt) => {
                unsafe {
                    let runtime = std::ptr::read(rt);
                    let replacing = runtime.finish();

                    if let Some(err) = &replacing.error {
                        println!("{err}");
                    }

                    let replacing = Rt::Finished(replacing);
                    let zeroed = mem::replace(self, replacing);
                    forget(zeroed);
                };
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct ExecutionParameters {
    limit: Option<usize>,
    per_frame_count: usize,
    per_event_time: Duration,
}

impl Application {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>, f: impl FnOnce() -> Runtime<Sim<()>>) -> Self {
        if env::var("RUST_LOG").is_err() {
            unsafe {
                env::set_var("RUST_LOG", "winit=warn,trace");
            }
        }

        let gui_capture = GuiTracingObserver::default();
        let stdout = std::io::stdout;
        let subscriber = tracing_subscriber::Registry::default()
            .with(
                EnvFilter::builder()
                    .with_default_directive(Directive::from(FALLBACK_LOG_LEVEL))
                    .from_env_lossy(),
            )
            .with(ErrorLayer::default())
            .with(
                Layer::default()
                    .with_ansi(false)
                    .event_format(gui_capture.clone()),
            )
            .with(
                Layer::default()
                    .with_writer(stdout)
                    .with_ansi(true)
                    .event_format(des::tracing::format()),
            );

        ::tracing::subscriber::set_global_default(subscriber).unwrap();

        egui_extras::install_image_loaders(&cc.egui_ctx);

        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.

        let runtime = f();

        Self {
            last_frame: Instant::now(),

            param: ExecutionParameters {
                limit: Some(0),
                per_frame_count: 0,
                per_event_time: Duration::ZERO,
            },
            rt: Rt::Runtime(runtime),
            logs: gui_capture,

            dir: temp_dir(),

            observe: Observer::default(),
            breakpoints: Vec::new(),

            // graph: generate_graph(topo),
            modals: Vec::new(),
            traces: vec![Vec::new()],

            tx_rx: channel(),

            frame_time: Duration::ZERO,

            show_module_selection: true,
            show_breakpoints: false,
            show_graph: false,
            show_errors: false,
        }
    }

    fn run_sim_step(&mut self, ctx: &egui::Context) -> ControlFlow<()> {
        // setup tracers
        while let Ok(req) = self.tx_rx.1.try_recv() {
            match req {
                ActionReq::Breakpoint(req) => {
                    self.show_breakpoints = true;
                    if let Some(i) = self
                        .breakpoints
                        .iter()
                        .position(|b| b.path == req.0 && b.key == req.1)
                    {
                        self.breakpoints.remove(i);
                    } else {
                        self.breakpoints.push(Breakpoint {
                            path: req.0,
                            key: req.1,
                            kind: BreakpointKind::OnValueChanged,
                            last: req.2,
                            triggered: false,
                            remove: false,
                        });
                    }
                }
                ActionReq::Trace(req) => {
                    self.traces[0].push(Box::new(TreeTracer::new(req.0, req.1)));
                }
            }
        }

        if let Rt::Runtime(ref mut runtime) = self.rt {
            if runtime.was_started()
                && (runtime.has_reached_limit() || runtime.num_events_remaining() == 0)
            {
                self.rt.finish().expect("failed");
                ctx.request_repaint();
                // TODO update observers
                return ControlFlow::Break(());
            }

            let can_progress = (self.param.limit.map_or(true, |v| v > 0)
                && runtime.num_events_remaining() > 0)
                || !runtime.was_started();
            if can_progress {
                if self.param.per_frame_count >= 1_000
                    && !self.frame_time.is_zero()
                    && !self.param.per_event_time.is_zero()
                {
                    // STEPS MAX
                    const FRAME_MAX: Duration = Duration::from_millis(33);
                    let remaining = FRAME_MAX.saturating_sub(self.frame_time).as_secs_f64();
                    let count = remaining / self.param.per_event_time.as_secs_f64() / 1.5;
                    self.param.per_frame_count = (count as usize).max(1_000);
                }

                let steps = self.param.per_frame_count;

                if !runtime.was_started() {
                    runtime.start().expect("failed to start");
                }

                let t0 = Instant::now();
                'outer: for _ in 0..steps {
                    runtime
                        .dispatch_n_events(1)
                        .expect("failed to dispatch events");

                    self.observe.update(&runtime.app);

                    for b in &mut self.breakpoints {
                        if let ControlFlow::Break(()) = b.update(&self.observe) {
                            self.param.limit = Some(0);
                            break 'outer;
                        }
                    }
                }

                if steps > 0 {
                    self.param.per_event_time = t0.elapsed() / steps as u32;
                }
                // Update not per event but per frame: TODO is that a good idea?
                self.traces
                    .iter_mut()
                    .for_each(|t| t.iter_mut().for_each(|trace| trace.update(&self.observe)));

                if let Some(ref mut limit) = self.param.limit {
                    *limit = limit.saturating_sub(steps);
                }
            }
        };
        ControlFlow::Continue(())
    }
}

fn load_props_value(module: ModuleRef) -> Mapping {
    let props = module.props_keys();
    let props_with_values = props
        .iter()
        .filter_map(|key| {
            let prop = module.prop_raw(&key);
            if let Some(value) = prop.as_value() {
                Some((&key[..], Cow::<Value>::Owned(value)))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut map = unify(&props_with_values);
    remove_empty(&mut map);
    map
}

impl eframe::App for Application {
    /// Called each time the UI needs repainting, which may be many times per second.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t0 = Instant::now();

        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        if let ControlFlow::Break(_) = self.run_sim_step(ctx) {
            return;
        }

        self.render_controls(ctx);

        self.modals.retain(|v| !v.remove);
        self.breakpoints.retain(|v| !v.remove);

        for modal in &mut self.modals {
            ctx.show_viewport_immediate(
                egui::ViewportId(Id::new(format!("panel-{}", modal.path))),
                ViewportBuilder::default()
                    .with_title(modal.path.to_string())
                    .with_inner_size([800.0, 1200.0]),
                |ctx, _| {
                    let tx = self.tx_rx.0.clone();
                    CentralPanel::default().show(ctx, |ui| {
                        modal.show(
                            ui,
                            self.observe
                                .get(&modal.path)
                                .expect("must be observerd")
                                .clone(),
                            tx,
                        )
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        // Tell parent to close us.
                        modal.remove = true;
                    }
                },
            );
        }

        if self.traces.iter().map(Vec::len).sum::<usize>() > 0 {
            self.show_plot(ctx);
        }

        if self.show_module_selection {
            SidePanel::left("module-selection").show(ctx, |ui| {
                let sim = match &self.rt {
                    Rt::Runtime(r) => &r.app,
                    Rt::Finished(r) => &r.app,
                };

                ui.label(RichText::new("Breakpoints").strong());
                ui.separator();

                ScrollArea::vertical().show(ui, |ui| {
                    for node_path in sim.nodes() {
                        ui.scope(|ui| {
                            let node = sim.globals().get(&node_path).expect("node must exist");
                            let exists = self.modals.iter().any(|n| n.path == node.path());

                            if exists {
                                ui.disable();
                            }
                            if ui.button(node_path.as_str()).clicked() {
                                let value = load_props_value(node);
                                self.observe
                                    .insert(node_path.clone(), Value::Mapping(value));
                                self.modals
                                    .push(ModuleInspector::new(node_path, self.logs.clone()));
                            }
                        });
                    }
                });
            });
        }

        if self.show_breakpoints {
            self.render_breakpoints(ctx);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.show_errors
                && let Rt::Finished(r) = &self.rt
                && let Some(e) = r.error.as_ref()
            {
                for (i, part) in e.into_iter().enumerate() {
                    let lines = part.repr.to_string();
                    let first = lines.lines().next().unwrap();
                    CollapsingHeader::new(format!("{i}: {}", first))
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.label(lines.trim_start_matches(first).trim_start_matches('\n'))
                        });
                }
            }

            if self.show_graph {
                let path = format!("{}topo.png", self.dir.as_path().display());
                if !fs::exists(&path).unwrap() {
                    generate_graph(self.rt.sim(), &self.dir);
                }

                ui.add(Image::new(format!("file://{path}")).shrink_to_fit());
            }

            ui.label(format!("{:?}", self.frame_time))
        });

        // Remove observers if no longer needed
        for k in self.observe.keys().cloned().collect::<Vec<_>>() {
            let needed = self.modals.iter().any(|m| m.path == k)
                || self.traces.iter().flatten().any(|v| v.needs_path(&k))
                || self.breakpoints.iter().any(|b| b.path == k);
            if !needed {
                self.observe.remove(&k);
                ::tracing::info!("Removed observer for path: {}", k);
            }
        }

        if matches!(self.rt, Rt::Runtime(_)) {
            let frame_time = Duration::from_secs(1) / 30;
            let next_frame = self.last_frame + frame_time;
            let now = Instant::now();
            let wait_time = next_frame.max(now).duration_since(now);

            ctx.request_repaint_after(wait_time);
        }

        self.frame_time = t0.elapsed();
    }
}

fn generate_graph(sim: &Sim<()>, dir: &Path) {
    let topo = sim.topology();

    let mut child = Command::new("dot")
        .arg("-Tpng")
        .arg("-Gdpi=300")
        .arg("-Gfontcolor=white")
        .arg("-Gcolor=white")
        .arg("-Nfontcolor=white")
        .arg("-Ncolor=white")
        .arg("-Efontcolor=white")
        .arg("-Ecolor=white")
        .arg("-Gbgcolor=black")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("dot failed");

    let graph = topo.map(
        |_, node| node.path().to_string(),
        |_, edge| format!("{}*{}", edge.source.name(), edge.target.name()),
    );
    let dot = Dot::with_attr_getters(
        &graph,
        &[Config::NodeNoLabel, Config::EdgeNoLabel],
        &|_, edge| {
            let (l, r) = edge.weight().split_once("*").unwrap();
            format!("headlabel={r:?} taillabel={l:?}")
        },
        &|_, node| format!("label={:?} shape=box", node.1),
    );

    println!("{dot}");

    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin
        .write_all(format!("{dot}").as_bytes())
        .expect("write failed");
    drop(stdin);

    let output = child.wait_with_output().expect("wait failed");
    File::create(format!("{}topo.png", dir.display()))
        .unwrap()
        .write_all(&output.stdout)
        .unwrap();

    ::tracing::info!("wrote topo to {}", format!("{}topo.png", dir.display()));
}
