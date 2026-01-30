# monad-eth-txpool-executor

Ethereum 风格交易池执行器（executor），负责协调交易池与共识层、网络层、状态后端的交互，是 Monad BFT 共识中交易池模块的运行时封装。

`monad-eth-txpool-executor` 负责 **接收命令 → 协调交易池操作 → 处理 IPC/转发/预加载 → 发送事件**。

---

## 初始化

### 启动节点时初始化

创建 `ParentExecutor` 的时候创建 `EthTxPoolExecutor`：

```rust
let mut executor = ParentExecutor { 
    ...
    txpool: EthTxPoolExecutor::start(
        create_block_policy(),
        state_backend.clone(),
        EthTxPoolIpcConfig {
            bind_path: node_state.mempool_ipc_path,
            tx_batch_size: node_state.node_config.ipc_tx_batch_size as usize,
            max_queued_batches: node_state.node_config.ipc_max_queued_batches as usize,
            queued_batches_watermark: node_state.node_config.ipc_queued_batches_watermark as usize,
        },
        Duration::from_secs(15),  // soft_tx_expiry
        Duration::from_secs(5 * 60),  // hard_tx_expiry
        node_state.chain_config,
        node_state
            .forkpoint_config
            .high_certificate
            .qc()
            .get_round(),
        0,  // execution_timestamp_s
        true // do_local_insert = true
    )
    .expect("txpool ipc succeeds"),
    ...
}
```

### `EthTxPoolExecutor::start` 里

```rust
Ok(EthTxPoolExecutorClient::new(
    {
        let metrics = metrics.clone();

        move |command_rx, forwarded_rx, event_tx| {
            let pool = EthTxPool::new(
                // ...
            );

            Self {
                pool,
                ipc,
                block_policy,
                reset: EthTxPoolResetTrigger::default(),
                state_backend,
                chain_config,
                events_tx,
                events,
                forwarding_manager: Box::pin(EthTxPoolForwardingManager::default()),
                preload_manager: Box::pin(EthTxPoolPreloadManager::default()),
                metrics,
                executor_metrics,
                _phantom: PhantomData,
            }
            .run(command_rx, forwarded_rx, event_tx)
        }
    },
    Box::new(move |executor_metrics: &mut ExecutorMetrics| {
        metrics.update(executor_metrics)
    }),
))
```

executor 在独立的 tokio 任务中运行，通过三个通道与外部通信：
- `command_rx`：接收来自共识层的命令
- `forwarded_rx`：接收来自网络层的转发交易
- `event_tx`：发送事件给共识层

---

## Executor 说明

### 对外导出

#### `EthTxPoolExecutorMetrics`

```rust
pub struct EthTxPoolExecutorMetrics {
    pub reject_forwarded_invalid_bytes: AtomicU64,  // 转发交易中因无效字节而被拒绝的数量

    pub create_proposal: AtomicU64,  // 成功创建提案的总次数
    pub create_proposal_elapsed_ns: AtomicU64,  // 创建提案的总耗时（纳秒）

    pub preload_backend_lookups: AtomicU64,  // 预加载过程中对 state backend 的 DB lookup 次数
    pub preload_backend_requests: AtomicU64,  // 预加载请求的地址总数

    pub pool: EthTxPoolMetrics,  // 底层交易池的指标
}
```

这些指标通过 `ExecutorMetrics` 导出到 `monad.bft.txpool.*` 命名空间。

#### `EthTxPoolExecutorClient`

`EthTxPoolExecutorClient` 是 executor 的客户端封装，实现了 `Executor` trait，提供同步接口：

- **`exec(commands)`**：批量执行命令，内部会分离 `InsertForwardedTxs` 到独立的转发通道
- **`poll_next()`**：实现 `Stream` trait，轮询并返回 executor 发出的事件

#### `EthTxPoolExecutor`

executor 的核心结构，包含以下组件：

- **`pool: EthTxPool<...>`**：核心交易池实例
- **`ipc: EthTxPoolIpcServer`**：IPC 服务器，接收本地 RPC 提交的交易
- **`block_policy: EthBlockPolicy<...>`**：区块策略，用于验证和计算 base fee
- **`state_backend: SBT`**：状态后端，用于查询账户余额和 nonce
- **`forwarding_manager: EthTxPoolForwardingManager`**：转发管理器，处理交易转发逻辑
- **`preload_manager: EthTxPoolPreloadManager`**：预加载管理器，提前加载账户余额
- **`reset: EthTxPoolResetTrigger`**：重置触发器，控制初始化完成状态

