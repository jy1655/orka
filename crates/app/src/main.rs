mod cli;
mod config;
mod envfile;
mod health;
mod ops;
mod outbound;
mod runtime;

use std::collections::HashMap;
use std::env;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{anyhow, Error, Result};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use config::{AppConfig, RuntimeEngine};
use health::{spawn_health_server, HealthState};
use outbound::MultiplexOutbound;
use runtime::CliAgentRuntime;

use orka_adapters_discord::{DiscordAdapter, DiscordOutbound};
use orka_adapters_telegram::{TelegramAdapter, TelegramOutbound};
use orka_core::model::RuntimePreference;
use orka_core::model::{Channel, InboundEvent};
use orka_core::pipeline::GatewayPipeline;
use orka_core::policy::AccessPolicy;
use orka_core::ports::{AgentRuntime, EchoAgentRuntime, EventStore, OutboundSender};
use orka_core::rate_limit::RateLimiter;
use orka_core::session::session_key_for_event;
use orka_storage_sqlite::{SqliteStore, StorageOptions};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = cli::parse_app_command(&args).map_err(Error::msg)?;
    let cwd = env::current_dir()?;
    let env_path = envfile::load_dotenv_upwards(&cwd)?;
    let workspace_root = env_path
        .as_deref()
        .and_then(|path| path.parent())
        .unwrap_or(cwd.as_path())
        .to_path_buf();

    match command {
        cli::AppCommand::RunGateway => {
            if cwd != workspace_root {
                env::set_current_dir(&workspace_root)?;
            }
            init_tracing();
            let cfg = AppConfig::from_env();
            run_gateway(cfg).await
        }
        cli::AppCommand::Doctor => {
            let cfg = AppConfig::from_env();
            ops::run_doctor(&cfg, &workspace_root, env_path.as_deref())
        }
        cli::AppCommand::Status { deep } => {
            let cfg = AppConfig::from_env();
            ops::run_status(&cfg, &workspace_root, env_path.as_deref(), deep)
        }
        cli::AppCommand::Onboard { force } => ops::run_onboard(&workspace_root, force),
    }
}

async fn run_gateway(cfg: AppConfig) -> Result<()> {
    info!("booting orka-gateway");

    let store = init_store(&cfg).await?;
    let pipeline = build_pipeline(&cfg, store)?;

    let health_state = HealthState::new(pipeline.clone());
    let mut health_task =
        spawn_health_server(cfg.health_bind.clone(), health_state.clone()).await?;

    let (inbound_tx, mut inbound_rx) = mpsc::channel(1024);
    let mut inbound_tx = Some(inbound_tx);
    let mut adapter_tasks = spawn_adapters(
        &cfg,
        inbound_tx
            .as_ref()
            .expect("inbound sender is available before shutdown"),
    );

    if adapter_tasks.is_empty() {
        warn!("no adapters enabled; set DISCORD_BOT_TOKEN and/or TELEGRAM_BOT_TOKEN");
    }

    health_state.ready.store(true, Ordering::Relaxed);
    info!("gateway ready");

    let concurrency = Arc::new(Semaphore::new(cfg.max_concurrent_tasks));
    let scope_locks: Arc<Mutex<HashMap<String, Arc<Semaphore>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(
        cfg.rate_limit_window_secs,
        cfg.rate_limit_max_requests,
    )));
    let mut inflight = JoinSet::new();

    let run_result = run_main_loop(
        &cfg,
        pipeline.clone(),
        &health_state,
        &mut health_task,
        &mut adapter_tasks,
        &mut inbound_tx,
        &mut inbound_rx,
        concurrency,
        scope_locks,
        rate_limiter,
        &mut inflight,
    )
    .await;

    abort_adapter_tasks(&mut adapter_tasks);
    health_task.abort();

    match run_result {
        Ok(()) => {
            info!("gateway stopped");
            Ok(())
        }
        Err(err) => {
            error!("gateway stopped with error: {err:#}");
            Err(err)
        }
    }
}

