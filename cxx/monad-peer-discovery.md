# Monad节点发现模块(monad-peer-discovery)

## 概述

`monad-peer-discovery` 是Monad网络中的点对点发现模块, 其核心功能包括:

1.节点间发现与连接管理
2.网络拓扑维护
3.Validator / FullNode角色协调
4.Raporcast二级广播的连接建立

#### 节点角色

节点角色枚举

```rust
pub enum PeerDiscoveryRole {
    ValidatorNone,      // Validator，不参与二级广播
    ValidatorPublisher, // Validator，参与二级广播作为 Publisher
    FullNodeNone,       // FullNode，不参与二级广播
    FullNodeClient,     // FullNode，参与二级广播作为 Client
}
```

#### 关键配置

在`node.toml`中的配置关键项
```
[bootstrap]
[[bootstrap.peers]]  初始引导节点

[peer_discovery]
self_address = "192.168.2.102:8000"   本节点对外IP+Port
self_record_seq_num = 0               节点配置序列号
self_name_record_sig = "a8e989b4f0"   记录签名
refresh_period = 30                   刷新周期
request_timeout = 5                   超时时间
unresponsive_prune_threshold = 5      未响应次数阈值
last_participation_prune_threshold = 5000    未参与二级广播round阈值
min_num_peers = 0                     最小peer数量
max_num_peers = 200                   最大peer数量

[fullnode_dedicated]
[[fullnode_dedicated.identities]]  初始固定节点

[fullnode_raptorcast]
enable_publisher = false    如果本节点是验证者,决定是ValidatorNone还是ValidadorPublisher
enable_client = false       如果本节点是FullNode, 决定是FullNodeNonde还是FullNodeClient

[[fullnode_raptorcast.full_nodes_prioritized]] 初始优先节点

[network]
bind_address_host = "0.0.0.0"
bind_address_port = 8000
```

## 启动流程

节点启动时,本节点的配置参数从`node.toml`配置文件加载

### 初始化加载流程
```
main()
│
└─ run()
   │
   └─ build_raptorcast_router(node_state)
      │
      └─ MultiRouter::new(node_config, builder)
         │
         └─ PeerDiscoveryDriver::new(builder)
            │
            └─ PeerDiscoveryBuilder::build()
               │
               └─ PeerDiscovery
```
#### 构建PeerDiscoveryBuilder

在`build_raptorcast_router`函数中构建`PeerDiscoveryBuilder`

```rust
let peer_discovery_builder = PeerDiscoveryBuilder {
        self_id,        // 本地节点id
        self_record,    // 节点的NameRecord
        current_round,   // 当前共识round
        current_epoch,   // 当前共识epoch
        epoch_validators: epoch_validators.clone(),    // epoch对应的验证者集
        pinned_full_nodes,    // 固定全节点集
        prioritized_full_nodes,    // 优先全节点集
        bootstrap_peers,    // 引导节点集
        refresh_period: Duration::from_secs(peer_discovery_config.refresh_period),    // 刷新周期
        request_timeout: Duration::from_secs(peer_discovery_config.request_timeout),    // 请求超时时间
        unresponsive_prune_threshold: peer_discovery_config.unresponsive_prune_threshold,    // 无响应剔除阈值
        last_participation_prune_threshold: peer_discovery_config.last_participation_prune_threshold,
        min_num_peers: peer_discovery_config.min_num_peers,    // 最小节点数
        max_num_peers: peer_discovery_config.max_num_peers,    // 最大节点数
        max_group_size: node_config.fullnode_raptorcast.max_group_size,    // 在raptorcast group的最大节点数
        enable_publisher: node_config.fullnode_raptorcast.enable_publisher, // 是否开启二级广播, 如果是验证者
        enable_client: node_config.fullnode_raptorcast.enable_client,    // 是否开启二级广播, 如果是非验证者
        rng: ChaCha8Rng::from_entropy(),    // 随机数
    }
```
#### 构建PeerDiscovery