---

## 命令处理（Commands）

executor 通过 `exec` 方法处理来自共识层的命令。所有命令都在同一个事件追踪器上下文中处理，确保指标和 IPC 事件的一致性。

### `TxPoolCommand::BlockCommit`

**来源**：共识状态（`monad-state/src/consensus.rs`），当区块被提交到账本时

**处理流程**：

```rust
TxPoolCommand::BlockCommit(committed_blocks) => {
    for committed_block in committed_blocks {
        // 1. 更新 block_policy 的 last_commit
        BlockPolicy::update_committed_block(
            &mut self.block_policy,
            &committed_block,
            &self.chain_config,
        );

        // 2. 更新预加载管理器，清理已提交的区块相关的预加载请求
        self.preload_manager.update_committed_block(&committed_block);

        // 3. 更新交易池，移除已上链的交易，推进 nonce usage
        self.pool.update_committed_block(
            &mut event_tracker,
            &self.chain_config,
            committed_block,
        );
    }

    // 4. 调度可转发的交易（在区块提交后，某些交易可能满足转发条件: 目前只看到了需要重发的交易）
    self.forwarding_manager
        .as_mut()
        .project()
        .schedule_egress_txs(&mut self.pool);
}
```

**触发时机**：每个区块提交后，按顺序处理所有已提交的区块。

### `TxPoolCommand::CreateProposal`

**来源**：共识状态（`monad-consensus-state/src/lib.rs`），当节点成为 leader 需要创建提案时

**处理流程**：

```rust
TxPoolCommand::CreateProposal {
    node_id, epoch, round, seq_num, high_qc, round_signature,
    last_round_tc, fresh_proposal_certificate,
    tx_limit, proposal_gas_limit, proposal_byte_limit,
    beneficiary, timestamp_ns,
    extending_blocks, delayed_execution_results,
} => {
    // 1. 更新预加载管理器，标记该 seqnum 的提案已创建
    self.preload_manager.update_on_create_proposal(seq_num);

    // 2. 计算 base fee
    let maybe_tfm_base_fees = self.block_policy.compute_base_fee(
        &extending_blocks,
        &self.chain_config,
        timestamp_ns,
    );

    let (base_fee, base_fee_field, base_fee_trend_field, base_fee_moment_field) =
        match maybe_tfm_base_fees {
            Some((base_fee, base_fee_trend, base_fee_moment)) => (
                base_fee,
                Some(base_fee),
                Some(base_fee_trend),
                Some(base_fee_moment),
            ),
            None => (PRE_TFM_BASE_FEE, None, None, None),
        };

    // 3. 调用交易池创建提案
    match self.pool.create_proposal(
        &mut event_tracker,
        epoch, round, seq_num, base_fee,
        tx_limit, proposal_gas_limit, proposal_byte_limit,
        beneficiary, timestamp_ns, node_id, round_signature.clone(),
        extending_blocks,
        &self.block_policy,
        &self.state_backend,
        &self.chain_config,
    ) {
        Ok(proposed_execution_inputs) => {
            // 4. 创建成功后, 发送 Proposal 事件给共识层
            self.events_tx.send(MempoolEvent::Proposal {
                epoch, round, seq_num, high_qc, timestamp_ns,
                round_signature,
                base_fee: base_fee_field,
                base_fee_trend: base_fee_trend_field,
                base_fee_moment: base_fee_moment_field,
                delayed_execution_results,
                proposed_execution_inputs,
                last_round_tc,
                fresh_proposal_certificate,
            })
        }
        Err(err) => {
            error!(?err, "txpool executor failed to create proposal");
        }
    }
}
```

**触发时机**：节点成为当前轮次或未来轮次的 leader 时。

### `TxPoolCommand::EnterRound`

**来源**：共识状态（`monad-state/src/consensus.rs`），当进入新的共识轮次时

**处理流程**：

```rust
TxPoolCommand::EnterRound {
    epoch: _,
    round,
    upcoming_leader_rounds,
} => {
    // 1. 通知交易池进入新轮次（检查 chain revision 变化）
    self.pool.enter_round(&mut event_tracker, &self.chain_config, round);

    // 2. 更新预加载管理器，为即将到来的 leader 轮次准备预加载请求
    self.preload_manager.enter_round(
        round,
        self.block_policy.get_last_commit(),
        upcoming_leader_rounds,
        || self.pool.generate_sender_snapshot(),  // 生成当前池中所有发送者地址的快照
    );
}
```

**触发时机**：每个共识轮次开始时。

