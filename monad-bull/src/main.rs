use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    process,
    sync::Arc,
    time::{Duration, Instant},
};

use monad_chain_config::ChainConfig;
use monad_consensus_state::ConsensusConfig;
use monad_consensus_types::{validator_data::{ValidatorSetDataWithEpoch, ValidatorsConfig}};
use monad_control_panel::ipc::ControlPanelIpcReceiver;
use monad_crypto::certificate_signature::{
    CertificateSignaturePubKey, CertificateSignatureRecoverable, PubKey,
};

use monad_dataplane::DataplaneBuilder;
use monad_eth_block_policy::EthBlockPolicy;
use monad_eth_block_validator::EthBlockValidator;
use monad_eth_txpool_executor::{EthTxPoolExecutor, EthTxPoolIpcConfig};
use monad_executor::{Executor};
use monad_executor_glue::{LogFriendlyMonadEvent, Message, MonadEvent};
use monad_ledger::MonadBlockFileLedger;
use monad_node_config::{
    ExecutionProtocolType, FullNodeIdentityConfig, NodeBootstrapConfig, NodeConfig,
    PeerDiscoveryConfig, SignatureCollectionType, SignatureType,
};
use monad_peer_discovery::{
    discovery::{PeerDiscovery, PeerDiscoveryBuilder},
    MonadNameRecord, NameRecord,
};

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
use futures_util::StreamExt;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, event, info, warn, Instrument, Level};
use alloy_rlp::{Decodable, Encodable};
use chrono::Utc;
use rand_chacha::{rand_core::SeedableRng, ChaCha8Rng};

use self::{cli::Cli, error::NodeSetupError, state::NodeState};

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
        .enable_all()                                           // 启用所有 I/O 驱动和时间驱动
        .build()                                               // 构建运行时实例
        .map_err(Into::into)                                   // 将构建错误转换为 NodeSetupError
        .unwrap_or_else(|e: NodeSetupError|                    // 如果构建失败，使用 clap 错误处理退出
            cmd.error(e.kind(), e).exit()
        );

    drop(cmd);

    MONAD_NODE_VERSION.map(|v| info!("starting monad-bft with version {}", v));

    if let Err(e) = runtime.block_on(run(node_state)) {
        error!("monad consensus node crashed: {:?}", e);
    }
}

async fn run(node_state: NodeState) -> Result<(), ()> {
    let locked_epoch_validators = node_state
        .validators_config
        .get_locked_validator_sets(&node_state.forkpoint_config);

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
                bind_path: node_state.mempool_ipc_path,
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

    let mut total_state_update_elapsed = Duration::ZERO;

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
            event = executor.next().instrument(ledger_span.clone()) => {
                let Some(event) = event else {
                    event!(Level::ERROR, "parent executor returned none!");
                    return Err(());
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

                let event = LogFriendlyMonadEvent {
                    timestamp: Utc::now(),
                    event,
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