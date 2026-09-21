//! S6/K1：信令面韧性——supervisor 重连环 + 每代 forwarder + consumer 重挂重放。
//!
//! 状态机（单一真源 = ctx.state_tx watch）：
//!
//! ```text
//! connect 成功 ──▶ Connected ──断链(Disconnected)──▶ Reconnecting ──重建+重放──▶ Connected
//!                      │                                  │
//!        重连被不可重试码拒(auth 族/4101) ──▶ Failed(终态，环退出)   auto_reconnect=off ──▶ Disconnected
//! ```
//!
//! 事件面：link 会话事件经 forwarder 中继进 `ev_tx`（RoomSession 各订阅者跨
//! 重连连续，app 无代际概念）。帧面：注册槽 frame_tx 跨重连存续——重放换新 pc
//! 不换槽，app 手里的 receiver 续流。
//!
//! 纪律：async 态禁 blocking_lock（LinkSignal 全部走 lock().await）；错误分支
//! 恒日志（C15）；重放单路失败不连坐他路。

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

/// 会话连接态（K1 观测面；[`crate::RoomSession::connection_state`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// auto_reconnect 关闭且断链（supervisor 已退出，不再重连）。
    Disconnected,
    /// 信令已连接（正常服务态）。
    Connected,
    /// 断链后 supervisor 重连中（无限退避环内）。
    Reconnecting,
    /// 重连被不可重试错误拒（auth 族/4101）——终态，环已退出（D273 红牌语义）。
    Failed,
}

/// 事件中继 broadcast 容量（forwarder→订阅者；lag = WARN 不静默）。
const EV_RELAY_CAP: usize = 128;
/// 重放单路尝试次数与间隔（K1：3 次耗尽即弃该路，WARN 留痕）。
const REPLAY_ATTEMPTS: u32 = 3;
const REPLAY_GAP: Duration = Duration::from_millis(200);

/// 重连退避参数（指数 ×2 封顶；首拍立即尝试，失败后睡）。
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

/// 启动三件（epoch-1 forwarder 入料 + dead 信箱 + shutdown 订阅）——start() 取走。
type Boot = (broadcast::Receiver<SignalEvent>, mpsc::UnboundedReceiver<()>, watch::Receiver<bool>);

/// supervisor 上下文——RoomSession 与后台任务共享（Arc 单实例；跨 swap 存续的
/// 只有 ev_tx/slots/state/shutdown 这些「会话级」件，link 会话本体在 signal 内换芯）。
pub(crate) struct SupervisorCtx {
    pub(crate) signal: Arc<LinkSignal>,
    pub(crate) engine: Arc<dyn Engine>,
    /// 中继事件总线：forwarder 写入，RoomSession/replay 订阅。
    pub(crate) ev_tx: broadcast::Sender<SignalEvent>,
    /// K4 注册槽（重放对象；死活 = 接收端 closed）。
    pub(crate) slots: Arc<std::sync::Mutex<Vec<Arc<ConsumerSlot>>>>,
    /// 用户 consume 与重放串行（SFU ack 共享事件流，不互抢应答）。
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
    /// 构建（不 spawn）：RoomSession 先取 events/pump_events 订阅，再 [`start`]。
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

    /// 收场：shutdown 广播（forwarder 随 watch 退出）+ supervisor abort。
    /// close()/Drop 共用单点；幂等（重复调用无害）。
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

/// spawn epoch-1 forwarder + supervisor（connect_with_engine 调一次；重复调用被拒）。
pub(crate) fn start(ctx: &Arc<SupervisorCtx>) {
    let taken = ctx.boot.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((raw, dead_rx, shutdown_rx)) = taken else {
        tracing::warn!("supervisor: 重复 start 被忽略（boot 已取走）");
        return;
    };
    spawn_forwarder(ctx, raw);
    let handle = tokio::spawn(run_supervisor(ctx.clone(), dead_rx, shutdown_rx));
    *ctx.supervisor.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

fn spawn_forwarder(ctx: &Arc<SupervisorCtx>, raw: broadcast::Receiver<SignalEvent>) {
    tokio::spawn(run_forwarder(ctx.clone(), raw));
}

/// 每代 forwarder：link 会话事件流 → 中继总线。Connected 吞（join ack 已被
/// connect 同步消费）；Disconnected 转发 + dead 信箱通知后退出；Closed（旧会话
/// 被释放）= dead 通知后退出；shutdown = 退出。
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
                    eprintln!("[fwd] got Message/Error (relay only)");
                    let _ = ctx.ev_tx.send(m);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[fwd] LAGGED {n}");
                    tracing::warn!("forwarder: 中继队列溢出丢 {n} 事件（续）");
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

/// supervisor 重连环：dead → 无限退避重建（带一次性 resume 票）→ swap →
/// 重放 → Connected。不可重试拒 = Failed 终态退出；shutdown = 退出。
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
                    break; // dead_tx 随 ctx 释放（理论不可达，穷尽处理）
                }
                if !ctx.auto_reconnect.load(Ordering::Acquire) || ctx.state() == ConnectionState::Failed {
                    ctx.set_state(ConnectionState::Disconnected);
                    break;
                }
                tracing::info!("supervisor: 信令断链，进入重连环");
                ctx.set_state(ConnectionState::Reconnecting);
                let mut delay = ctx.params.base;
                // a2 一次性票：仅首轮尝试带旧会话 nonce（link take 即焚，失败不重放票）。
                let mut pending_nonce = ctx.signal.session_nonce().await;
                loop {
                    ctx.client.set_resume_ticket(pending_nonce.take());
                    match ctx.client.connect().await {
                        Ok(new_session) => {
                            // 新会话事件流先同步订阅再 swap（LinkSignal::events 在 async 态
                            // blocking_lock 会 panic——订阅必须在此同步点完成）。
                            let raw = new_session.events();
                            ctx.signal.swap_session(new_session).await;
                            spawn_forwarder(&ctx, raw);
                            replay_consumers(&ctx).await;
                            ctx.set_state(ConnectionState::Connected);
                            // 排空同窗陈旧 dead 信（重放期间旧代余响），不双重重连。
                            while dead_rx.try_recv().is_ok() {}
                            break;
                        }
                        Err(e) => {
                            let e = classify_link_error(e);
                            tracing::warn!(error = %e, "supervisor: 重连尝试失败");
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

/// 重连成功后逐路重放注册槽：每路 ≤3 次 × 200ms；单路失败不连坐他路（C15 WARN）。
/// 换 pc 不换槽 = app 的帧 receiver 跨重连续流。
async fn replay_consumers(ctx: &Arc<SupervisorCtx>) {
    let _seq = ctx.consume_lock.lock().await;
    let slots: Vec<Arc<ConsumerSlot>> = ctx.slots.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for slot in slots {
        if slot.is_dead() {
            tracing::debug!("K1 重放: 路已撤走（接收端 closed），跳过");
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
                        "K1 重放: consumer 路已重建（帧流续原 receiver）"
                    );
                    healed = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        producer_id = slot.producer_id(),
                        attempt,
                        error = %e,
                        "K1 重放: 尝试失败"
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
                "K1 重放: 单路尝试耗尽即弃（不连坐他路；app 侧该流停更）"
            );
        }
    }
}