### `TxPoolCommand::Reset`

**来源**：状态同步（`monad-state/src/lib.rs`），当状态同步完成需要重置交易池时，可以看 monad-eth-txpool.md 的 118 行；

**处理流程**：

```rust
TxPoolCommand::Reset {
    last_delay_committed_blocks,
} => {
    // 1. 重置 block_policy
    BlockPolicy::reset(
        &mut self.block_policy,
        last_delay_committed_blocks.iter().collect(),
        &self.chain_config,
    );

    // 2. 重置交易池（清空 tracked，更新 last_commit）
    self.pool.reset(
        &mut event_tracker,
        &self.chain_config,
        last_delay_committed_blocks,
    );

    // 3. 标记重置完成（允许后续操作）
    self.reset.set_reset();
}
```

**触发时机**：状态同步完成后，对齐到最后一个 delay-committed 区块。

### `TxPoolCommand::InsertForwardedTxs`

**一句话**：把“别的节点转发过来的交易字节批次”交给 txpool executor，最终以 `Forwarded` 类型插入交易池。

**来源**：网络层收到 `ForwardedTx` 消息后，先转换成 `MempoolEvent::ForwardedTxs`，再由共识层发出本命令：

- 网络消息：`MonadMessage::ForwardedTx(Vec<Bytes>)`
- 转换为事件：`MonadEvent::MempoolEvent(MempoolEvent::ForwardedTxs { sender, txs })`
- 共识层处理：`handle_mempool_event` 把它变成 `TxPoolCommand::InsertForwardedTxs { sender, txs }`

**处理流程**：

该命令在 `EthTxPoolExecutorClient::exec` 中被**特殊处理**，不会进入 `command_rx`（强一致的命令队列），而是被旁路到 `forwarded_rx`（best-effort 队列），由 executor 的 `run` 循环异步处理：
强一致队列（command_rx）：不能丢、不能乱序、落后就“硬失败”（panic），保证系统状态演进可控。
best-effort 队列（forwarded_rx）：允许丢、允许延迟，优先保证系统不被噪声流量拖垮。

```rust
// 在 client.rs 中
let (commands, forwarded): (Vec<Self::Command>, Vec<ForwardedTxs<SCT>>) =
    commands.into_iter().partition_map(|command| match command {
        TxPoolCommand::InsertForwardedTxs { sender, txs } => {
            Either::Right(ForwardedTxs { sender, txs })
        }
        command => Either::Left(command),
    });

    if !commands.is_empty() {
            self.command_tx
                .try_send(commands)
                .expect("EthTxPoolExecutorClient executor is lagging")
        }

    if !forwarded.is_empty() {
        if let Err(err) = self.forwarded_tx.try_send(forwarded) {
            warn!(
                ?err,
                "txpool executor client forwarded channel full, dropping forwarded txs"
            );
        }
    }

// 在 executor 的 run 循环中
result = forwarded_rx.recv() => {
    self.process_forwarded_txs(forwarded_txs);
}
```

**处理细节**：

```rust
fn process_forwarded_txs(&mut self, forwarded_txs: Vec<ForwardedTxs<SCT>>) {
    for ForwardedTxs { sender, txs } in forwarded_txs {
        // 1. 解码交易，过滤无效字节
        let txs = txs.into_iter().filter_map(|raw_tx| {
            if let Ok(tx) = TxEnvelope::decode(&mut raw_tx.as_ref()) {
                Some(tx)
            } else {
                num_invalid_bytes += 1;
                None
            }
        }).collect();

        // 2. 添加到转发管理器的 ingress 队列（先入队、再分批 poll_ingress 插入 pool）
        self.forwarding_manager
            .as_mut()
            .project()
            .add_ingress_txs(txs);
    }
}
```

**为什么要走 `forwarded_rx` 旁路**：

- `command_rx.try_send(...)` 会“要求 executor 不能落后”（落后会 `expect(...)` 直接 panic），用于处理共识/状态机必须执行的命令。
- `forwarded_rx.try_send(...)` 是 **best-effort**：满了会记录 warning 并**丢弃**这批转发交易（见 `client.rs`）。

**后续真正插入交易池的地方**：

- `process_forwarded_txs` 只是把解码后的 `TxEnvelope` 放入 forwarding manager 的 `ingress` 队列。
- executor 的 `Stream::poll_next` 会 `poll_ingress` 按节流策略分批取出（`INGRESS_CHUNK_MAX_SIZE = 128`，`INGRESS_CHUNK_INTERVAL_MS = 8`），恢复签名者后以 `PoolTransactionKind::Forwarded` 插入 `pool.insert_txs(...)`，并触发预加载（preload）。