由`PeerDiscoveryBuilder`的`build`初始化`PeerDiscovery`并尝试与引导节点建立连接

这时`node.toml`的配置全部加载进来,参数生效:
1. 判定本节点的初始角色Role
2. PeerDicovery的刷新时间周期
3. 最小最大对等节点数量
4. 本节点的优先全节点列表(信任级别高于固定全节点) prioritized_full_nodes
5. 本节点的初始固定全节点列表 pinned_full_nodes(fullnode_dedicated)

```rust
impl<ST: CertificateSignatureRecoverable> PeerDiscoveryAlgoBuilder for PeerDiscoveryBuilder<ST> {

    type PeerDiscoveryAlgoType = PeerDiscovery<ST>;

    fn build(self,) -> (Self::PeerDiscoveryAlgoType,
        Vec<PeerDiscoveryCommand<<Self::PeerDiscoveryAlgoType as PeerDiscoveryAlgo>::SignatureType>,>,) {
        debug!("initializing peer discovery");
        assert!(self.max_num_peers > self.min_num_peers);

        let is_current_epoch_validator = self
            .epoch_validators
            .get(&self.current_epoch)
            .is_some_and(|validators| validators.contains(&self.self_id));
        // 从加载的validators + node.toml的配置中判定节点角色
        let self_role = match (is_current_epoch_validator, self.enable_publisher,self.enable_client,)
         {
            // 1.是Validator, enable_publisher=Ture, role = ValidatorPublisher
            (true, true, _) => {
                info!("Starting peer discovery as ValidatorPublisher");
                PeerDiscoveryRole::ValidatorPublisher
            }
            // 2.是Validator, enable_publisher=False, role = ValidatorNone
            (true, false, _) => {
                info!("Starting peer discovery as ValidatorNone");
                PeerDiscoveryRole::ValidatorNone
            }
            // 3.不是Validator, enable_client=True, role = FullNodeClient
            (false, _, true) => {
                info!("Starting peer discovery as FullNodeClient");
                PeerDiscoveryRole::FullNodeClient
            }
            // 4.不是Validator, enable_client=False, role = FullNodeNone
            (false, _, false) => {
                info!("Starting peer discovery as FullNodeNone");
                PeerDiscoveryRole::FullNodeNone
            }
        };

        let mut state = PeerDiscovery {
            self_id: self.self_id,
            self_record: self.self_record,
            self_role,    // 当前节点角色
            current_round: self.current_round,
            current_epoch: self.current_epoch,
            epoch_validators: self.epoch_validators,
            initial_bootstrap_peers: self      // 初始化引导节点
                .bootstrap_peers
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            prioritized_full_nodes: self.prioritized_full_nodes,   // 优先节点集
            pinned_full_nodes: self.pinned_full_nodes,             // 固定节点集
            routing_info: Default::default(),    // 路由表
            participation_info: Default::default(),    // raptorcast参与状态信息
            pending_queue: Default::default(),    // 待确认的节点（等待 pong）
            outstanding_lookup_requests: Default::default(),    // 待响应的查找请求
            metrics: Default::default(),
            refresh_period: self.refresh_period,
            request_timeout: self.request_timeout,
            unresponsive_prune_threshold: self.unresponsive_prune_threshold,
            last_participation_prune_threshold: self.last_participation_prune_threshold,
            min_num_peers: self.min_num_peers,
            max_num_peers: self.max_num_peers,
            max_group_size: self.max_group_size,
            enable_publisher: self.enable_publisher,
            enable_client: self.enable_client,
            rng: self.rng,
        };

        let mut cmds = Vec::new();
        // 把引导节点放入pending队列中并调用send_ping()发起连接请求,并开始refresh()刷新路由表
        self.bootstrap_peers
            .into_iter()
            .for_each(|(peer_id, name_record)| {
                if let Ok(cmds_from_insert) = state.insert_peer_to_pending(peer_id, name_record) {
                    cmds.extend(cmds_from_insert);
                }
            });

        cmds.extend(state.refresh());

        (state, cmds)
    }
}
```

## 定期刷新

