//! S6/K1: signaling resilience -- the supervisor reconnect chain + a per-generation forwarder + consumer re-mount replay.
//!
//! The state machine (single truth source = the ctx.state_tx watch):
//!
//! ```text
//! connect succeeds --> Connected --link break (Disconnected)--> Reconnecting --rebuild+replay--> Connected
//!                      │                                  │
//!        a reconnect rejected with a non-retryable code (auth family/4101) --> Failed (terminal, the loop exits)   auto_reconnect=off --> Disconnected
//! ```
//!
//! Event surface: link session events are relayed into `ev_tx` by the forwarder (RoomSession's subscribers stay
//! continuous across reconnects; the app has no generation concept). Frame surface: the registered slot's frame_tx survives reconnects -- the replay swaps in a new pc
//! without swapping the slot, the app's receiver keeps streaming.
//!
//! Discipline: never blocking_lock in async context (all LinkSignal access goes through lock().await); error branches
//! always log (C15); a single-path replay failure never couples into other paths.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mediaservo_link::{SignalClient, SignalEvent};
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc, watch};

use crate::consumer::ConsumerSlot;
use crate::engine::Engine;
use crate::error::classify_link_error;
use crate::session::consume_sequence;
use crate::signal::LinkSignal;

/// The session connection state (the K1 observation surface; [`crate::RoomSession::connection_state`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// auto_reconnect off and disconnected (the supervisor has exited, no more reconnects).
    Disconnected,
    /// Signaling connected (the normal serving state).
    Connected,
    /// Reconnecting after a break (inside the unbounded-backoff loop).
    Reconnecting,
    /// A reconnect rejected by a non-retryable error (the auth family/4101) -- terminal, the loop has exited (D273 red-card semantics).
    Failed,
}

/// Event-relay broadcast capacity (forwarder -> subscribers; lag = WARN, never silent).
const EV_RELAY_CAP: usize = 128;
/// Per-path replay attempts and interval (K1: 3 exhausted attempts abandons that path, WARN on record).
const REPLAY_ATTEMPTS: u32 = 3;
const REPLAY_GAP: Duration = Duration::from_millis(200);

/// Reconnect backoff params (exponential x2 capped; the first try is immediate, sleeping after failure).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReconnectParams {
    base: Duration,
    max: Duration,
}

impl Default for ReconnectParams {
    fn default() -> Self {
        Self { base: Duration::from_millis(200), max: Duration::from_secs(5) }
    }
}

/// The startup trio (epoch-1 forwarder feeding + the dead mailbox + the shutdown subscription) -- taken by start().
type Boot = (broadcast::Receiver<SignalEvent>, mpsc::UnboundedReceiver<()>, watch::Receiver<bool>);

/// The supervisor context -- shared between RoomSession and the background tasks (a single Arc instance; the only "session-level" pieces
/// surviving a swap are ev_tx/slots/state/shutdown; the link session body swaps cores inside signal).
pub(crate) struct SupervisorCtx {
    pub(crate) signal: Arc<LinkSignal>,
    pub(crate) engine: Arc<dyn Engine>,
    /// The relay event bus: written by the forwarder, subscribed by RoomSession/replay.
    pub(crate) ev_tx: broadcast::Sender<SignalEvent>,
    /// The K4 registration slots (the replay targets; liveness = receivers closed).
    pub(crate) slots: Arc<std::sync::Mutex<Vec<Arc<ConsumerSlot>>>>,
    /// User consumes and replays serialize (the SFU acks share one event stream, never racing for replies).
    pub(crate) consume_lock: AsyncMutex<()>,
    client: Arc<SignalClient>,
    state_tx: watch::Sender<ConnectionState>,
    shutdown_tx: watch::Sender<bool>,
    dead_tx: mpsc::UnboundedSender<()>,
    auto_reconnect: AtomicBool,
    params: ReconnectParams,
    supervisor: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    boot: std::sync::Mutex<Option<Boot>>,
}

