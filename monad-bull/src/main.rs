use std::{
    collections::{BTreeMap, BTreeSet, HashMap}, marker::PhantomData, net::{IpAddr, SocketAddr, ToSocketAddrs}, path::{Path, PathBuf}, process, sync::{Arc, atomic::{AtomicU64, Ordering}}, thread::sleep, time::{Duration, Instant}
};

use monad_chain_config::ChainConfig;
use monad_consensus_state::ConsensusConfig;
use monad_chain_config::revision::CHAIN_PARAMS_LATEST;
use monad_consensus_types::{metrics::Metrics, validator_data::{ValidatorSetDataWithEpoch, ValidatorsConfig}};
use monad_control_panel::ipc::ControlPanelIpcReceiver;
use monad_crypto::certificate_signature::{
    CertificateSignaturePubKey, CertificateSignatureRecoverable, PubKey,
};

use monad_dataplane::DataplaneBuilder;
use monad_eth_block_policy::EthBlockPolicy;
use monad_eth_block_validator::EthBlockValidator;
use monad_eth_txpool_executor::{EthTxPoolExecutor, EthTxPoolIpcConfig};
use monad_eth_testutil::make_legacy_tx_with_chain_id;
use alloy_primitives::B256;
use monad_executor::{Executor, ExecutorMetricsChain};
use monad_executor_glue::{LogFriendlyMonadEvent, Message, MonadEvent, TxPoolCommand};
use monad_ledger::MonadBlockFileLedger;
use monad_node_config::{
    ExecutionProtocolType, FullNodeIdentityConfig, NodeBootstrapConfig, NodeConfig,
    PeerDiscoveryConfig, SignatureCollectionType, SignatureType,
};
use monad_peer_discovery::{
    discovery::{PeerDiscovery, PeerDiscoveryBuilder},
    MonadNameRecord, NameRecord,
};
use monad_pprof::start_pprof_server;

use monad_raptorcast::{
    config::{RaptorCastConfig, RaptorCastConfigPrimary},
    AUTHENTICATED_RAPTORCAST_SOCKET, RAPTORCAST_SOCKET,
};
use monad_router_multi::MultiRouter;
use monad_state::{MonadMessage, MonadStateBuilder, VerifiedMonadMessage};
use monad_state_backend::{NopStateBackend, StateBackendThreadClient};
use monad_executor_glue::StateSyncCommand;
use monad_types::{DropTimer, Epoch, NodeId, Round, SeqNum, GENESIS_SEQ_NUM};
use monad_updaters::{
    config_file::ConfigFile, config_loader::ConfigLoader, loopback::LoopbackExecutor,
    parent::ParentExecutor, timer::TokioTimer, tokio_timestamp::TokioTimestamp,
    val_set::MockValSetUpdaterNop,
};
use monad_validator::{
    signature_collection::SignatureCollection, validator_set::ValidatorSetFactory,
    weighted_round_robin::WeightedRoundRobin,
};
use monad_wal::wal::WALoggerConfig;

use clap::CommandFactory;
use futures_util::{FutureExt, StreamExt};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::UnixStream;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, event, info, warn, Instrument, Level};
use alloy_rlp::{Decodable, Encodable};
use chrono::Utc;
use rand::Rng;
use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};
use opentelemetry::metrics::MeterProvider;
use opentelemetry_otlp::{MetricExporter, WithExportConfig};

use self::{cli::Cli, error::NodeSetupError, state::NodeState};

use std::thread;

mod cli;
mod error;
mod state;

const MONAD_NODE_VERSION: Option<&str> = option_env!("MONAD_VERSION");
const EXECUTION_DELAY: u64 = 3;

fn main() {
    let mut cmd = Cli::command();

    let node_state = NodeState::setup(&mut cmd).unwrap_or_else(|e| cmd.error(e.kind(), e).exit());

    // thread.txt
    rayon::ThreadPoolBuilder::new()           // 创建一个新的线程池构建器
        .num_threads(8)                        // 设置线程池中有 8 个工作线程
        .thread_name(|i| format!("monad-bft-rn-{}", i))  // 为每个线程命名，格式为 "monad-bft-rn-{index}"
        .build_global()                        // 将此线程池设置为全局默认线程池
        .map_err(Into::into)                   // 将错误转换为 NodeSetupError 类型
        .unwrap_or_else(|e: NodeSetupError|    // 如果设置失败，使用 clap 的错误处理退出程序
            cmd.error(e.kind(), e).exit()
        );

    // thread.txt
    let runtime = tokio::runtime::Builder::new_multi_thread()  // 创建多线程运行时构建器
        .worker_threads(8)                                          // 设置工作线程数
        .enable_all()                                           // 启用所有 I/O 驱动和时间驱动
        .build()                                               // 构建运行时实例
        .map_err(Into::into)                                   // 将构建错误转换为 NodeSetupError
        .unwrap_or_else(|e: NodeSetupError|                    // 如果构建失败，使用 clap 错误处理退出
            cmd.error(e.kind(), e).exit()
        );

    drop(cmd);

    MONAD_NODE_VERSION.map(|v| info!("starting monad-bft with version {}", v));

    if let Some(pprof) = node_state.pprof.as_ref().filter(|p| p.len() > 0).cloned() {
        runtime.spawn({
            async {
                let server = match start_pprof_server(pprof) {
                    Ok(server) => server,
                    Err(err) => {
                        error!("failed to start pprof server: {}", err);
                        return;
                    }
                };
                if let Err(err) = server.await {
                    error!("pprof server failed: {}", err);
                }
            }
        });
    }

    if let Err(e) = runtime.block_on(run(node_state)) {
        error!("monad consensus node crashed: {:?}", e);
    }
}

