use breakpoint::{Breakpoint, BreakpointKind};
use des::{prelude::*, tracing::FALLBACK_LOG_LEVEL};
use egui::{CentralPanel, Id, Image, ViewportBuilder};
use fxhash::FxHashMap;
use plot::{Tracer, TreeTracer};
use serde_yml::{Mapping, Value};
use std::{
    borrow::Cow,
    env::temp_dir,
    fs::File,
    io::Write,
    mem::{self, forget},
    ops::{ControlFlow, Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};
use tracing_subscriber::{EnvFilter, filter::Directive, fmt::Layer, layer::SubscriberExt};
use valuable::ValueOwned;

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

    enable_graph: bool,
}

#[derive(Debug, Default)]
struct Observer {
    map: FxHashMap<ObjectPath, Value>,
}

impl Observer {
    fn update(&mut self, sim: &Sim<()>) {
        for (path, value) in &mut self.map {
            let Ok(module) = sim.globals().node(path.clone()) else {
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

#[derive(Default, Debug)]
pub struct ExecutionParameters {
    limit: Option<usize>,
    pre_frame_count: usize,
}

impl Application {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>, f: impl FnOnce() -> Runtime<Sim<()>>) -> Self {
        let gui_capture = GuiTracingObserver::default();
        let stdout = std::io::stdout;
        let subscriber = tracing_subscriber::Registry::default()
            .with(
                EnvFilter::builder()
                    .with_default_directive(Directive::from(FALLBACK_LOG_LEVEL))
                    .from_env_lossy(),
            )
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
                pre_frame_count: 0,
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

            enable_graph: false,
        }
    }

    fn run_sim_step(&mut self, ctx: &egui::Context) -> ControlFlow<()> {
        // setup tracers
        while let Ok(req) = self.tx_rx.1.try_recv() {
            match req {
                ActionReq::Breakpoint(req) => {
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
                let steps = self.param.pre_frame_count as usize;
                if !runtime.was_started() {
                    runtime.start();
                }

                'outer: for _ in 0..steps {
                    runtime.dispatch_n_events(1);

                    self.observe.update(&runtime.app);

                    for b in &mut self.breakpoints {
                        if let ControlFlow::Break(()) = b.update(&self.observe) {
                            self.param.limit = Some(0);
                            break 'outer;
                        }
                    }
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
                Some((&key[..], Cow::<ValueOwned>::Owned(value)))
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
        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        if let ControlFlow::Break(_) = self.run_sim_step(ctx) {
            return;
        }

        self.render_controls(ctx);

        self.modals.retain(|v| !v.remove);
        for modal in &mut self.modals {
            ctx.show_viewport_immediate(
                egui::ViewportId(Id::new(format!("panel-{}", modal.path))),
                ViewportBuilder::default()
                    .with_title(modal.path.to_string())
                    .with_inner_size([500.0, 1200.0]),
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

        self.render_breakpoints(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.enable_graph {
                ui.add(
                    Image::new(format!("file://{}topo.png", self.dir.as_path().display()))
                        .shrink_to_fit(),
                );
            }
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

    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin
        .write_all(topo.as_dot().as_bytes())
        .expect("write failed");
    drop(stdin);

    let output = child.wait_with_output().expect("wait failed");
    File::create(format!("{}topo.png", dir.display()))
        .unwrap()
        .write_all(&output.stdout)
        .unwrap();

    ::tracing::info!("wrote topo to {}", format!("{}topo.png", dir.display()));
}