impl SupervisorCtx {
    /// Build (no spawn): RoomSession takes the events/pump_events subscriptions first, then [`start`].
    pub(crate) fn new(
        signal: Arc<LinkSignal>,
        client: Arc<SignalClient>,
        engine: Arc<dyn Engine>,
        raw_events: broadcast::Receiver<SignalEvent>,
    ) -> Arc<Self> {
        let (ev_tx, _) = broadcast::channel(EV_RELAY_CAP);
        let (dead_tx, dead_rx) = mpsc::unbounded_channel();
        let (state_tx, _) = watch::channel(ConnectionState::Connected);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Arc::new(Self {
            signal,
            engine,
            ev_tx,
            slots: Arc::new(std::sync::Mutex::new(Vec::new())),
            consume_lock: AsyncMutex::new(()),
            client,
            state_tx,
            shutdown_tx,
            dead_tx,
            auto_reconnect: AtomicBool::new(true),
            params: ReconnectParams::default(),
            supervisor: std::sync::Mutex::new(None),
            boot: std::sync::Mutex::new(Some((raw_events, dead_rx, shutdown_rx))),
        })
    }

    #[must_use]
    pub(crate) fn state(&self) -> ConnectionState {
        *self.state_tx.borrow()
    }

    pub(crate) fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_tx.subscribe()
    }

    pub(crate) fn set_auto_reconnect(&self, on: bool) {
        self.auto_reconnect.store(on, Ordering::Release);
    }

    /// Wrap-up: the shutdown broadcast (the forwarder exits with the watch) + supervisor abort.
    /// Shared single point of close()/Drop; idempotent (repeat calls harmless).
    pub(crate) fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.supervisor.lock().unwrap_or_else(|e| e.into_inner()).take() {
            handle.abort();
        }
    }

    fn set_state(&self, s: ConnectionState) {
        let _ = self.state_tx.send(s);
    }
}

/// spawns the epoch-1 forwarder + supervisor (called once by connect_with_engine; repeat calls rejected).
pub(crate) fn start(ctx: &Arc<SupervisorCtx>) {
    let taken = ctx.boot.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((raw, dead_rx, shutdown_rx)) = taken else {
        tracing::warn!("supervisor: a repeat start was ignored (boot already took it)");
        return;
    };
    spawn_forwarder(ctx, raw);
    let handle = tokio::spawn(run_supervisor(ctx.clone(), dead_rx, shutdown_rx));
    *ctx.supervisor.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

fn spawn_forwarder(ctx: &Arc<SupervisorCtx>, raw: broadcast::Receiver<SignalEvent>) {
    tokio::spawn(run_forwarder(ctx.clone(), raw));
}

/// The per-generation forwarder: the link session event stream -> the relay bus. Connected is swallowed (the join ack was
/// consumed synchronously by connect); Disconnected forwarded + dead-mailbox notified, then exit; Closed (the old session
/// released) = exit after the dead notice; shutdown = exit.
async fn run_forwarder(ctx: Arc<SupervisorCtx>, mut raw: broadcast::Receiver<SignalEvent>) {
    let mut shutdown = ctx.shutdown_tx.subscribe();
    eprintln!("[fwd] forwarder started, waiting for events...");
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            ev = raw.recv() => match ev {
                Ok(SignalEvent::Connected { .. }) => { eprintln!("[fwd] got Connected"); }
                Ok(SignalEvent::Disconnected { reason }) => {
                    eprintln!("[fwd] got Disconnected({reason}), sending dead");
                    let _ = ctx.ev_tx.send(SignalEvent::Disconnected { reason });
                    let _ = ctx.dead_tx.send(());
                    break;
                }
                Ok(m @ (SignalEvent::Message(_) | SignalEvent::Error(_))) => {
                    tracing::debug!("forwarder: relay Message/Error");
                    let _ = ctx.ev_tx.send(m);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[fwd] LAGGED {n}");
                    tracing::warn!("forwarder: relay queue overflow dropped {n} events (continuing)");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    eprintln!("[fwd] channel CLOSED, sending dead");
                    let _ = ctx.dead_tx.send(());
                    break;
                }
                Ok(_) => {}
            },
        }
    }
}