async fn run(node_state: NodeState) -> Result<(), ()> {
    let locked_epoch_validators = node_state
        .validators_config
        .get_locked_validator_sets(&node_state.forkpoint_config);

    // ...checkpoint to store statedb
    let current_epoch = node_state
        .forkpoint_config
        .high_certificate
        .qc()
        .get_epoch();
    let current_round = node_state
        .forkpoint_config
        .high_certificate
        .qc()
        .get_round()
        + Round(1);
        
    let router = build_raptorcast_router::<
        SignatureType,
        SignatureCollectionType,
        MonadMessage<SignatureType, SignatureCollectionType, ExecutionProtocolType>,
        VerifiedMonadMessage<SignatureType, SignatureCollectionType, ExecutionProtocolType>,
    >(
        node_state.node_config.clone(),
        node_state.node_config.peer_discovery,
        node_state.router_identity,
        node_state.node_config.bootstrap.clone(),
        &node_state.node_config.fullnode_dedicated.identities,
        locked_epoch_validators.clone(),
        current_epoch,
        current_round,
    );

    let statesync_threshold: usize = node_state.node_config.statesync_threshold.into();

    _ = std::fs::remove_file(node_state.mempool_ipc_path.as_path());
    _ = std::fs::remove_file(node_state.control_panel_ipc_path.as_path());
    _ = std::fs::remove_file(node_state.statesync_ipc_path.as_path());

    let mut bootstrap_nodes = Vec::new();
    for peer_config in &node_state.node_config.bootstrap.peers {
        let peer_id = NodeId::new(peer_config.secp256k1_pubkey);
        bootstrap_nodes.push(peer_id);
    }

    // TODO: use PassThruBlockPolicy and NopStateBackend for consensus only mode
    let create_block_policy = || {
        EthBlockPolicy::new(
            GENESIS_SEQ_NUM, // FIXME: MonadStateBuilder is responsible for updating this to forkpoint root if necessary
            EXECUTION_DELAY,
        )
    };

    let state_backend = StateBackendThreadClient::new({
        move || {
            NopStateBackend::default()
        }
    });

    let mut executor = ParentExecutor {
        metrics: Default::default(),
        router,
        timer: TokioTimer::default(),
        ledger: MonadBlockFileLedger::new(node_state.ledger_path),
        config_file: ConfigFile::new(
            node_state.forkpoint_path,
            node_state.validators_path.clone(),
            node_state.chain_config,
        ),
        val_set: {
            let validators_config = ValidatorsConfig::<SignatureCollectionType>::read_from_path(&node_state.validators_path)
                .expect("failed to read validators config");
            let genesis_validator_data = validators_config.get_validator_set(&monad_types::Epoch(0)).clone();
            MockValSetUpdaterNop::<_, _, ExecutionProtocolType>::new(
                genesis_validator_data,
                node_state.chain_config.get_epoch_length(),
            )
        },
        timestamp: TokioTimestamp::new(Duration::from_millis(5), 100, 10001),
        txpool: EthTxPoolExecutor::start(
            create_block_policy(),
            state_backend.clone(),
            EthTxPoolIpcConfig {
                bind_path: node_state.mempool_ipc_path.clone(),
                tx_batch_size: node_state.node_config.ipc_tx_batch_size as usize,
                max_queued_batches: node_state.node_config.ipc_max_queued_batches as usize,
                queued_batches_watermark: node_state.node_config.ipc_queued_batches_watermark
                    as usize,
            },
            // TODO(andr-dev): Add tx_expiry to node config
            Duration::from_secs(15),
            Duration::from_secs(5 * 60),
            node_state.chain_config,
            node_state
                .forkpoint_config
                .high_certificate
                .qc()
                .get_round(),
            // TODO(andr-dev): Use timestamp from last commit in ledger
            0,
            true,
        )
        .expect("txpool ipc succeeds"),
        control_panel: ControlPanelIpcReceiver::new(
            node_state.control_panel_ipc_path,
            node_state.reload_handle,
            1000,
        )
        .expect("uds bind failed"),
        loopback: LoopbackExecutor::default(),
        // TODO ignore StateSync - use dummy executor
        state_sync: DummyStateSyncExecutor::default(),
        config_loader: ConfigLoader::new(node_state.node_config_path),
    };

    let logger_config: WALoggerConfig<LogFriendlyMonadEvent<_, _, _>> = WALoggerConfig::new(
        node_state.wal_path.clone(), // output wal path
        false,                       // flush on every write
    );
    let Ok(mut wal) = logger_config.build() else {
        event!(
            Level::ERROR,
            path = node_state.wal_path.as_path().display().to_string(),
            "failed to initialize wal",
        );
        return Err(());
    };

    let block_sync_override_peers = node_state
        .node_config
        .blocksync_override
        .peers
        .into_iter()
        .map(|p| NodeId::new(p.secp256k1_pubkey))
        .collect();

    let whitelisted_statesync_nodes = node_state
        .node_config
        .fullnode_dedicated
        .identities
        .into_iter()
        .map(|p| NodeId::new(p.secp256k1_pubkey))
        .chain(
            node_state
                .node_config
                .fullnode_raptorcast
                .full_nodes_prioritized
                .identities
                .into_iter()
                .map(|p| NodeId::new(p.secp256k1_pubkey)),
        )
        .collect();

    let mut last_ledger_tip: Option<SeqNum> = None;

    let builder = MonadStateBuilder {
        validator_set_factory: ValidatorSetFactory::default(),
        leader_election: WeightedRoundRobin::default(),
        block_validator: EthBlockValidator::default(),
        block_policy: create_block_policy(),
        state_backend,
        key: node_state.secp256k1_identity,
        certkey: node_state.bls12_381_identity,
        beneficiary: node_state.node_config.beneficiary.into(),
        forkpoint: node_state.forkpoint_config.into(),
        locked_epoch_validators,
        block_sync_override_peers,
        consensus_config: ConsensusConfig {
            execution_delay: SeqNum(EXECUTION_DELAY),
            delta: Duration::from_millis(100),
            // StateSync -> Live transition happens here
            statesync_to_live_threshold: SeqNum(statesync_threshold as u64),
            // Live -> StateSync transition happens here
            live_to_statesync_threshold: SeqNum(statesync_threshold as u64 * 3 / 2),
            // Live starts execution here
            start_execution_threshold: SeqNum(statesync_threshold as u64 / 2),
            chain_config: node_state.chain_config,
            timestamp_latency_estimate_ns: 20_000_000,
            _phantom: Default::default(),
        },
        whitelisted_statesync_nodes,
        statesync_expand_to_group: node_state.node_config.statesync.expand_to_group,
        _phantom: PhantomData,
    };

    let (mut state, init_commands) = builder.build();
    executor.exec(init_commands);

    let mut ledger_span = tracing::info_span!(
        "ledger_span",
        last_ledger_tip = last_ledger_tip.map(|s| s.as_u64())
    );

    let (maybe_otel_meter_provider, mut maybe_metrics_ticker) = node_state
        .otel_endpoint_interval
        .map(|(otel_endpoint, record_metrics_interval)| {
            let provider = build_otel_meter_provider(
                &otel_endpoint,
                format!(
                    "{network_name}_{node_name}",
                    network_name = node_state.node_config.network_name,
                    node_name = node_state.node_config.node_name
                ),
                node_state.node_config.network_name.clone(),
                record_metrics_interval,
            )
            .expect("failed to build otel monad-node");

            let mut timer = tokio::time::interval(record_metrics_interval);

            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            (provider, timer)
        })
        .unzip();

    let maybe_otel_meter = maybe_otel_meter_provider
        .as_ref()
        .map(|provider| provider.meter("opentelemetry"));

    let mut gauge_cache = HashMap::new();
    let process_start = Instant::now();
    let mut total_state_update_elapsed = Duration::ZERO;

    let enable_continuous_fill = std::env::var("MONAD_CONTINUOUS_FILL")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap_or(false);

    // 共享的 epoch 变量（供 lastCommit写 和 spawn任务读 使用）
    let shared_epoch = Arc::new(AtomicU64::new(1));
    let shared_seq_num = Arc::new(AtomicU64::new(0));
    if enable_continuous_fill {
        info!("Continuous fill enabled");

        let addresses = generate_secrets(50000).await;

        // 优化为 spawn 独立任务，避免阻塞主循环
        let _command_tx_clone = executor.txpool.command_tx.clone();  // 克隆 Arc 发送器
        let addresses_clone = addresses.clone();  // 克隆地址池
        let chain_id = node_state.chain_config.chain_id();

        // 共享的 epoch 变量（供 spawn 任务读取）
        let current_epoch_clone = shared_epoch.clone();
        let current_seq_num_clone = shared_seq_num.clone();

        // 根据操作系统选择 IPC 路径前缀
        let ipc_prefix = if cfg!(target_os = "macos") {
            "/Users/lewis/RustroverProjects/monad-bft-xyz/"
        } else {
            ""  // 默认前缀路径
        };
        let mempool_ipc_path = PathBuf::from(ipc_prefix).join(&node_state.mempool_ipc_path);
        info!(os = std::env::consts::OS, path = %mempool_ipc_path.display(), "IPC path configured");

        tokio::spawn(async move {

            let vote_delay = CHAIN_PARAMS_LATEST.vote_pace;

            // 维护持久 IPC 连接
            let mut ipc_stream: Option<UnixStream> = None;

            // 地址使用索引，循环使用地址池
            let mut address_index: usize = 0;

            loop {
                // 封装每次迭代为独立 async 块，便于捕获异常
                let iteration = async {
                    // 停顿个出块事件再继续构造数据
                    tokio::time::sleep(vote_delay).await;

                    // 从共享变量拿到最新的 epoch
                    let current_epoch = current_epoch_clone.load(Ordering::SeqCst);
                    let current_seq_num = current_seq_num_clone.load(Ordering::SeqCst);

                    // 检查并重建连接（如果断开或不存在）
                    if ipc_stream.is_none() {
                        match UnixStream::connect(&mempool_ipc_path).await {
                            Ok(stream) => {
                                ipc_stream = Some(stream);
                                info!(path = %mempool_ipc_path.display(), "IPC connection established");
                            }
                            Err(e) => {
                                error!(path = %mempool_ipc_path.display(), error = %e, "Failed to connect to IPC");
                                return Ok::<(), Box<dyn std::error::Error + Send + Sync>>(());
                            }
                        }
                    }

                    // 使用 BufWriter 优化写入性能
                    let mut writer = BufWriter::new(ipc_stream.as_mut().unwrap());

                    use alloy_eips::eip2718::Encodable2718;

                    let strategy = get_tx_strategy(current_epoch);
                    if !strategy.enabled {
                        info!(epoch = current_epoch, "Fill disabled for this epoch");
                        return Ok(());
                    }

                    let num_txs = strategy.num_txs;
                    if num_txs > addresses_clone.len() {
                        error!("====== enlarge address pool - num_txs > addresses_clone.len(), num_txs: {}, addresses_clone.len: {}", num_txs, addresses_clone.len());
                        return Ok(());
                    }

                    let gas_limit = strategy.gas_limit;
                    let input_len = strategy.input_len;
                    let gas_price: u128 = 100_000_000_000u128.into();

                    let now = Instant::now();

                    // 批量收集交易数据
                    let mut batch_data = Vec::with_capacity(num_txs);

                    for _ in 0..num_txs {

                        // 使用当前索引的地址
                        let (_address, secret) = &addresses_clone[address_index];

                        // 检查是否需要重置索引（地址池一轮使用完毕）
                        if address_index + 1 >= addresses_clone.len() {
                            address_index = 0;
                            info!(address_index, total_addresses = addresses_clone.len(), "====== Address index reset");
                        } else {
                            address_index += 1;
                        }

                        // 从 secret 创建 KeyPair 以获取 pubkey
                        let mut secret_bytes = secret.as_slice().to_vec();
                        let kp = match monad_secp::KeyPair::from_bytes(&mut secret_bytes) {
                            Ok(kp) => kp,
                            Err(e) => {
                                error!(error = %e, "Failed to create KeyPair from secret");
                                return Ok(());
                            }
                        };
                        let _sender = NodeId::new(kp.pubkey());

                        // 构造普通交易
                        let tx = make_legacy_tx_with_chain_id(
                            secret.clone(),
                            gas_price,
                            gas_limit,
                            0, // ignore 
                            input_len,
                            chain_id,
                        );

                        let mut encoded = Vec::new();
                        tx.encode_2718(&mut encoded);

                        // 收集到批量缓冲区
                        let len = encoded.len() as u32;
                        batch_data.extend_from_slice(&len.to_be_bytes());
                        batch_data.extend_from_slice(&encoded);
                    }

                    // 使用 BufWriter 批量发送
                    if !batch_data.is_empty() {
                        if let Err(e) = writer.write_all(&batch_data).await {
                            error!(error = %e, num_txs, "IPC batch write failed, will reconnect next tick");
                            ipc_stream = None;
                        } else if let Err(e) = writer.flush().await {
                            error!(error = %e, num_txs, "IPC batch flush failed, will reconnect next tick");
                            ipc_stream = None;
                        }
                    }

                    info!(strategy = %strategy.description, epoch = current_epoch, seq_num = current_seq_num, num_txs, address_index, elapsed = now.elapsed().as_millis(), "====== Continuous fill txs done in background");

                    Ok(())
                };

                // 捕获未知异常，崩溃后自动重启
                let iteration_future = std::panic::AssertUnwindSafe(iteration);
                match iteration_future.catch_unwind().await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        error!(error = %e, "Iteration returned error, continuing...");
                    }
                    Err(panic_info) => {
                        if let Some(s) = panic_info.downcast_ref::<&str>() {
                            error!(panic = %s, "Continuous fill loop panicked");
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            error!(panic = %s, "Continuous fill loop panicked");
                        } else {
                            error!(panic = ?panic_info, "Continuous fill loop panicked with unknown error");
                        }
                        // 重置状态后重启
                        ipc_stream = None;
                        address_index = 0;
                        warn!("Restarting continuous fill loop...");
                    }
                }
            }
        });
    } else {
        info!("Continuous fill disabled");
    }

    let mut sigterm = signal(SignalKind::terminate()).expect("in tokio rt");
    let mut sigint = signal(SignalKind::interrupt()).expect("in tokio rt");

    loop {
        tokio::select! {
            biased; // events are in order of priority

            result = sigterm.recv() => {
                info!(?result, "received SIGTERM, exiting...");
                break;
            }
            result = sigint.recv() => {
                info!(?result, "received SIGINT, exiting...");
                break;
            }
            _ = match &mut maybe_metrics_ticker {
                Some(ticker) => ticker.tick().boxed(),
                None => futures_util::future::pending().boxed(),
            } => {
                let otel_meter = maybe_otel_meter.as_ref().expect("otel_endpoint must have been set");
                let state_metrics = state.metrics();
                let executor_metrics = executor.metrics();
                send_metrics(otel_meter, &mut gauge_cache, state_metrics, executor_metrics, &process_start, &total_state_update_elapsed);
            }
            event = executor.next().instrument(ledger_span.clone()) => {
                let Some(inner_event) = event else {
                    event!(Level::ERROR, "parent executor returned none!");
                    return Err(());
                };

                // 从事件中提取 epoch 和 seq_num（用于持续填充策略选择）
                let (event_epoch, event_seq_num) = match &inner_event {
                    MonadEvent::MempoolEvent(mempool_event) => {
                        match mempool_event {
                            monad_executor_glue::MempoolEvent::Proposal { epoch, seq_num, .. } => {
                                (Some(epoch.0), Some(seq_num.0))
                            }
                            _ => (None, None),
                        }
                    }
                    MonadEvent::ValidatorEvent(validator_event) => {
                        match validator_event {
                            monad_executor_glue::ValidatorEvent::UpdateValidators(vset) => {
                                (Some(vset.epoch.0), None)
                            }
                        }
                    }
                    _ => (None, None),
                };

                let current_epoch = shared_epoch.load(Ordering::SeqCst);
                let current_seq_num = shared_seq_num.load(Ordering::SeqCst);
                if enable_continuous_fill  {
                    if event_epoch.is_some() && event_epoch.unwrap() > current_epoch {
                        let new_epoch = event_epoch.unwrap();
                        shared_epoch.store(new_epoch, Ordering::SeqCst);  // 同步到共享变量
                        info!("============ epoch detected - new: {}, old: {}", new_epoch, current_epoch);
                    }
                    if event_seq_num.is_some() && event_seq_num.unwrap() > current_seq_num {
                        let new_seq_num = event_seq_num.unwrap();
                        shared_seq_num.store(new_seq_num, Ordering::SeqCst);  // 同步到共享变量
                        // info!("============ seq_num detected - new: {}, old: {}", new_seq_num, current_seq_num);
                    }
                }

                // 包装成 LogFriendlyMonadEvent 用于日志
                let event = LogFriendlyMonadEvent {
                    timestamp: Utc::now(),
                    event: inner_event,
                };

                let event_debug = {
                    let _timer = DropTimer::start(Duration::from_millis(1), |elapsed| {
                        warn!(
                            ?elapsed,
                            ?event,
                            "long time to format event"
                        )
                    });
                    format!("{:?}", event)
                };

                {
                    let _ledger_span = ledger_span.enter();
                    let _wal_event_span = tracing::trace_span!("wal_event_span").entered();
                    if let Err(err) = wal.push(&event) {
                        event!(Level::ERROR, ?err, "failed to push to wal",);
                        return Err(());
                    }
                };

                let commands = {
                    let _timer = DropTimer::start(Duration::from_millis(50), |elapsed| {
                        warn!(
                            ?elapsed,
                            event =? event_debug,
                            "long time to update event"
                        )
                    });
                    let _ledger_span = ledger_span.enter();
                    let _event_span = tracing::trace_span!("event_span", ?event.event).entered();
                    let start = Instant::now();
                    let cmds = state.update(event.event);
                    total_state_update_elapsed += start.elapsed();
                    cmds
                };

                if !commands.is_empty() {
                    let num_commands = commands.len();
                    let _timer = DropTimer::start(Duration::from_millis(50), |elapsed| {
                        warn!(
                            ?elapsed,
                            event =? event_debug,
                            num_commands,
                            "long time to execute commands"
                        )
                    });
                    let _ledger_span = ledger_span.enter();
                    let _exec_span = tracing::trace_span!("exec_span", num_commands).entered();
                    executor.exec(commands);
                }

                if let Some(ledger_tip) = executor.ledger.last_commit() {
                    if last_ledger_tip.is_none_or(|last_ledger_tip| ledger_tip > last_ledger_tip) {
                        last_ledger_tip = Some(ledger_tip);
                        ledger_span = tracing::info_span!("ledger_span", last_ledger_tip = last_ledger_tip.map(|s| s.as_u64()));
                    }
                }
            }
        }
    }

    Ok(())
}