**丢弃/过滤点汇总**：

- `forwarded_rx` 满：整批丢弃（warning）
- 字节解码失败：按笔过滤、计数并 warning（`reject_forwarded_invalid_bytes`）
- `ingress` 队列满：按容量丢弃（`INGRESS_MAX_SIZE = 8 * 1024`）
- 签名恢复失败：按笔丢弃（`InvalidSignature`）

---

## 事件发送（Events）

executor 通过 `Stream` trait 的 `poll_next` 方法异步产生事件，发送给共识层。

### `MempoolEvent::Proposal`

**接收者**：共识状态（`monad-state/src/consensus.rs::handle_mempool_event`）

**内容**：

```rust
MempoolEvent::Proposal {
    epoch, round, seq_num, high_qc, timestamp_ns, round_signature,
    base_fee, base_fee_trend, base_fee_moment,  // TFM 相关字段
    delayed_execution_results,
    proposed_execution_inputs,  // 包含 header 和 body（交易列表）
    last_round_tc, fresh_proposal_certificate,
}
```

**触发时机**：成功调用 `pool.create_proposal` 后，立即通过 `events_tx` 发送。

**处理**：共识层收到后，构建 `ProposalMessage` 并通过 Raptorcast 广播。

### `MempoolEvent::ForwardTxs`

**接收者**：共识状态（`monad-state/src/consensus.rs::handle_mempool_event`）

**内容**：

```rust
MempoolEvent::ForwardTxs(Vec<Bytes>)  // 编码后的交易字节列表
```

**触发时机**：转发管理器的 egress 队列准备好一批交易时（通过 `poll_egress`）。

**处理**：共识层收到后，通过 `RouterCommand::Publish` 发送给未来的 leader 节点。

**转发条件**（在 `forward.rs` 中定义）：

- 如果是重发：`EGRESS_MIN_COMMITTED_SEQ_NUM_DIFF = 5`：交易距离上次被尝试转发时的 committed seq_num 至少过去 5 个 committed block，才允许再次进入 egress
- 如果是重发：`EGRESS_MAX_RETRIES = 3`：最多重试转发 3 次
- 交易必须是 `Owned` 类型（本地创建或接收的）
- 交易必须满足 base fee 要求

一路追溯会找到:

```rust
// monad-consensus-state/src/lib.rs:1825 发给未来三个 leader 节点 NUM_LEADERS_FORWARD_TXS = 3
pub fn iter_future_other_leaders<'a>(
    &'a self,
) -> impl Iterator<Item = NodeId<CertificateSignaturePubKey<ST>>> + 'a {
    self.compute_upcoming_leader_round_pairs::<false, NUM_LEADERS_FORWARD_TXS>()
        .filter_map(|(nodeid, _)| (nodeid != *self.nodeid).then_some(nodeid))
        .unique()
}
```

---

## 异步处理（Stream）

executor 实现了 `Stream` trait，在 `poll_next` 中处理多个异步源：

### 1. 内部事件转发

```rust
if let Poll::Ready(result) = events.poll_recv(cx) {
    let event = result.expect("events_tx never dropped");
    return Poll::Ready(Some(MonadEvent::MempoolEvent(event)));
}
```

处理 executor 内部通过 `events_tx` 发送的事件（如 `Proposal`）。

### 2. IPC 交易接收

```rust
if let Poll::Ready(unvalidated_txs) = ipc.as_mut().poll_txs(cx, || pool.generate_snapshot()) {
    // 1. 恢复签名者
    let recovered_txs = unvalidated_txs.into_par_iter().partition_map(|tx| {
        match tx.secp256k1_recover() {
            Ok(signer) => Either::Left((Recovered::new_unchecked(tx, signer), PoolTransactionKind::Owned { ... })),
            Err(_) => Either::Right((tx_hash, Drop { reason: InvalidSignature })),
        }
    });

    // 2. 插入交易池
    pool.insert_txs(..., recovered_txs, |tx| {
        inserted_addresses.insert(tx.signer());
        if tx.is_owned_and_forwardable() {
            immediately_forwardable_txs.push(tx.raw().tx().clone());
        }
    });

    // 3. 添加预加载请求
    preload_manager.add_requests(inserted_addresses.iter());

    // 4. 添加立即可转发的交易
    forwarding_manager.add_egress_txs(immediately_forwardable_txs.iter());

    // 5. 广播 IPC 事件
    ipc.as_mut().broadcast_tx_events(ipc_events);
}
```