通过引导节点完成初始化启动后, 后续定期刷新维护本地的路由表以及节点其他相关状态更新.

### 周期性刷新流程

核心方法: `PeerDiscovery.refresh(self)`

```
├─ [定时器触发]
│  └─ refresh_timer 倒计时归零
│     └─ 触发 refresh() 函数
│
├─ [阶段1: 清理不活跃节点]
│  ├─ 根据participation_info判断节点是否触达未参与阈值
│  └─ 执行清理策略
│     ├─ 固定节点
│     │  └─ 只清除状态，不移除
│     │
│     └─ 普通节点
│        └─ 完全移除
│
├─ [阶段2: 超过节点上限时剔除]
│  ├─ 检查连接数是否超过 max_num_peers
│  └─ 随机剔除
│     ├─ 跳过固定节点
│     └─ 跳过验证者
│
├─ [阶段3: 发现缺失验证者(当前epoch+下一个epoch)]
│  ├─ 获取当前轮验证者集合
│  ├─ 获取下一轮验证者集合
│  └─ 筛选出 routing_info 中不存在的验证者
│
├─ [阶段4: 选择发现策略]
│  │
│  ├─ 策略A: 节点数 < min_num_peers
│  │  └─ 开放发现模式
│  │     ├─ 查找缺失验证者
│  │     └─ 获取所有已知节点
│  │
│  └─ 策略B: 节点数 >= min_num_peers 且存在缺失验证者
│     └─ 定向查找模式
│        └─ 只查找缺失的特定验证者,不获取其他额外节点
│
├─ [阶段5: 全节点开放发现]
│  ├─ 触发条件: FullNodeNone / FullNodeClient
│  └─ 随机选择一个验证者发起开放发现
│
├─ [阶段6: 角色特定操作]
│  │
│  ├─ FullNodeClient
│  │  └─ look_for_upstream_validators()
│  │     ├─ 尝试连接上游验证者
│  │     └─ 建立 Raptorcast 通道
│  │
│  └─ ValidatorPublisher
│     └─ 收集 Connected 状态的全节点
│        └─ 更新下游全节点数监控指标
│
├─ [阶段7: 更新指标 + 重置]
│  ├─ 导出 peer 数量指标
│  ├─ 导出 pending 数量指标
│  └─ 重置 refresh_timer
│
└─ [结束]
   └─ 返回待执行的命令列表
```
#### 请求目标选择(适配上面阶段4 阶段5)

作用: 选择发送 PeerLookupRequest 的目标节点，用于发现更多对等节点, 根据节点角色适配不同的策略

| 节点角色 | 数据来源 | 选择数量 | 优先级 |
|---------|---------|---------|--------|
| ValidatorNone / ValidatorPublisher | routing_info (任意节点) | 3 个 | - |
| FullNodeClient | participation_info → routing_info | 3 个 | Connected 优先，不足时从 routing_info 补充 |
| FullNodeNone | initial_bootstrap_peers | 3 个 | 仅引导节点 |