fn build_raptorcast_router<ST, SCT, M, OM>(
    node_config: NodeConfig<ST>,
    peer_discovery_config: PeerDiscoveryConfig<ST>,
    identity: ST::KeyPairType,
    bootstrap_nodes: NodeBootstrapConfig<ST>,
    full_nodes: &[FullNodeIdentityConfig<CertificateSignaturePubKey<ST>>],
    locked_epoch_validators: Vec<ValidatorSetDataWithEpoch<SCT>>,
    current_epoch: Epoch,
    current_round: Round,
) -> MultiRouter<
    ST,
    M,
    OM,
    MonadEvent<ST, SCT, ExecutionProtocolType>,
    PeerDiscovery<ST>,
    monad_raptorcast::auth::WireAuthProtocol,
>
where
    ST: CertificateSignatureRecoverable<KeyPairType = monad_secp::KeyPair>,
    SCT: SignatureCollection<NodeIdPubKey = CertificateSignaturePubKey<ST>>,
    M: Message<NodeIdPubKey = CertificateSignaturePubKey<ST>>
        + Decodable
        + From<OM>
        + Send
        + Sync
        + 'static,
    OM: Encodable + Clone + Send + Sync + 'static,
{
    let bind_address = SocketAddr::new(
        IpAddr::V4(node_config.network.bind_address_host),
        node_config.network.bind_address_port,
    );
    let authenticated_bind_address = node_config
        .network
        .authenticated_bind_address_port
        .map(|port| SocketAddr::new(IpAddr::V4(node_config.network.bind_address_host), port));
    let Some(SocketAddr::V4(name_record_address)) = resolve_domain_v4(
        &NodeId::new(identity.pubkey()),
        &peer_discovery_config.self_address,
    ) else {
        panic!(
            "Unable to resolve self address: {:?}",
            peer_discovery_config.self_address
        );
    };

    tracing::debug!(
        ?bind_address,
        ?authenticated_bind_address,
        ?name_record_address,
        "Monad-node starting, pid: {}",
        process::id()
    );

    let network_config = node_config.network;

    let mut dp_builder = DataplaneBuilder::new(&bind_address, network_config.max_mbps.into());
    if let Some(buffer_size) = network_config.buffer_size {
        dp_builder = dp_builder.with_udp_buffer_size(buffer_size);
    }
    dp_builder = dp_builder
        .with_tcp_connections_limit(
            network_config.tcp_connections_limit,
            network_config.tcp_per_ip_connections_limit,
        )
        .with_tcp_rps_burst(
            network_config.tcp_rate_limit_rps,
            network_config.tcp_rate_limit_burst,
        );

    let mut udp_sockets = vec![monad_dataplane::UdpSocketConfig {
        socket_addr: bind_address,
        label: RAPTORCAST_SOCKET.to_string(),
    }];
    if let Some(auth_addr) = authenticated_bind_address {
        udp_sockets.push(monad_dataplane::UdpSocketConfig {
            socket_addr: auth_addr,
            label: AUTHENTICATED_RAPTORCAST_SOCKET.to_string(),
        });
    }
    dp_builder = dp_builder.extend_udp_sockets(udp_sockets);

    // auth port in peer discovery config and network config should be set and unset simultaneously
    assert_eq!(
        peer_discovery_config.self_auth_port.is_some(),
        network_config.authenticated_bind_address_port.is_some()
    );

    let self_id = NodeId::new(identity.pubkey());
    let self_record = match peer_discovery_config.self_auth_port {
        Some(auth_port) => NameRecord::new_with_authentication(
            *name_record_address.ip(),
            name_record_address.port(),
            name_record_address.port(),
            auth_port,
            peer_discovery_config.self_record_seq_num,
        ),
        None => NameRecord::new(
            *name_record_address.ip(),
            name_record_address.port(),
            peer_discovery_config.self_record_seq_num,
        ),
    };
    let self_record = MonadNameRecord::new(self_record, &identity);
    info!("MonadNameRecord is {:?}", self_record.signature);
    assert!(
        self_record.signature == peer_discovery_config.self_name_record_sig,
        "self name record signature mismatch"
    );

    // initial set of peers
    let bootstrap_peers: BTreeMap<_, _> = bootstrap_nodes
        .peers
        .iter()
        .filter_map(|peer| {
            let node_id = NodeId::new(peer.secp256k1_pubkey);
            if node_id == self_id {
                return None;
            }
            let address = match resolve_domain_v4(&node_id, &peer.address) {
                Some(SocketAddr::V4(addr)) => addr,
                _ => {
                    warn!(?node_id, ?peer.address, "Unable to resolve");
                    return None;
                }
            };

            let peer_entry = monad_executor_glue::PeerEntry {
                pubkey: peer.secp256k1_pubkey,
                addr: address,
                signature: peer.name_record_sig,
                record_seq_num: peer.record_seq_num,
                auth_port: peer.auth_port,
            };

            match MonadNameRecord::try_from(&peer_entry) {
                Ok(monad_name_record) => Some((node_id, monad_name_record)),
                Err(_) => {
                    warn!(?node_id, "invalid name record signature in config file");
                    None
                }
            }
        })
        .collect();

    let epoch_validators: BTreeMap<Epoch, BTreeSet<NodeId<CertificateSignaturePubKey<ST>>>> =
        locked_epoch_validators
            .iter()
            .map(|epoch_validators| {
                (
                    epoch_validators.epoch,
                    epoch_validators
                        .validators
                        .0
                        .iter()
                        .map(|validator| validator.node_id)
                        .collect(),
                )
            })
            .collect();
    let prioritized_full_nodes: BTreeSet<_> = node_config
        .fullnode_raptorcast
        .full_nodes_prioritized
        .identities
        .iter()
        .map(|id| NodeId::new(id.secp256k1_pubkey))
        .collect();
    let pinned_full_nodes: BTreeSet<_> = full_nodes
        .iter()
        .map(|full_node| NodeId::new(full_node.secp256k1_pubkey))
        .chain(prioritized_full_nodes.clone())
        .chain(bootstrap_peers.keys().cloned())
        .collect();

    let peer_discovery_builder = PeerDiscoveryBuilder {
        self_id,
        self_record,
        current_round,
        current_epoch,
        epoch_validators: epoch_validators.clone(),
        pinned_full_nodes,
        prioritized_full_nodes,
        bootstrap_peers,
        refresh_period: Duration::from_secs(peer_discovery_config.refresh_period),
        request_timeout: Duration::from_secs(peer_discovery_config.request_timeout),
        unresponsive_prune_threshold: peer_discovery_config.unresponsive_prune_threshold,
        last_participation_prune_threshold: peer_discovery_config
            .last_participation_prune_threshold,
        min_num_peers: peer_discovery_config.min_num_peers,
        max_num_peers: peer_discovery_config.max_num_peers,
        max_group_size: node_config.fullnode_raptorcast.max_group_size,
        enable_publisher: node_config.fullnode_raptorcast.enable_publisher,
        enable_client: node_config.fullnode_raptorcast.enable_client,
        rng: ChaCha8Rng::from_entropy(),
    };

    let shared_key = Arc::new(identity);
    let wireauth_config = monad_wireauth::Config::default();
    let auth_protocol =
        monad_raptorcast::auth::WireAuthProtocol::new(wireauth_config, shared_key.clone());

    MultiRouter::new(
        self_id,
        RaptorCastConfig {
            shared_key,
            mtu: network_config.mtu,
            udp_message_max_age_ms: network_config.udp_message_max_age_ms,
            primary_instance: RaptorCastConfigPrimary {
                raptor10_redundancy: 2.5f32,
                fullnode_dedicated: full_nodes
                    .iter()
                    .map(|full_node| NodeId::new(full_node.secp256k1_pubkey))
                    .collect(),
            },
            secondary_instance: node_config.fullnode_raptorcast,
        },
        dp_builder,
        peer_discovery_builder,
        current_epoch,
        epoch_validators,
        auth_protocol,
    )
}