处理来自本地 RPC 的交易提交。

### 3. 转发交易出站

```rust
if let Poll::Ready(forward_txs) = forwarding_manager
    .as_mut()
    .poll_egress(pool.current_revision().1.execution_chain_params(), cx)
{
    return Poll::Ready(Some(MonadEvent::MempoolEvent(MempoolEvent::ForwardTxs(forward_txs))));
}
```

当转发管理器准备好一批交易时，发送 `ForwardTxs` 事件。

### 4. 转发交易入站

```rust
while let Poll::Ready(forwarded_txs) = forwarding_manager.as_mut().poll_ingress(cx) {
    // 1. 恢复签名者
    let recovered_txs = forwarded_txs.into_par_iter().partition_map(|tx| {
        match tx.secp256k1_recover() {
            Ok(signer) => Either::Left((Recovered::new_unchecked(tx, signer), PoolTransactionKind::Forwarded)),
            Err(_) => Either::Right((tx_hash, Drop { reason: InvalidSignature })),
        }
    });

    // 2. 插入交易池
    pool.insert_txs(..., recovered_txs, |tx| {
        inserted_addresses.insert(tx.signer());
    });

    // 3. 添加预加载请求
    preload_manager.add_requests(inserted_addresses.iter());

    // 4. 完成 ingress 处理
    forwarding_manager.as_mut().complete_ingress();
}
```

处理转发管理器的 ingress 队列中的交易（来自 `process_forwarded_txs`）。

### 5. 账户预加载

```rust
while let Poll::Ready((predicted_proposal_seqnum, addresses)) =
    preload_manager.as_mut().poll_requests(cx)
{
    // 1. 记录 DB lookup 次数
    let total_db_lookups_before = state_backend.total_db_lookups();

    // 2. 预加载账户余额
    if let Err(state_backend_error) = block_policy.compute_account_base_balances(
        predicted_proposal_seqnum,
        state_backend,
        chain_config,
        None,
        addresses.iter(),
    ) {
        warn!(?state_backend_error, "txpool executor failed to preload account balances")
    }

    // 3. 更新指标
    metrics.preload_backend_lookups.fetch_add(
        state_backend.total_db_lookups() - total_db_lookups_before,
        Ordering::SeqCst,
    );

    // 4. 标记请求完成
    preload_manager.complete_polled_requests(predicted_proposal_seqnum, addresses.into_iter());
}
```

当预加载管理器准备好一批地址时，提前加载这些地址的余额，减少创建提案时的延迟。

---

## 转发管理器（Forwarding Manager）

`EthTxPoolForwardingManager` 负责管理交易的转发逻辑，包括入站（ingress）和出站（egress）两个方向。

### 出站（Egress）

**作用**：从交易池中挑选可转发的交易，批量发送给未来的 leader 节点。

**批量限制**：

- 每批交易的总字节数不超过 `egress_max_size_bytes(execution_params)`
- 如果单笔交易超过限制，会被跳过并记录错误

### 入站（Ingress）

**作用**：接收来自其他节点的转发交易，批量处理后插入交易池。

**批处理策略**：

- `INGRESS_CHUNK_MAX_SIZE = 128`：每批最多 128 笔交易
- `INGRESS_CHUNK_INTERVAL_MS = 8`：每批间隔 8 毫秒
- `INGRESS_MAX_SIZE = 8 * 1024`：队列最大容量 8K 笔交易

**处理流程**：

1. 接收转发交易（来自 `process_forwarded_txs`）
2. 添加到 ingress 队列
3. 定时批量取出（通过 `poll_ingress`）
4. 恢复签名者并插入交易池
5. 完成处理后重置定时器

---

## 预加载管理器（Preload Manager）

`EthTxPoolPreloadManager` 负责提前加载账户余额，减少创建提案时的延迟。

### 预加载策略

**目标**：在创建提案之前，提前加载可能参与提案的账户余额。

**触发时机**：

1. **进入轮次时**：如果当前节点是当前轮次或下一轮次的 leader，为预测的提案 seqnum 创建预加载条目
2. **插入交易时**：当新交易插入后，将发送者地址添加到所有活跃的预加载条目
3. **收到转发交易时**：同样将发送者地址添加到预加载条目

### 预测的 proposal seqnum 是怎么来的

`enter_round` 会根据当前 round、上次 commit 的 seqnum、以及“未来 leader rounds”计算：

