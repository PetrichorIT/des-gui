use des::{
    net::{HandlerFn, ModuleFn},
    prelude::*,
};
use tracing::info_span;

pub fn sim() -> Runtime<Sim<()>> {
    let mut sim = Sim::new(());
    sim.node(
        "ping",
        ModuleFn::new(
            || 0usize,
            |state, _| {
                *state += 1;
                info_span!("pinger", state).in_scope(|| {
                    tracing::info!("PONG");
                    current().prop::<usize>("counter").unwrap().set(*state);
                    current()
                        .prop::<String>("key")
                        .unwrap()
                        .set("value".to_string());
                });
            },
        ),
    );
    sim.node(
        "pong",
        ModuleFn::new(
            || 0usize,
            |state, msg| {
                *state += 1;
                tracing::info!("PING");
                send(msg, "port");
                current().prop::<usize>("counter").unwrap().set(*state);
            },
        ),
    );

    sim.node("pang", HandlerFn::new(|_| {}));
    sim.node("peng", HandlerFn::new(|_| {}));

    sim.gate("ping", "port").connect(
        sim.gate("pong", "port"),
        Some(Channel::new(ChannelMetrics::new(
            8000,
            Duration::from_millis(20),
            Duration::ZERO,
            ChannelDropBehaviour::Queue(None),
        ))),
    );

    sim.gate("pang", "pp").connect(sim.gate("ping", "pp"), None);
    sim.gate("pang", "pe").connect(sim.gate("peng", "pe"), None);

    let gate = sim.gate("ping", "port");

    let mut rt = Builder::seeded(123).build(sim);
    for i in 0..100 {
        rt.add_message_onto(
            gate.clone(),
            Message::new().id(i).build(),
            (i as f64).into(),
        );
    }

    rt
}