fn resolve_domain_v4<P: PubKey>(node_id: &NodeId<P>, domain: &String) -> Option<SocketAddr> {
    let resolved = match domain.to_socket_addrs() {
        Ok(resolved) => resolved,
        Err(err) => {
            warn!(?node_id, ?domain, ?err, "Unable to resolve");
            return None;
        }
    };

    for entry in resolved {
        match entry {
            SocketAddr::V4(_) => return Some(entry),
            SocketAddr::V6(_) => continue,
        }
    }

    warn!(?node_id, ?domain, "No IPv4 DNS record");
    None
}

/// A dummy executor that ignores all StateSync commands
/// Used when StateSync is disabled to satisfy trait bounds
pub struct DummyStateSyncExecutor<ST, EPT> {
    _phantom: std::marker::PhantomData<(ST, EPT)>,
}

impl<ST, EPT> Default for DummyStateSyncExecutor<ST, EPT> {
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<ST, EPT> monad_executor::Executor for DummyStateSyncExecutor<ST, EPT>
where
    ST: monad_crypto::certificate_signature::CertificateSignatureRecoverable,
    EPT: monad_types::ExecutionProtocol,
{
    type Command = StateSyncCommand<ST, EPT>;

    fn exec(&mut self, _commands: Vec<Self::Command>) {
        // Ignore all commands - StateSync is disabled
    }

    fn metrics(&self) -> monad_executor::ExecutorMetricsChain {
        monad_executor::ExecutorMetricsChain::default()
    }
}

impl<ST, EPT> futures::Stream for DummyStateSyncExecutor<ST, EPT>
where
    ST: monad_crypto::certificate_signature::CertificateSignatureRecoverable,
    EPT: monad_types::ExecutionProtocol,
{
    type Item = monad_executor_glue::MonadEvent<ST, monad_bls::BlsSignatureCollection<monad_crypto::certificate_signature::CertificateSignaturePubKey<ST>>, EPT>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // Never produce any events - StateSync is disabled
        std::task::Poll::Pending
    }
}