```rust
// 预测提案 seqnum = last_commit_seqnum + COMMITTED_TO_PROPOSAL_SEQNUM_DIFF + round_diff
let round_diff = leader_round - current_round;
let predicted_proposal_seqnum = last_commit_seqnum + SeqNum(2) + SeqNum(round_diff);
```

**状态管理**：

- `preload: bool`：是否启用预加载（当提案已创建或不再需要时设为 false）
- `pending: IndexSet<Address>`：待预加载的地址集合
- `done: HashSet<Address>`：已完成预加载的地址集合

**清理时机**：

- 当区块提交时，清理已提交区块之前的预加载条目
- 当提案创建时，禁用该 seqnum 及之前的所有预加载条目

### 核心结构

#### `EthTxPoolPreloadManager`

- **`map: BTreeMap<SeqNum, EthTxPoolPreloadEntry>`**
  - key 是“预测的 proposal seqnum”（`predicted_proposal_seqnum`）
  - `BTreeMap` 保证按 seqnum 递增遍历（优先处理更近的 proposal）
- **`timer: Sleep`**
  - 用于对预热批次做节流（每批之间 sleep 一小段时间）
- **`waker: Option<Waker>`**
  - 当当前没有可预热地址时，保存 waker；后续有新请求加入时唤醒 executor 继续 poll

#### `EthTxPoolPreloadEntry`

每个 `predicted_proposal_seqnum` 对应一个 entry，记录该 proposal 前要预热的地址集合：

- **`preload: bool`**：该 seqnum 是否还处于“允许预热”的状态
- **`pending: IndexSet<Address>`**：待预热地址（去重 + 保持插入顺序）
- **`done: HashSet<Address>`**：已预热地址（用于去重，避免重复预热）

### 关键常量

- **`PRELOAD_CHUNK_MAX_ADDRESSES = 128`**
  - 注释给出经验值：单个账户 lookup ~30µs，因此每批最多约 128 个，避免单次预热阻塞线程过久（~4ms 内）
- **`PRELOAD_INTERVAL_MS = 1`**
  - 每处理一批后，至少间隔 1ms 再处理下一批（限速/让出）
- **`COMMITTED_TO_PROPOSAL_SEQNUM_DIFF = 2`**
  - 用于从 `last_commit_seqnum` 推导“当前/未来 round 里将要创建的 proposal seqnum”

### 运行流程

#### 进入新 round：`enter_round(...)`:

入口：`TxPoolCommand::EnterRound` 会调用 `preload_manager.enter_round(...)`，同时把 `generate_requests` 设为 `pool.generate_sender_snapshot()`（从 txpool 快照里拿一批发送方地址）。

`enter_round` 主要做 3 件事：

- **裁剪旧状态**
  - `self.map = self.map.split_off(&(last_commit_seqnum + 1))`
  - 只保留未来（大于 last_commit_seqnum）的预测条目

- **更新哪些 seqnum 还“值得预热”**
  - 如果某个 entry 的 seqnum 已不在“未来 leader 预测集合”里：
    - 若 `done` 为空：直接删除该 entry（完全没价值）
    - 若 `done` 非空：保留 entry 但置 `preload = false`（避免继续无意义预热）
  - 同时清空每个 entry 的 `pending`（后面会用新快照重建）
  - 代码注释特别提到：当 round 因 TC（timeout certificate）推进而 seqnum 不变时，旧的预测可能失效，需要及时“失效化”避免浪费预热。
- **仅在“本 round 或下一 round 将成为 leader”时启用预热**
  - 若 `upcoming_leader_rounds` 包含当前 round：确保存在 `predicted = last_commit_seqnum + 2` 的 entry，并置 `preload = true`
  - 若包含下一 round：确保存在 `predicted = last_commit_seqnum + 3` 的 entry，并置 `preload = true`
  - 然后用 `generate_requests()` 的结果调用 `add_requests` 加入待预热队列

#### 新交易进入 txpool：`add_requests(...)`

在 `EthTxPoolExecutor` 中，每当插入交易成功，会把“本次插入涉及的发送方地址集合”喂给：

- `preload_manager.add_requests(inserted_addresses.iter())`

`add_requests` 会遍历 `map` 中所有 entry，对 `preload=true` 的 entry：

- 若地址已在 `done`：跳过
- 否则插入 `pending`
- 最后若当前保存了 `waker`，会立刻 `wake()`，让 executor 尽快开始预热

#### 执行器轮询并触发预热：`poll_requests(...)`

`EthTxPoolExecutor::poll_next` 内部有一个循环：