type AdapterTaskResult = (&'static str, Result<()>);
type AdapterTasks = JoinSet<AdapterTaskResult>;
type HealthTask = JoinHandle<Result<()>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatMode {
    None,
    Typing,
    ReplyProgress,
}

fn heartbeat_mode_for_event(event: &InboundEvent) -> HeartbeatMode {
    if event.reply_token.is_some() {
        HeartbeatMode::ReplyProgress
    } else if event.channel == Channel::Discord {
        HeartbeatMode::Typing
    } else {
        HeartbeatMode::None
    }
}

async fn init_store(cfg: &AppConfig) -> Result<Arc<dyn EventStore>> {
    let store = SqliteStore::connect_with_options(
        &cfg.database_url,
        StorageOptions {
            store_full_payloads: cfg.store_full_payloads,
        },
    )
    .await?;
    store.migrate().await?;
    Ok(Arc::new(store))
}

fn build_pipeline(cfg: &AppConfig, store: Arc<dyn EventStore>) -> Result<Arc<GatewayPipeline>> {
    let policy = AccessPolicy::new(cfg.allowlist.clone(), cfg.open_access);
    let runtime = build_runtime(cfg)?;
    let outbound = build_outbound(cfg);
    let default_runtime = RuntimePreference {
        provider: cfg.default_provider,
        mode: cfg.default_runtime_mode,
    };

    Ok(Arc::new(GatewayPipeline::new(
        store,
        runtime,
        outbound,
        policy,
        default_runtime,
        cfg.session_fail_fallback_event,
        cfg.operator_env_report(),
    )))
}

fn build_runtime(cfg: &AppConfig) -> Result<Arc<dyn AgentRuntime>> {
    match cfg.runtime_engine {
        RuntimeEngine::Echo => Ok(Arc::new(EchoAgentRuntime)),
        RuntimeEngine::Cli => Ok(Arc::new(CliAgentRuntime::new(cfg.cli_runtime.clone())?)),
    }
}

fn build_outbound(cfg: &AppConfig) -> Arc<dyn OutboundSender> {
    let multiplex = MultiplexOutbound::new(
        Arc::new(DiscordOutbound::with_options(
            cfg.discord_bot_token.clone(),
            cfg.discord_use_embeds,
        )),
        Arc::new(TelegramOutbound::new(cfg.telegram_bot_token.clone())),
    );
    Arc::new(multiplex)
}

fn spawn_adapters(cfg: &AppConfig, inbound_tx: &mpsc::Sender<InboundEvent>) -> AdapterTasks {
    let mut tasks = JoinSet::new();

    if cfg.discord_bot_token.trim().is_empty() {
        warn!("discord adapter is disabled");
    } else {
        let discord = DiscordAdapter::new(cfg.discord_bot_token.clone(), inbound_tx.clone());
        tasks.spawn(async move { ("discord", discord.run().await) });
    }

    if cfg.telegram_bot_token.trim().is_empty() {
        warn!("telegram adapter is disabled");
    } else {
        let telegram = TelegramAdapter::new(cfg.telegram_bot_token.clone(), inbound_tx.clone());
        tasks.spawn(async move { ("telegram", telegram.run().await) });
    }

    tasks
}

#[allow(clippy::too_many_arguments)]
async fn run_main_loop(
    cfg: &AppConfig,
    pipeline: Arc<GatewayPipeline>,
    health_state: &HealthState,
    health_task: &mut HealthTask,
    adapter_tasks: &mut AdapterTasks,
    inbound_tx: &mut Option<mpsc::Sender<InboundEvent>>,
    inbound_rx: &mut mpsc::Receiver<InboundEvent>,
    concurrency: Arc<Semaphore>,
    scope_locks: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    inflight: &mut JoinSet<()>,
) -> Result<()> {
    let mut eviction_counter: u64 = 0;

    loop {
        tokio::select! {
          _ = tokio::signal::ctrl_c() => {
            health_state.ready.store(false, Ordering::Relaxed);
            info!(
              drain_timeout_ms = cfg.shutdown_drain_timeout_ms,
              "shutdown signal received; stopping adapters and draining"
            );
            abort_adapter_tasks(adapter_tasks);
            let _ = inbound_tx.take();

            // Wait for all in-flight tasks to complete before draining the queue.
            await_inflight(inflight, cfg.shutdown_drain_timeout_ms / 2).await;

            // Drain remaining queued (not yet spawned) events sequentially.
            drain_queued_events(&pipeline, inbound_rx, cfg.shutdown_drain_timeout_ms / 2).await;
            break;
          }
          health_result = &mut *health_task => {
            match health_result {
              Ok(Ok(())) => warn!("health server stopped"),
              Ok(Err(err)) => return Err(err.context("health server task failed")),
              Err(err) => return Err(err.into()),
            }
            break;
          }
          adapter_result = adapter_tasks.join_next(), if !adapter_tasks.is_empty() => {
            health_state.ready.store(false, Ordering::Relaxed);
            let _ = inbound_tx.take();
            inflight.abort_all();
            abort_adapter_tasks(adapter_tasks);

            match adapter_result {
              Some(Ok((name, result))) => return Err(unexpected_adapter_exit(name, result)),
              Some(Err(err)) => return Err(anyhow!("adapter task join failed: {err}")),
              None => return Err(anyhow!("all adapter tasks stopped unexpectedly")),
            }
          }
          // Reap completed tasks to keep the JoinSet from growing.
          Some(_) = inflight.join_next(), if !inflight.is_empty() => {}
          maybe_event = inbound_rx.recv() => {
            if let Some(event) = maybe_event {
              let scope_key = session_key_for_event(&event);

              {
                let mut rl = rate_limiter.lock().await;
                if !rl.check(&scope_key) {
                  warn!(scope_key = %scope_key, "rate limited");
                  let limited = event.reply("Rate limited. Please wait before sending more requests.".to_string());
                  if let Err(err) = pipeline.dispatch_outbound(&limited, Some(&scope_key)).await {
                    error!("failed to send rate limit response: {err}");
                  }
                  continue;
                }
              }

              let task_pipeline = pipeline.clone();
              let task_concurrency = concurrency.clone();
              let task_scope_locks = scope_locks.clone();

              let heartbeat_mode = heartbeat_mode_for_event(&event);
              let heartbeat_outbound = if heartbeat_mode == HeartbeatMode::None {
                None
              } else {
                Some(pipeline.outbound().clone())
              };

              inflight.spawn(async move {

                let _global_permit = match task_concurrency.acquire().await {
                  Ok(permit) => permit,
                  Err(_) => {
                    error!("global concurrency semaphore closed");
                    return;
                  }
                };

                let scope_sem = {
                  let mut locks = task_scope_locks.lock().await;
                  locks
                    .entry(scope_key.clone())
                    .or_insert_with(|| Arc::new(Semaphore::new(1)))
                    .clone()
                };

                let _scope_permit = match scope_sem.try_acquire() {
                  Ok(permit) => permit,
                  Err(_) => {
                    warn!(scope_key = %scope_key, "scope is busy; sending busy response");
                    let busy = event.reply(
                      "This scope is currently processing a request. Please wait and try again.".to_string(),
                    );
                    if let Err(err) = task_pipeline.dispatch_outbound(&busy, Some(&scope_key)).await {
                      error!("failed to send busy response: {err}");
                    }
                    return;
                  }
                };

                let heartbeat_handle = heartbeat_outbound.map(|outbound| {
                  let heartbeat_event = event.clone();
                  tokio::spawn(async move {
                    let mut elapsed_secs = 5u64;
                    loop {
                      let result = match heartbeat_mode {
                        HeartbeatMode::None => return,
                        HeartbeatMode::Typing => {
                          if elapsed_secs == 5 {
                            outbound
                              .send_typing(heartbeat_event.channel, &heartbeat_event.chat_id)
                              .await
                          } else {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            outbound
                              .send_typing(heartbeat_event.channel, &heartbeat_event.chat_id)
                              .await
                          }
                        }
                        HeartbeatMode::ReplyProgress => {
                          tokio::time::sleep(Duration::from_secs(5)).await;
                          let mut progress = heartbeat_event.reply(String::new());
                          progress.text = format!("Processing... ({elapsed_secs}s)");
                          outbound.send(&progress).await
                        }
                      };

                      if let Err(err) = result {
                        warn!("heartbeat send failed: {err}");
                        break;
                      }

                      elapsed_secs += 5;
                    }
                  })
                });

                let result = task_pipeline.handle_event(event).await;

                if let Some(handle) = heartbeat_handle {
                  handle.abort();
                }

                if let Err(err) = result {
                  error!("pipeline error: {err}");
                }
              });

              // Periodically evict unused scope locks (every 100 events).
              eviction_counter += 1;
              if eviction_counter % 100 == 0 {
                evict_unused_scope_locks(&scope_locks).await;
                rate_limiter.lock().await.evict_stale();
              }
            } else {
              warn!("inbound queue closed unexpectedly");
              break;
            }
          }
        }
    }
    Ok(())
}

fn unexpected_adapter_exit(name: &str, result: Result<()>) -> Error {
    match result {
        Ok(()) => anyhow!("{name} adapter stopped unexpectedly"),
        Err(err) => err.context(format!("{name} adapter stopped unexpectedly")),
    }
}

async fn evict_unused_scope_locks(scope_locks: &Arc<Mutex<HashMap<String, Arc<Semaphore>>>>) {
    let mut locks = scope_locks.lock().await;
    let before = locks.len();
    // An Arc with strong_count == 1 means only the HashMap holds it;
    // no task is currently using this scope lock.
    locks.retain(|_, sem| Arc::strong_count(sem) > 1);
    let evicted = before - locks.len();
    if evicted > 0 {
        info!(
            evicted,
            remaining = locks.len(),
            "evicted unused scope locks"
        );
    }
}

async fn await_inflight(inflight: &mut JoinSet<()>, timeout_ms: u64) {
    if inflight.is_empty() {
        return;
    }
    let count = inflight.len();
    info!(
        inflight_tasks = count,
        "waiting for in-flight tasks to complete"
    );

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if inflight.is_empty() {
            info!("all in-flight tasks completed");
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                remaining_tasks = inflight.len(),
                "in-flight drain timeout; aborting remaining tasks"
            );
            inflight.abort_all();
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(remaining) => {
                warn!(
                    remaining_tasks = inflight.len(),
                    "in-flight drain timeout; aborting remaining tasks"
                );
                inflight.abort_all();
                break;
            }
            result = inflight.join_next() => {
                if result.is_none() {
                    break;
                }
            }
        }
    }
}