fn build_otel_meter_provider(
    otel_endpoint: &str,
    service_name: String,
    network_name: String,
    interval: Duration,
) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, NodeSetupError> {
    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_timeout(interval * 2)
        .with_endpoint(otel_endpoint)
        .build()?;

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(interval / 2)
        .build();

    let mut attrs = vec![
        opentelemetry::KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            service_name,
        ),
        opentelemetry::KeyValue::new("network", network_name),
    ];
    if let Some(version) = MONAD_NODE_VERSION {
        attrs.push(opentelemetry::KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
            version,
        ));
    }

    let provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(
            opentelemetry_sdk::Resource::builder_empty()
                .with_attributes(attrs)
                .build(),
        );

    Ok(provider_builder.build())
}

const GAUGE_TOTAL_UPTIME_US: &str = "monad.total_uptime_us";
const GAUGE_STATE_TOTAL_UPDATE_US: &str = "monad.state.total_update_us";
const GAUGE_NODE_INFO: &str = "monad_node_info";

fn send_metrics(
    meter: &opentelemetry::metrics::Meter,
    gauge_cache: &mut HashMap<&'static str, opentelemetry::metrics::Gauge<u64>>,
    state_metrics: &Metrics,
    executor_metrics: ExecutorMetricsChain,
    process_start: &Instant,
    total_state_update_elapsed: &Duration,
) {
    let node_info_gauge = gauge_cache
        .entry(GAUGE_NODE_INFO)
        .or_insert_with(|| meter.u64_gauge(GAUGE_NODE_INFO).build());
    node_info_gauge.record(1, &[]);

    for (k, v) in state_metrics
        .metrics()
        .into_iter()
        .chain(executor_metrics.into_inner())
        .chain(std::iter::once((
            GAUGE_TOTAL_UPTIME_US,
            process_start.elapsed().as_micros() as u64,
        )))
        .chain(std::iter::once((
            GAUGE_STATE_TOTAL_UPDATE_US,
            total_state_update_elapsed.as_micros() as u64,
        )))
    {
        let gauge = gauge_cache
            .entry(k)
            .or_insert_with(|| meter.u64_gauge(k).build());
        gauge.record(v, &[]);
    }
}