- `while let Poll::Ready((predicted_proposal_seqnum, addresses)) = preload_manager.poll_requests(cx) { ... }`

`poll_requests` 的行为可概括为：

- **若还没到 timer 时间**：返回 `Pending`
- **若存在某个 seqnum 的 entry 满足 `preload=true` 且 `pending` 非空**：
  - 返回该 seqnum 以及一批（最多 128 个）地址
  - 当 `pending.len() > 128` 时，会“分批”返回一部分地址，剩余继续留在 `pending`
- **若所有 entry 都没有可做的 pending**：
  - 保存 waker 并返回 `Pending`
  - 之后只要有新地址加入（`add_requests`），就会唤醒继续 poll

#### 4 一批预热完成：`complete_polled_requests(...)`

`EthTxPoolExecutor` 对 `poll_requests` 返回的 `(seqnum, addresses)` 执行预热后，会调用：

- `preload_manager.complete_polled_requests(seqnum, addresses.into_iter())`

该方法会：

- **重置 timer**：设置下一次允许预热的时间点（`sleep(1ms)`）
- **把本批地址加入 `done`**（即使预热失败也会这样做；失败会在外层只记录 warn）
- 如果 seqnum 不存在，会 `warn!`（“unknown seqnum”），然后丢弃本次完成信息

#### 5 创建 proposal：`update_on_create_proposal(seqnum)`

在 `TxPoolCommand::CreateProposal` 处理时，执行器先调用：

- `preload_manager.update_on_create_proposal(seq_num)`

其效果：

- 对 `map` 中 `<= create_proposal_seqnum` 的所有 entry：
  - `preload=false`
  - 清空 `pending`

含义：一旦进入实际提案流程，这个 seqnum 及更早的预测都不再继续预热（避免和关键路径争资源/避免对已过期窗口继续工作）。

#### 6 提交区块：`update_committed_block(committed_block)`

在 `TxPoolCommand::BlockCommit` 中，执行器会调用：

- `preload_manager.update_committed_block(&committed_block)`

其效果：

- `self.map = self.map.split_off(&(committed_seqnum + 1))`
- 也就是把已提交及之前的 seqnum 全部裁掉，只保留未来窗口

---

## 与其他模块的交互（总结）

### 命令流（Commands）

executor 接收来自以下模块的命令：

| 命令 | 来源模块 | 触发时机 | 说明 |
|------|---------|---------|------|
| `BlockCommit` | `monad-state` (共识状态) | 区块提交到账本后 | 更新交易池状态，移除已上链交易 |
| `CreateProposal` | `monad-consensus-state` | 节点成为 leader 时 | 创建区块提案，包含交易列表 |
| `EnterRound` | `monad-state` (共识状态) | 进入新共识轮次时 | 更新预加载管理器，准备未来轮次 |
| `Reset` | `monad-state` | 状态同步完成后 | 重置交易池，对齐到最新区块 |
| `InsertForwardedTxs` | `monad-state` (网络层) | 收到转发交易消息时 | 将转发交易插入交易池 |

### 事件流（Events）

executor 发送以下事件给共识层：

| 事件 | 接收模块 | 触发时机 | 说明 |
|------|---------|---------|------|
| `MempoolEvent::Proposal` | `monad-state` (共识状态) | 成功创建提案后 | 包含提案的 header 和 body |
| `MempoolEvent::ForwardTxs` | `monad-state` (共识状态) | 转发管理器准备好一批交易时 | 需要转发给未来 leader 的交易 |

### 交互流程图

#### 命令流（Commands Flow）

```
┌─────────────────────────────────────────────────────────────┐
│                    Commands 来源                            │
└─────────────────────────────────────────────────────────────┘

1. CreateProposal
   ConsensusState (leader 节点)
        │
        └─> TxPoolCommand::CreateProposal
            └─> EthTxPoolExecutor::exec
                ├─> Pool::create_proposal
                └─> MempoolEvent::Proposal ──> ConsensusState

2. BlockCommit
   ConsensusState (区块提交后)
        │
        └─> TxPoolCommand::BlockCommit
            └─> EthTxPoolExecutor::exec
                ├─> BlockPolicy::update_committed_block
                ├─> PreloadManager::update_committed_block
                ├─> Pool::update_committed_block
                └─> ForwardingManager::schedule_egress_txs

3. EnterRound
   ConsensusState (进入新轮次)
        │
        └─> TxPoolCommand::EnterRound
            └─> EthTxPoolExecutor::exec
                ├─> Pool::enter_round
                └─> PreloadManager::enter_round

4. Reset
   StateSync (状态同步完成)
        │
        └─> TxPoolCommand::Reset
            └─> EthTxPoolExecutor::exec
                ├─> BlockPolicy::reset
                └─> Pool::reset

5. InsertForwardedTxs
   Network Layer ──> ConsensusState
        │
        └─> TxPoolCommand::InsertForwardedTxs
            └─> EthTxPoolExecutorClient (路由到 forwarded_rx)
                └─> EthTxPoolExecutor::process_forwarded_txs
                    └─> ForwardingManager::add_ingress_txs
```