fn abort_adapter_tasks(tasks: &mut AdapterTasks) {
    tasks.abort_all();
}

async fn drain_queued_events(
    pipeline: &GatewayPipeline,
    inbound_rx: &mut mpsc::Receiver<InboundEvent>,
    timeout_ms: u64,
) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut drained_events = 0_u64;

    loop {
        if Instant::now() >= deadline {
            warn!(
                drained_events,
                "shutdown drain timeout reached; forcing stop"
            );
            break;
        }

        match inbound_rx.try_recv() {
            Ok(event) => {
                drained_events += 1;
                if let Err(err) = pipeline.handle_event(event).await {
                    error!("pipeline error during shutdown drain: {err}");
                }
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {
                info!(drained_events, "queued event drain complete");
                break;
            }
        }
    }
}

fn init_tracing() {
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .json()
        .init();
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use chrono::Utc;

    use super::{heartbeat_mode_for_event, unexpected_adapter_exit, HeartbeatMode};
    use orka_core::model::{Channel, InboundEvent};

    fn inbound_event(channel: Channel, reply_token: Option<&str>) -> InboundEvent {
        InboundEvent {
            idempotency_key: "evt-1".to_string(),
            channel,
            chat_id: "123".to_string(),
            user_id: "user-1".to_string(),
            text: "hello".to_string(),
            received_at: Utc::now(),
            is_direct_message: false,
            reply_token: reply_token.map(str::to_string),
            claims: vec![],
            attachments: vec![],
        }
    }

    #[test]
    fn heartbeat_mode_uses_typing_for_discord_messages_without_reply_token() {
        let event = inbound_event(Channel::Discord, None);
        assert_eq!(heartbeat_mode_for_event(&event), HeartbeatMode::Typing);
    }

    #[test]
    fn heartbeat_mode_preserves_progress_edits_for_interactions() {
        let event = inbound_event(Channel::Discord, Some("1:token"));
        assert_eq!(
            heartbeat_mode_for_event(&event),
            HeartbeatMode::ReplyProgress
        );
    }

    #[test]
    fn heartbeat_mode_stays_disabled_for_plain_telegram_messages() {
        let event = inbound_event(Channel::Telegram, None);
        assert_eq!(heartbeat_mode_for_event(&event), HeartbeatMode::None);
    }

    #[test]
    fn unexpected_adapter_exit_marks_clean_stop_as_failure() {
        let err = unexpected_adapter_exit("discord", Ok(()));
        assert_eq!(err.to_string(), "discord adapter stopped unexpectedly");
    }

    #[test]
    fn unexpected_adapter_exit_preserves_source_error_context() {
        let err = unexpected_adapter_exit("telegram", Err(anyhow!("boom")));
        assert_eq!(err.to_string(), "telegram adapter stopped unexpectedly");
        let chain = err.chain().map(|item| item.to_string()).collect::<Vec<_>>();
        assert!(chain.iter().any(|item| item == "boom"));
    }
}