// 基础私钥（用于派生）
const BASE_SECRET: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn generate_secrets(address_count: usize) -> Vec<(alloy_primitives::Address, B256)> {

    // 开始计时
    let start_time = std::time::Instant::now();

    // 使用基础私钥的低 30 字节与索引组合，生成确定性派生密钥
    let base_bytes: [u8; 32] = hex::decode(BASE_SECRET).unwrap().try_into().unwrap();

    // 并发生成所有密钥和地址
    let tasks: Vec<_> = (0..address_count)
        .map(|i| {
            tokio::task::spawn_blocking(move || {
                // 派生: 修改基础私钥的最后 2 字节（与大端序索引异或）
                let mut derived = base_bytes;
                let idx_bytes = (i as u16).to_be_bytes();
                derived[30] ^= idx_bytes[0];
                derived[31] ^= idx_bytes[1];

                let derived_secret = B256::from(derived);
                let sender = secret_to_eth_address(derived_secret);

                // println!("   [{}] 0x{:?}", i, sender);

                (sender, derived_secret)
            })
        })
        .collect();

    // 等待所有任务完成并收集结果
    let results: Vec<(alloy_primitives::Address, B256)> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();

    // 结束计时并打印耗时
    let elapsed = start_time.elapsed();
    let rate = address_count as f64 / elapsed.as_secs_f64();

    println!("🔐 生成 {}/{} 个发送方地址: 耗时 {:.3}s ({:.0} addr/s)", results.len(), address_count, elapsed.as_secs_f64(), rate);

    results
}