#### 事件流（Events Flow）

```
┌─────────────────────────────────────────────────────────────┐
│                    Events 流向                              │
└─────────────────────────────────────────────────────────────┘

1. Proposal Event
   EthTxPoolExecutor
        │
        └─> MempoolEvent::Proposal
            └─> ConsensusState::handle_mempool_event
                └─> 构建 ProposalMessage ──> Network (广播)

2. ForwardTxs Event
   EthTxPoolExecutor (Stream::poll_next)
        │
        └─> ForwardingManager::poll_egress
            └─> MempoolEvent::ForwardTxs
                └─> ConsensusState::handle_mempool_event
                    └─> RouterCommand::Publish ──> Future Leaders
```


### 详细交互时序

#### 1. 创建提案流程

```
ConsensusState (leader)
    │
    ├─> TxPoolCommand::CreateProposal
    │
EthTxPoolExecutor
    │
    ├─> PreloadManager::update_on_create_proposal
    ├─> BlockPolicy::compute_base_fee
    ├─> Pool::create_proposal
    │   ├─> 从交易池挑选交易
    │   ├─> 验证交易（余额、nonce）
    │   └─> 生成 ProposedExecutionInputs
    │
    └─> MempoolEvent::Proposal
        │
ConsensusState
    │
    └─> 构建 ProposalMessage 并广播
```

#### 2. 区块提交流程

```
ConsensusState
    │
    ├─> TxPoolCommand::BlockCommit
    │
EthTxPoolExecutor
    │
    ├─> BlockPolicy::update_committed_block
    ├─> PreloadManager::update_committed_block
    ├─> Pool::update_committed_block
    │   ├─> 移除已上链交易
    │   └─> 推进 nonce usage
    │
    └─> ForwardingManager::schedule_egress_txs
        │
        └─> 检查是否有新交易满足转发条件
```

#### 3. 交易转发流程

```
Network Layer
    │
    ├─> ForwardedTxs (消息)
    │
ConsensusState
    │
    ├─> TxPoolCommand::InsertForwardedTxs
    │
EthTxPoolExecutor
    │
    ├─> process_forwarded_txs
    │   ├─> 解码交易
    │   └─> ForwardingManager::add_ingress_txs
    │
    └─> Stream::poll_next (异步)
        │
        ├─> ForwardingManager::poll_ingress
        ├─> 恢复签名者
        ├─> Pool::insert_txs
        └─> PreloadManager::add_requests
```

#### 4. 账户预加载流程

```
EthTxPoolExecutor
    │
    ├─> TxPoolCommand::EnterRound
    │   └─> PreloadManager::enter_round
    │       └─> 为未来 leader 轮次创建预加载条目
    │
    ├─> Pool::insert_txs (IPC 或转发交易)
    │   └─> PreloadManager::add_requests
    │
    └─> Stream::poll_next (异步)
        │
        └─> PreloadManager::poll_requests
            │
            └─> BlockPolicy::compute_account_base_balances
                │
                └─> StateBackend (提前加载余额)
```

---

## 关键设计点

### 1. 异步架构

executor 在独立的 tokio 任务中运行，通过通道与外部通信，避免阻塞共识层。

### 2. 批量处理

- **命令批量执行**：所有命令在同一个 `exec` 调用中处理，共享事件追踪器
- **转发批量发送**：转发管理器批量收集交易，减少网络开销
- **预加载批量请求**：预加载管理器批量加载账户余额，提高效率

### 3. 预加载优化

通过预测未来 leader 轮次，提前加载账户余额，显著减少创建提案时的延迟。

### 4. 转发策略

- **延迟转发**：交易必须距离已提交区块至少 5 个区块才转发，避免重复
- **重试限制**：最多重试 3 次，防止无限转发
- **批量限制**：每批交易有字节数上限，避免网络拥塞

### 5. 错误处理

- **无效交易**：自动过滤并记录指标
- **状态后端错误**：记录警告但不中断流程
- **通道关闭**：优雅关闭 executor