Refresh方法详解
```rust
fn refresh(&mut self) -> Vec<PeerDiscoveryCommand<ST>> {
    debug!("refreshing peer discovery");
    self.metrics[GAUGE_PEER_DISC_REFRESH] += 1;
    let mut cmds = Vec::new();

    // ==================== 阶段1：清理不活跃节点 ====================
    // 移除参与信息超过 last_participation_prune_threshold 轮未更新的节点
    let non_participating_nodes: Vec<_> = self
        .participation_info
        .iter()
        .filter_map(|(node_id, info)| {
            // 计算：当前轮次 - 最后活跃轮次 >= 剔除阈值
            if self.current_round.max(info.last_active) - info.last_active
                >= self.last_participation_prune_threshold
            {
                Some(*node_id)
            } else {
                None
            }
        })
        .collect();

    for node_id in non_participating_nodes {
        if self.is_pinned_node(&node_id) {
            // 固定节点：只清除参与状态，不移除
            debug!(?node_id, "clearing participation info for pinned node");
            if let Some(info) = self.participation_info.get_mut(&node_id) {
                info.status = SecondaryRaptorcastConnectionStatus::None;
            }
        } else {
            // 非固定节点：完全移除
            debug!(?node_id, "removing non-participating peer");
            self.participation_info.remove(&node_id);
            self.routing_info.remove(&node_id);
        }
    }

    // ==================== 阶段2：超过上限时随机剔除节点 ====================
    if self.routing_info.len() > self.max_num_peers {
        let num_to_prune = self.routing_info.len() - self.max_num_peers;
        let excessive_full_nodes: Vec<_> = self
            .routing_info
            .keys()
            .filter(|node| !self.is_pinned_node(node))  // 跳过验证者和固定节点
            .cloned()
            .collect();

        let nodes_to_prune: Vec<_> = excessive_full_nodes
            .into_iter()
            .choose_multiple(&mut self.rng, num_to_prune);  // 随机选择要剔除的节点

        if nodes_to_prune.is_empty() {
            info!("more validators and pinned full nodes than max number of peers");
        } else {
            for node_id in nodes_to_prune {
                debug!(?node_id, "pruning excessive full nodes");
                self.participation_info.remove(&node_id);
                self.routing_info.remove(&node_id);
            }
        }
    }

    // ==================== 阶段3：发现缺失的验证者 ====================
    // 获取当前轮和下一轮中尚未在 routing_info 中的验证者
    let missing_validators = self
        .epoch_validators
        .get(&self.current_epoch)
        .into_iter()
        .flatten()
        .chain(
            self.epoch_validators
                .get(&(self.current_epoch + Epoch(1)))
                .into_iter()
                .flatten(),
        )
        .filter(|validator| {
            !self.routing_info.contains_key(validator) && *validator != &self.self_id
        })
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .choose_multiple(&mut self.rng, NUM_LOOKUP_PEERS);

    // 选择用于发起查找的节点
    let chosen_peers = self.select_peers_to_lookup_from();

    // ==================== 阶段4：根据节点数决定发现策略 ====================
    if self.routing_info.len() < self.min_num_peers {
        // 情况A：节点数过少 → 开放发现（同时查找多个目标）
        debug!(?chosen_peers, "discover more peers");
        for (validator_id, peer) in missing_validators.iter().zip(chosen_peers.iter()) {
            cmds.extend(self.send_peer_lookup_request(*peer, *validator_id, true));
        }
    } else if !missing_validators.is_empty() {
        // 情况B：节点数足够 → 定向查找（只查找缺失的验证者）
        for (validator_id, peer) in missing_validators.iter().zip(chosen_peers.iter()) {
            debug!(
                ?validator_id,
                "sending targeted peer lookup for missing validator"
            );
            cmds.extend(self.send_peer_lookup_request(*peer, *validator_id, false));
        }
    }

    // ==================== 阶段5：全节点开放发现更新验证者信息 ====================
    if self.self_role == PeerDiscoveryRole::FullNodeNone
        || self.self_role == PeerDiscoveryRole::FullNodeClient
    {
        if let Some(validators) = self.epoch_validators.get(&self.current_epoch) {
            if let Some(peer) = chosen_peers.last() {
                if let Some(random_validator) = validators.iter().choose(&mut self.rng) {
                    trace!(
                        ?random_validator,
                        "full node sending open discovery to random validator"
                    );
                    // open_discovery = true：查找该验证者的同时，获取其已知的所有节点
                    cmds.extend(self.send_peer_lookup_request(*peer, *random_validator, true));
                }
            }
        }
    }

    // ==================== 阶段6：全节点尝试连接上游验证者 ====================
    if self.self_role == PeerDiscoveryRole::FullNodeClient {
        cmds.extend(self.look_for_upstream_validators());
    } else if self.self_role == PeerDiscoveryRole::ValidatorPublisher {
        // 收集下游全节点连接数指标
        let connected_public_full_nodes = self
            .participation_info
            .iter()
            .filter(|(_, info)| info.status == SecondaryRaptorcastConnectionStatus::Connected)
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        self.metrics[GAUGE_PEER_DISC_NUM_DOWNSTREAM_FULLNODES] =
            connected_public_full_nodes.len() as u64;
    }

    // ==================== 阶段7：更新指标并重置定时器 ====================
    self.metrics[GAUGE_PEER_DISC_NUM_PEERS] = self.routing_info.len() as u64;
    self.metrics[GAUGE_PEER_DISC_NUM_PENDING_PEERS] = self.pending_queue.len() as u64;
    
    // 重置刷新定时器，等待下次刷新
    cmds.extend(self.reset_refresh_timer());
    
    // 导出指标
    cmds.push(PeerDiscoveryCommand::MetricsCommand(
        PeerDiscoveryMetricsCommand(self.metrics.clone()),
    ));

    cmds
}
```