/// The supervisor reconnect chain: dead -> unbounded-backoff rebuild (carrying a one-shot resume ticket) -> swap ->
/// replay -> Connected. A non-retryable rejection = Failed terminal exit; shutdown = exit.
async fn run_supervisor(
    ctx: Arc<SupervisorCtx>,
    mut dead_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    dead = dead_rx.recv() => {
                        if dead.is_none() {
                            break; // dead_tx released with ctx (theoretically unreachable, handled exhaustively)
                        }
                        if !ctx.auto_reconnect.load(Ordering::Acquire) || ctx.state() == ConnectionState::Failed {
                            ctx.set_state(ConnectionState::Disconnected);
                            break;
                        }
                        tracing::info!("supervisor: signaling broke, entering the reconnect chain");
                        ctx.set_state(ConnectionState::Reconnecting);
                        let mut delay = ctx.params.base;
        // the a2 one-shot ticket: only the first attempt carries the old session's nonce (link takes-and-burns it; a failure never replays the ticket).
                        let mut pending_nonce = ctx.signal.session_nonce().await;
                        loop {
                            ctx.client.set_resume_ticket(pending_nonce.take());
                            match ctx.client.connect().await {
                                Ok(new_session) => {
        // the new session's event stream is subscribed synchronously before the swap (blocking_lock on LinkSignal::events in async context
        // would panic -- the subscription must complete at this sync point).
                                    let raw = new_session.events();
                                    ctx.signal.swap_session(new_session).await;
                                    spawn_forwarder(&ctx, raw);
                                    replay_consumers(&ctx).await;
                                    ctx.set_state(ConnectionState::Connected);
        // drain stale dead notices in the same window (echoes of the old generation during replay), never reconnecting twice.
                                    while dead_rx.try_recv().is_ok() {}
                                    break;
                                }
                                Err(e) => {
                                    let e = classify_link_error(e);
                                    tracing::warn!(error = %e, "supervisor: a reconnect attempt failed");
                                    if !e.is_retryable() {
                                        ctx.set_state(ConnectionState::Failed);
                                        return;
                                    }
                                    // ponytail: no max attempts, cockpit viewer must heal forever;
                                    // D273 red-card family = auth-only terminal
                                    tokio::select! {
                                        biased;
                                        _ = shutdown.changed() => return,
                                        _ = tokio::time::sleep(delay) => {}
                                    }
                                    delay = delay.saturating_mul(2).min(ctx.params.max);
                                }
                            }
                        }
                    }
                }
    }
}

/// After a successful reconnect, replay the registered slots path by path: <=3 attempts x 200ms each; a single-path failure never couples into others (C15 WARN).
/// A pc swap without a slot swap = the app's frame receiver keeps streaming across reconnects.
async fn replay_consumers(ctx: &Arc<SupervisorCtx>) {
    let _seq = ctx.consume_lock.lock().await;
    let slots: Vec<Arc<ConsumerSlot>> = ctx.slots.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for slot in slots {
        if slot.is_dead() {
            tracing::debug!("K1 replay: the path already withdrew (receivers closed), skipping");
            continue;
        }
        let mut healed = false;
        for attempt in 1..=REPLAY_ATTEMPTS {
            let mut ev = ctx.ev_tx.subscribe();
            let result = consume_sequence(
                slot.producer_id(),
                &ctx.engine,
                &ctx.signal,
                &mut ev,
                slot.frame_tx(),
            )
            .await;
            match result {
                Ok(pc) => {
                    slot.set_pc(pc);
                    tracing::info!(
                        producer_id = slot.producer_id(),
                        attempt,
                        "K1 replay: the consumer path rebuilt (the frame stream continues on the original receiver)"
                    );
                    healed = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        producer_id = slot.producer_id(),
                        attempt,
                        error = %e,
                        "K1 replay: an attempt failed"
                    );
                    if attempt < REPLAY_ATTEMPTS {
                        let mut shutdown = ctx.shutdown_tx.subscribe();
                        tokio::select! {
                            biased;
                            _ = shutdown.changed() => return,
                            _ = tokio::time::sleep(REPLAY_GAP) => {}
                        }
                    }
                }
            }
        }
        if !healed {
            tracing::warn!(
                producer_id = slot.producer_id(),
                "K1 replay: attempts exhausted for this path, abandoning it (never coupling into others; that stream stops updating on the app side)"
            );
        }
    }
}