fn secret_to_eth_address(secret: B256) -> alloy_primitives::Address {
    let mut secret_bytes = secret.as_slice().to_vec();
    let kp = monad_secp::KeyPair::from_bytes(&mut secret_bytes).unwrap();
    let pubkey_bytes = kp.pubkey().bytes();
    assert!(pubkey_bytes.len() == 65);
    let hash = alloy_primitives::keccak256(&pubkey_bytes[1..]);
    alloy_primitives::Address::from_slice(&hash[12..])
}

/// 交易策略参数
#[derive(Clone, Debug)]
struct TxStrategy {
    /// 是否启用交易发送
    enabled: bool,
    /// 每个区块填充的交易数量
    num_txs: usize,
    /// 输入数据长度（字节）
    input_len: usize,
    /// Gas limit
    gas_limit: u64,
    /// 描述
    description: String,
}

/// 根据 epoch 获取交易策略
fn get_tx_strategy(epoch: u64) -> TxStrategy {
    match epoch % 3 {
        0 => TxStrategy {
            enabled: false,
            num_txs: 0,
            input_len: 0,
            gas_limit: 21_000,
            description: "空块模式（epoch % 3 == 0）".to_string(),
        },
        1 => TxStrategy {
            enabled: true,
            num_txs: rand::thread_rng().gen_range(1..=800),
            input_len: 500,
            gas_limit: 50_000,
            description: "低负载模式（epoch % 3 == 1）".to_string(),
        },
        _ => TxStrategy {  // 2
            enabled: true,
            num_txs: rand::thread_rng().gen_range(1000..=5000),
            input_len: 300,
            gas_limit: 100_000,
            description: "高负载模式（epoch % 3 == 2）".to_string(),
        },
    }
}