### 共识驱动刷新

#### 验证者集变更

触发: 共识模块通知新的验证者集合`RouterCommand::AddEpochValidatorSet`通过router转发由`Peer-Discovery-Driver`触发

作用: 更新本地存储的验证者集合，并处理因验证者集合变更导致的自身角色功能切换。

核心方法: `PeerDiscovery.update_validator_set(self, epoch, validators)`

整体流程
```
update_validator_set(epoch, validators)
│
├── 阶段 1: 更新存储
│   └── epoch_validators.insert(epoch, validators)
│
├── 阶段 2: 条件判断(只处理当前的下一个epoch),自己是不是下一个epoch的验证者
│   └── epoch == current_epoch + 1
│
├── 阶段 3: 晋升处理(自己)
│   └── FullNode → Validator
│       └── 发送 Ping 给已连接的验证者
│
└── 阶段 4: 降级处理(自己)
    └── Validator → FullNode
        ├── enable_client=true → FullNodeClient + 寻找上游(Reresh的阶段6)
        └── enable_client=false → FullNodeNone
```
* 阶段3代码实现
```rust
// ============================================
// 阶段 3: 全节点 → 验证者 晋升处理
// ============================================
if (self.self_role == FullNodeClient || self.self_role == FullNodeNone) && is_next_epoch_validator
{
    // 发送 Ping 给下一个epoch的验证者
    for validator in next_validators {
        if self.routing_info.contains_key(&validator) {
            cmds.push(PeerDiscoveryCommand::RouterCommand {
                target: validator,
                message: PeerDiscoveryMessage::Ping(Ping {
                    id: self.rng.next_u32(),
                    local_name_record: self.self_record.clone(),
                }),
            });
        }
    }
}
```
* 阶段4代码实现
```rust
// ============================================
// 阶段 4: 验证者 → 全节点 降级处理
// ============================================
else if (self.self_role == ValidatorNone || self.self_role == ValidatorPublisher) && !is_next_epoch_validator
{
    if self.enable_client {
        self.self_role = FullNodeClient;
        // 清理二级广播的连接信息
        self.clear_connection_info();
        cmds.extend(self.look_for_upstream_validators());
    } else {
        self.self_role = FullNodeNone;
        // 清理二级广播的连接信息
        self.clear_connection_info();
    }
}
```
#### Round变更

触发: 共识模块EnterRound时`RouterCommand::UpdateCurrentRound`通过router转发由`Peer-Discovery-Driver`触发

作用: 更新current_round和current_epoch, 清理过期验证者, 更新本地节点晋升为验证者标记

核心方法: `PeerDiscovery.update_current_round(self, round, epoch)`

```rust
// 当节点晋升为验证者时
if (self.self_role == PeerDiscoveryRole::FullNodeNone || self.self_role == PeerDiscoveryRole::FullNodeClient)
    && self.check_current_epoch_validator(&self.self_id)
{
    if self.enable_publisher {
        self.self_role = PeerDiscoveryRole::ValidatorPublisher;
    } else {
        self.self_role = PeerDiscoveryRole::ValidatorNone;
    }
    // 清理二级广播的连接信息
    self.clear_connection_info();
}
```
