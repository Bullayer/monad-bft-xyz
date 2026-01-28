# monad-eth-txpool

Ethereum 风格交易池（mempool）核心库，负责待打包交易的校验、存储、排序与提案构建，是 Monad BFT 共识中执行层输入（区块体）的来源。

`monad-eth-txpool` 负责 **接收交易 → 基本校验 → 按账户/nonce 组织与管理 → 出块时挑选可执行交易 → 区块提交后清理更新**。

> **补充说明**：本说明基于 `monad-bft-xyz/exp` 模块的 monad-bft-xyz/exp 实现。我看最近这个模块有一些推送，但是基本逻辑没变只是把配置项拎出来了。

txpool 会做一些与链上状态相关的"轻量检查"（比如余额、nonce），以避免明显无效交易占用资源。

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
                EthTxPoolConfig {
                    limits: TrackedTxLimitsConfig::new(
                        None,
                        None,
                        None,
                        None,
                        soft_tx_expiry,
                        hard_tx_expiry,
                    ),
                    do_local_insert,
                },
                chain_config.chain_id(),
                chain_config.get_chain_revision(round),
                chain_config.get_execution_chain_revision(execution_timestamp_s),
            );

            Self {
                ...
            }
            .run(command_rx, forwarded_rx, event_tx)
        }
    },
    Box::new(move |executor_metrics: &mut ExecutorMetrics| {
        metrics.update(executor_metrics)
    }),
))
```

这部分会在 `EthTxPoolExecutor` 章节再做说明，此处只需要知道 `TxPool` 初始化流程即可。

### `EthTxPool::new`

```rust
pub fn new(
    config: EthTxPoolConfig,
    chain_id: u64,
    chain_revision: CRT,
    execution_revision: MonadExecutionRevision,
) -> Self {
    let EthTxPoolConfig {
        limits: config_limits,
        do_local_insert,
    } = config;

    Self {
        tracked: TrackedTxMap::new(config_limits),

        last_commit: None,

        chain_id,
        chain_revision,
        execution_revision,

        do_local_insert,
    }
}
```

### 补充说明：初始化完成，此时 `last_commit` 为 `None`

设置 `last_commit` 的时机在 `monad-state/src/lib.rs/maybe_start_consensus`。

在 `DbSyncStatus::Done` 之后：
- 取 `root_parent_chain` 上最近 `2 * execution_delay` 个已提交块，验证成 `EthValidatedBlock` 列表 `last_two_delay_committed_blocks`。
- 调用 `block_policy.reset(...)`。
- 发出 `TxPoolCommand::Reset { last_delay_committed_blocks }` 给 txpool

**TxPoolExecutor** 处理：

```rust
TxPoolCommand::Reset {
    last_delay_committed_blocks,
} => {
    BlockPolicy::<ST, SCT, EthExecutionProtocol, SBT, CCT, CRT>::reset(
        &mut self.block_policy,
        last_delay_committed_blocks.iter().collect(),
        &self.chain_config,
    );

    self.pool.reset(
        &mut event_tracker,
        &self.chain_config,
        last_delay_committed_blocks,
    );

    self.reset.set_reset();
}
```

**TxPool** 的 `reset`：

```rust
self.last_commit = last_delay_committed_blocks
    .last()
    .map(|block| block.header().clone());
// ...
```

txpool 在状态同步完成之后，通过 Reset 对齐到最后一个 delay-committed 区块。

---

## TxPool 说明

### 对外导出

#### `EthTxPoolMetrics`

```rust
pub struct EthTxPoolMetrics {
    pub insert_owned_txs: AtomicU64, // 插入成功的本地交易
    pub insert_forwarded_txs: AtomicU64, // 插入成功的转发交易（Forwarded）总数。

    pub drop_not_well_formed: AtomicU64, // 交易结构/参数不合法而被丢弃的次数。
    pub drop_invalid_signature: AtomicU64, // 签名 / 系统账户相关检查未通过的丢弃次数。
    pub drop_nonce_too_low: AtomicU64, // nonce 低于该地址已知的 account_nonce（即已经不可执行）时被丢弃的次数。
    pub drop_fee_too_low: AtomicU64, // max_fee_per_gas 低于 txpool 设定的最低 base fee 而被拒。
    pub drop_insufficient_balance: AtomicU64, // 账户余额不足以覆盖该交易的最大 gas 消耗时被丢弃的次数。
    pub drop_existing_higher_priority: AtomicU64, // 同 (address, nonce) 已有更高优先级交易，新交易的优先级不够替换时被拒。
    pub drop_replaced_by_higher_priority: AtomicU64, // 旧交易因为来了"更高优先级的新交易"而被替换掉的次数。
    pub drop_pool_full: AtomicU64, // 因为 txpool 达到容量限制（地址数 / tx 数 / 字节数）而不得不丢掉交易的次数。
    pub drop_pool_not_ready: AtomicU64, // txpool 还没 ready（do_local_insert=false 或 last_commit=None）时，直接丢掉交易的次数。
    pub drop_internal_state_backend_error: AtomicU64, // 在插入过程中访问 state backend（查余额/nonce）出错导致内部错误时，被丢弃的交易数。
    pub drop_internal_not_ready: AtomicU64,  // 其它 "内部还没准备好" 类型的错误导致的丢弃次数

    pub create_proposal: AtomicU64, // 成功调用 EthTxPool::create_proposal 的总次数（不管最终选了多少 tx）。
    pub create_proposal_txs: AtomicU64, // 所有 proposal 中 被选入 的交易总数（累加）。
    pub create_proposal_tracked_addresses: AtomicU64, // 所有调用中，提案时池里有多少地址被统计为"tracked 地址"的累加值。
    pub create_proposal_available_addresses: AtomicU64, // 所有调用中，ProposalSequencer 构造时 参与排序的地址数量 的累加。
    pub create_proposal_backend_lookups: AtomicU64, // 所有 proposal 过程中，为计算余额、验证交易等，对 state backend 做的 DB lookup 次数累加。

    pub tracked: EthTxPoolTrackedMetrics,
}

pub struct EthTxPoolTrackedMetrics {
    pub addresses: AtomicU64, // 当前池里 活跃地址数 的最新值。
    pub txs: AtomicU64, // 当前池里 总交易数 的最新值。
    pub evict_expired_addresses: AtomicU64, // 因过期清理而被整个地址列表清空、移除地址 entry 的次数。
    pub evict_expired_txs: AtomicU64, // 因过期被移除的 单笔交易数量（累加）。
    pub remove_committed_addresses: AtomicU64, // 因为提交新块（BlockCommit）而导致某个地址 所有 tx 都已上链/被清空，从 map 中删除该地址 的次数。
    pub remove_committed_txs: AtomicU64, // 因为提交新块而被从池里移除的 单笔交易数量（累加）。
}
```

#### `EthTxPoolEventTracker`

把 txpool 内部发生的 Insert/Drop/Commit/Evict 等事件写入一个 `BTreeMap<TxHash, EthTxPoolEventType>`，并通过它更新 `EthTxPoolMetrics`。

```rust
pub struct EthTxPoolEventTracker<'a> {
    pub now: Instant,

    metrics: &'a EthTxPoolMetrics,
    events: &'a mut BTreeMap<TxHash, EthTxPoolEventType>,
}
```

`EthTxPoolEventTracker` 以交易哈希为键，事件类型为值。

**记录的事件类型**：

- **Insert**：记录内容 - 交易哈希、签名地址、是否为本节点拥有、完整交易数据（TxEnvelope）
- **Drop**：记录内容 - 交易哈希、丢弃原因（如格式错误、签名无效、nonce 过低、费用过低、余额不足、被更高优先级替换、池满等）
- **Commit**：记录内容 - 交易哈希
- **Evict**：记录内容 - 交易哈希、驱逐原因（目前仅 Expired）
- **Replace**：记录内容 - 旧交易被标记为 Drop（原因：被更高优先级替换），新交易被标记为 Insert

在 `EthTxPoolExecutor` 里的 `exec`：

```rust
fn exec(&mut self, commands: Vec<Self::Command>) {
    let mut ipc_events = BTreeMap::default();
    let mut event_tracker = EthTxPoolEventTracker::new(&self.metrics.pool, &mut ipc_events);

    for command in commands {
        // 执行具体的指令
    }

    // EthTxPoolMetrics 通过 update 方法导出的 ExecutorMetrics 里的对应 key 命名空间 monad.bft.txpool.pool.*
    self.metrics.update(&mut self.executor_metrics);

    // broadcast_tx_events 会消费 events map
    self.ipc.as_mut().broadcast_tx_events(ipc_events);
}
```

#### `EthTxPool`

##### `src/pool/transaction.rs`

定义并实现以太坊交易在进入 Monad 交易池前的初步验证逻辑，并把通过验证的交易包装成 `ValidEthTransaction` 结构体，供后续的交易池管理、排序、替换、转发等逻辑使用。

```rust
pub struct ValidEthTransaction {
    tx: Recovered<TxEnvelope>, // 已恢复签名者的原始交易（EIP-2718 格式） 核心交易本体
    owned: bool, // 是否本地自己创建/接收的（本地 RPC 来的 vs P2P 来的） 决定是否允许转发
    forward_last_seqnum: SeqNum, // 上次转发时的最新 commit seq num 控制转发频率、防重放
    forward_retries: usize, // 已经重试转发了几次 防止无限转发
    max_value: Balance, // 该交易可能从账户扣的最大金额（value + gas 费用上限） 用于账户余额粗检查
    max_gas_cost: Balance,// 最大 gas 费用（EIP-1559 情况下是 max_fee_per_gas × gas_limit） 用于账户余额粗检查
    valid_recovered_authorizations: Box<[ValidEthRecoveredAuthorization]>, // EIP-7702 授权列表中验证通过的授权（已恢复 authority） 后续执行时真正会使用的授权
}
```

**最重要的函数：`validate`**

这是交易进入 txpool 的第一道关卡，几乎所有常见的拒绝原因都在这里判定。

```rust
pub fn validate<...>(...) -> Result<Self, (Recovered<TxEnvelope>, EthTxPoolDropReason)>
```

**验证流程**：

1. **编码长度限制** → 防止有人塞超大交易进来攻击内存/CPU
2. **禁止使用系统地址签名** → `SYSTEM_SENDER_ETH_ADDRESS` 是 Monad 内部系统调用专用的，不能被外部交易占用
3. **禁止伪造系统调用** → 防止普通交易伪装成系统交易
4. **最低 base fee 检查** → 当前实现是静态的 `MIN_BASE_FEE` / `PRE_TFM_BASE_FEE`
5. **计算 max_value 和 max_gas_cost** → 用于后续账户余额的保守估计（最坏情况扣款）
6. **静态验证（`static_validate_transaction` 这个函数在 block-policy 里）**：
   - 如果 `tx.is_eip4844()`，不允许 EIP-4844（blob 交易）。EIP-4844，Reth 当前版本可能还没完全支持或刻意禁用
   - `TfmValidator::validate`：通常是 Reth 内部的额外/特定交易格式验证（Reth 自定义），可能是转账、合约创建、EIP-1559 等基础格式
   - `EthHomesteadForkValidation`：Homestead 之后的规则 + EIP-155 链 ID 保护（EIP-155），防止重放攻击，最重要的签名验证之一
   - `EthLondonForkValidation`：London 硬分叉规则 + EIP-1559 费用结构（EIP-1559），baseFee + priority fee 格式正确性
   - `EthShanghaiForkValidation`：Shanghai 硬分叉规则 + EIP-3860 合约大小限制（EIP-3860 max code size initcode），限制 initcode 最大 49152 字节
   - `YellowPaperValidation`：黄皮书最基础的内在 gas 消耗计算是否足够（Yellow Paper / 经典 gas 计算规则），签名、数据、访问列表等 gas 成本
   - `EthPragueForkValidation`：Prague 硬分叉规则 + EIP-7623 calldata 成本调整（EIP-7623 calldata gas 涨价），未来分叉（Prague），目前可能是预留/测试
7. **EIP-7702 授权列表特殊处理**：
   - 限制最多 4 条授权（txpool 自己加的限制，防止签名验证爆炸）
   - 逐条恢复 authority
   - 禁止系统地址做授权 signer
   - 只保留验证通过的授权

通过以上所有检查 → 包装成 `ValidEthTransaction` 才能进入交易池。

##### `src/pool/sequencer.rs`

为提议的下一个区块（proposal block）构建交易序列（transaction ordering / sequencing）的核心组件。它实现了一个交易排序器（sequencer），专门用来从交易池（txpool）中挑选和排序交易，生成一个"提案区块"里应该包含的交易列表。

| 部分 | 作用 | 关键点 / 特点 |
|------|------|--------------|
| `ProposalSequencer` | 主排序器结构，从 txpool 里挑选交易，构建 proposal | 使用 BinaryHeap + 虚拟时间戳（virtual_time）实现近似公平排序 |
| 按什么排序？ | 主要按 **effective tip per gas**（有效小费/ gas）降序 | 再 tie-break 用 gas limit 降序（优先填大额 gas 交易） |
| 为什么用 virtual_time？ | 防止同一个账户的交易一直霸占前面位置（类似 round-robin 的弱公平性） | 每次从同一个账户取出交易后，给它一个递增的 virtual_time，下次优先级降低 |
| 处理 EIP-7702 | 支持 set code 授权（authority），会跟踪 authority 的 nonce 可能变化 | 在 proposal 构建过程中动态调整后续交易的 nonce 检查 |
| 限制条件 | 区块 gas 上限、字节大小上限、账户余额/ nonce 校验 | 使用 `EthBlockPolicyBlockValidator` 做状态模拟验证 |
| 随机化（shuffle） | 在初始化时对前 N 个候选账户做部分随机打乱 | 防止某些极端场景下排序过于确定性 |

##### `EthTxPool<ST, SCT, SBT, CCT, CRT>`

**字段**：

- `tracked: TrackedTxMap<...>`：真正存交易的结构
- `last_commit: Option<ConsensusBlockHeader<...>>`：txpool 是否 ready 的关键（没有它不能做基于状态的校验/转发判定）
- `chain_id/chain_revision/execution_revision`：用来做静态验证与协议升级分支
- `do_local_insert`：是否允许插入

**主要方法**：

- **`insert_txs(event_tracker, block_policy, state_backend, chain_config, txs, on_insert)`**
  - 输入：一批 `Recovered<TxEnvelope>` + `PoolTransactionKind`（Owned/Forwarded）
  - 核心流程：
    - `do_local_insert == false` 或 `last_commit == None`→ 全部 drop
    - 并行调用 `ValidEthTransaction::validate(...)` 做静态验证（失败的逐个 drop）
    - 用 `block_policy` Balance 和 Nonce 检查，过滤掉明显无效的交易，避免占用池资源
    - 按地址分组后调用 `TrackedTxMap::try_insert_txs(...)` 真正写入
    - 最后 `update_aggregate_metrics`

- **`create_proposal(event_tracker, epoch, round, proposed_seq_num, base_fee, tx_limit, gas_limit, byte_limit, beneficiary, timestamp_ns, node_id, round_signature, extending_blocks, block_policy, state_backend, chain_config)`**
  - 做什么：生成 `ProposedExecutionInputs`（header + body）
  - 关键点：
    - 先 `tracked.evict_expired_txs`（出块前主动清掉过期）
    - 检测 `chain_revision/execution_revision` 是否变化，变化则 `static_validate_all_txs`（协议升级导致的静态规则变化）
    - 生成 system tx（`get_system_transactions`）
    - 生成 user tx（`sequence_user_transactions`）

- **`sequence_user_transactions(...)`**
  - 做什么：用 `ProposalSequencer` 从 `tracked` 里挑 user tx
  - 关键点：
    - 强依赖 `last_commit` 与 `block_policy.get_last_commit()` 一致（不一致会 error 并返回空）
    - `ProposalSequencer::new(self.tracked.iter(), ...)`
    - 用 `block_policy.compute_account_base_balances` 做一次按候选地址集合的余额快照（并统计 db lookup）
    - 用 `EthBlockPolicyBlockValidator::new(...)` 创建 validator，然后 `sequencer.build_proposal(...)`
    - 最后 `event_tracker.record_create_proposal(...)`

- **`update_committed_block(event_tracker, chain_config, committed_block)`**
  - 做什么：推进 `last_commit`，根据 committed block 更新 txpool（移除已上链、推进 nonce usage、驱逐过期）
  - 关键点：
    - 断言 committed block seqnum 连续（否则 panic/assert）
    - execution revision 变化会触发 `static_validate_all_txs`
    - `tracked.update_committed_nonce_usages(...)`
    - `tracked.evict_expired_txs(...)`

- **`enter_round(event_tracker, chain_config, round)`**
  - 做什么：round 进入时检查 chain revision 是否变化；变化则对池内所有 tx 做静态重验并移除无效项。

- **`reset(event_tracker, chain_config, last_delay_committed_blocks)`**
  - 做什么：用于 reorg/重置场景：更新 `last_commit`（取传入 blocks 的最后一个），更新 execution revision，清空 `tracked`。

- **`get_forwardable_txs<const MIN_SEQNUM_DIFF, const MAX_RETRIES>(&mut self)`**
  - 做什么：返回一个迭代器，遍历池内交易并挑出"可转发"的 tx（只针对 Owned 且满足 seqnum diff/retry/basefee 条件）
  - 关键点：内部调用 `ValidEthTransaction::get_if_forwardable(...)`（在 `transaction.rs`）

---

## `pool/tracked/`：存储、优先级与限制

### `monad-eth-txpool/src/pool/tracked/mod.rs`

**作用**

- `TrackedTxMap`：txpool 的"主存储结构"，按地址持有 `TrackedTxList`: `IndexMap<Address, TrackedTxList>`。
- 同时管理：
  - `PriorityMap`：地址级优先级（用于驱逐选择）
  - `TrackedTxLimits`：容量与 expiry 策略（用于超限/过期）

**重点类型/函数**

- **`TrackedTxMap::new(limits_config)`**
  - 初始化 limits，并用 limits 的配置 创建 `IndexMap`（为了迭代与 entry API 的性能）。

- **`try_insert_txs(event_tracker, last_commit, address, txs, account_nonce, on_insert)`**
  - **语义**：把某地址的一组 `ValidEthTransaction` 插入到 `TrackedTxList`
  - **插入后**：更新该地址 priority，并在超限时循环驱逐地址（整组移除）

- **`update_committed_nonce_usages(event_tracker, nonce_usages)`**
  - **语义**：区块提交后，按地址推进 nonce usage 并批量清理已上链 tx
  - **行为**：如果地址列表变空，会移除该地址并从 `PriorityMap` 删除

- **`evict_expired_txs(event_tracker)`**
  - **语义**：遍历地址，按 `TrackedTxLimits::expiry_duration_during_evict()` 计算 expiry，并调用 `TrackedTxList::evict_expired_txs`

- **`static_validate_all_txs(...)`**
  - **语义**：当 chain/execution revision 变化时，重新做静态验证并删除不再合法的 tx。

---

### `monad-eth-txpool/src/pool/tracked/list.rs`

**作用**

- `TrackedTxList`：单地址的交易列表（`BTreeMap<Nonce, (ValidEthTransaction, Instant)>`）
  - 强制 `nonce >= account_nonce`
  - 提供"nonce 连续队列"视图（`get_queued`）
  - 支持同 nonce 替换、过期驱逐、commit 清理（推进 `account_nonce`）

**重点函数**

- **`TrackedTxList::try_new(vacant_entry, event_tracker, limits, txs, account_nonce, on_insert, last_commit_base_fee)`**
  - **语义**：首次创建某地址列表并插入一批 tx；如果最终为空则不插入 map（保证 map 不出现空列表）。

- **`try_insert_tx(event_tracker, limits, tx, last_commit_base_fee)`**
  - **语义**：单笔插入（或替换）
  - **关键分支**：
    - nonce 太低 → drop `NonceTooLow`
    - vacant → `limits.add_tx` + `event_tracker.insert`
    - occupied → 旧 tx 未过期且新 tx 优先级不够 → drop `ExistingHigherPriority`
    - 否则替换：`limits` 先 add 再 remove；`event_tracker.replace`

- **`get_queued(pending_nonce_usage)`**
  - **语义**：从 `account_nonce`（可叠加 pending usage）开始，产出严格连续 nonce 的 tx；遇到断档立即停止。

- **`update_committed_nonce_usage(event_tracker, limits, occupied_entry, nonce_usage) -> bool`**
  - **语义**：推进 `account_nonce`，移除已提交之前的 tx
  - **返回值**：true 表示地址仍保留，false 表示地址被移除

- **`evict_expired_txs(event_tracker, limits, indexed_entry, tx_expiry) -> bool`**
  - **语义**：按时间驱逐过期 tx；地址变空则移除并返回 true

- **`evict_pool_full(event_tracker, limits, occupied_entry)`**
  - **语义**：容量超限时整组驱逐某地址（drop reason = `PoolFull`）

---

### `monad-eth-txpool/src/pool/tracked/limits.rs`

**作用**

- 维护 txpool 容量/水位线与 expiry 策略（插入阶段与驱逐阶段可能用不同的 expiry）。

`TrackedTxLimitsConfig` 定义了 txpool 的容量限制和过期策略：

| 配置项 | 类型 | 默认值 | 作用说明 |
|--------|------|--------|----------|
| `max_addresses` | `usize` | `16 * 1024` (16K) | 交易池中允许的最大唯一地址数。设计考虑：为生产 5k tx 的区块，需要至少 15k 地址（因为可能在最近提交的区块更新中清理掉 5k 地址，在 pending blocktree 中清理掉 5k 地址，仍保留至少 5k 其他地址用于创建下一个区块） |
| `max_txs` | `usize` | `64 * 1024` (64K) | 交易池中允许的最大交易总数。与 `max_addresses` 配合，控制池的整体容量 |
| `max_eip2718_bytes` | `u64` | `4 * 1024 * 1024 * 1024` (4GiB) | 交易池中所有交易的 EIP-2718 编码总字节数上限。用于限制内存占用，防止超大交易或大量交易导致内存溢出 |
| `soft_evict_addresses_watermark` | `usize` | `DEFAULT_MAX_ADDRESSES - 512` (15.5K) | 软驱逐水位线。**注意**：虽然名称包含 "addresses"，但实际比较的是 **tx 数量**（`limits.txs`）。当交易数达到此水位线时，驱逐阶段会切换到更激进的过期策略（`soft_tx_expiry`） |
| `soft_tx_expiry` | `Duration` | 必须指定 | 软交易过期时间。当交易数达到 `soft_evict_addresses_watermark` 时，在驱逐阶段使用此更短的过期时间，以更激进地清理过期交易，释放池容量 |
| `hard_tx_expiry` | `Duration` | 必须指定 | 硬交易过期时间。正常情况下使用的过期时间，在插入阶段和未达到水位线时的驱逐阶段使用 |

**过期策略机制**：

- **插入阶段**：始终使用 `hard_tx_expiry`（通过 `expiry_duration_during_insert()`）
- **驱逐阶段**：
  - 当 `limits.txs < soft_evict_addresses_watermark` 时，使用 `hard_tx_expiry`
  - 当 `limits.txs >= soft_evict_addresses_watermark` 时，切换到 `soft_tx_expiry` 并记录 info 日志

**重点类型/函数**

- **`TrackedTxLimitsConfig::new(...)`**
  - 默认：
    - `max_addresses = 16 * 1024`
    - `max_txs = 64 * 1024`
    - `max_eip2718_bytes = 4GiB`
    - `soft_evict_addresses_watermark = DEFAULT_MAX_ADDRESSES - 512`
  - `soft_tx_expiry / hard_tx_expiry`：分别用于"压力大时更激进驱逐"和"正常情况下"的过期策略。

- **`TrackedTxLimits::is_exceeding_limits(addresses)`**
  - 超过任一限制即 true：地址数、tx 数、总字节数。

- **`expiry_duration_during_evict()`**
  - 当 `txs` 达到 `soft_evict_addresses_watermark` 后，会切换到 `soft_tx_expiry` 并打 info log。
  - > **注意**：此处比较的是 **tx 数量**（`limits.txs`），配置名虽为 `addresses_watermark`，实现用的是 tx 计数。

- **`add_tx/remove_tx/remove_txs/reset`**
  - **语义**：维护聚合计数（tx 数与 eip2718_bytes），并在 underflow 时打 error 并自愈为 0（防止 panic）。

---

### `monad-eth-txpool/src/pool/tracked/priority.rs`

**作用**

- 当交易池（txpool）容量达到上限时，系统需要决定**哪些地址的交易应该被驱逐**，为新交易腾出空间。这需要一个公平且高效的优先级机制。

**`Priority`**：

```rust
pub(super) struct Priority {
    pub nonce_gap: u64,
    pub tips: [u64; 7],
}
```

nonce_gap：衡量"可执行性"

**定义**：`nonce_gap = 第一笔交易的 nonce - account_nonce`

**含义**：
- `nonce_gap = 0`：地址的交易队列是连续的，从 `account_nonce` 开始，可以立即执行
- `nonce_gap > 0`：存在断档，比如 `account_nonce = 5`，但池中最早的是 `nonce = 8` 的交易，那么 `nonce_gap = 3`
- `nonce_gap = u64::MAX`：该地址在池中没有任何交易（`txs` 为空）

tips[7]：衡量"经济价值"

**定义**：取该地址**前 7 笔可执行交易**的 `effective_tip_per_gas`，然后转换为反向值：

```rust
tips[i] = u64::MAX - effective_tip_per_gas
```

在 Rust 的 `Ord` trait 中，`Priority` 的比较规则是：
1. 先比较 `nonce_gap`（越小越好）
2. 再比较 `tips` 数组（按字典序，越小越好）

PriorityMap：双重数据结构设计

```rust
pub struct PriorityMap {
    by_address: HashMap<Address, (Priority, Instant)>,  // O(1) 查找
    sorted: BTreeSet<(Priority, Instant, Address)>,      // O(log n) 排序
}
```

**`by_address`（HashMap）**：
- 快速查找：给定地址，立即知道其优先级
- 用途：更新优先级时，需要先删除旧记录

**`sorted`（BTreeSet）**：
- 自动排序：按 `(Priority, Instant, Address)` 三元组排序
- 用途：快速找到"最应该被驱逐"的地址（排序最大的）

**`Instant` 的作用**：
- 作为 tie-breaker：当两个地址的 `Priority` 完全相同时，用时间戳区分
- **重要**：首次插入时用 `event_tracker.now`，后续更新时**保留原时间戳**
- 这样保证了"先来后到"的公平性：相同优先级时，后加入的更容易被驱逐

完整示例：理解优先级排序

### 场景

假设有 3 个地址：

**地址 A**：
- `account_nonce = 5`
- 池中交易：`nonce = 5, 6, 7`（连续）
- tip：`[100, 100, 100]`（高 tip）

**地址 B**：
- `account_nonce = 10`
- 池中交易：`nonce = 15, 16`（断档 = 5）
- tip：`[50, 50]`（低 tip）

**地址 C**：
- `account_nonce = 20`
- 池中交易：`nonce = 20, 21`（连续）
- tip：`[30, 30]`（最低 tip）

### 计算 Priority

**地址 A**：
- `nonce_gap = 5 - 5 = 0`
- `tips = [u64::MAX - 100, u64::MAX - 100, u64::MAX - 100, u64::MAX, ...]`
- Priority = `(0, [很小的值, ...])`

**地址 B**：
- `nonce_gap = 15 - 10 = 5`
- `tips = [u64::MAX - 50, u64::MAX - 50, u64::MAX, ...]`
- Priority = `(5, [中等值, ...])`

**地址 C**：
- `nonce_gap = 20 - 20 = 0`
- `tips = [u64::MAX - 30, u64::MAX - 30, u64::MAX, ...]`
- Priority = `(0, [较大的值, ...])`

### 排序结果（从小到大）

1. **地址 A**：`nonce_gap = 0` 最小，且 `tips` 最小（tip 最高）
2. **地址 C**：`nonce_gap = 0`，但 `tips` 比 A 大（tip 较低）
3. **地址 B**：`nonce_gap = 5` 最大

### 驱逐顺序

当需要驱逐时，`pop_last()` 会按以下顺序选择：
1. **地址 B**（最优先被驱逐：断档大）
2. **地址 C**（其次：tip 低）
3. **地址 A**（最后：连续且 tip 高）

实际驱逐流程

在 `TrackedTxMap::try_insert_txs` 中：

```rust
while self.limits.is_exceeding_limits(self.txs.len()) {
    let Some(removal_address) = self.priority.pop_eviction_address() else {
        error!("cannot find eviction address but exceeding limits");
        self.reset();
        return;
    };

    // 整组驱逐该地址的所有交易
    TrackedTxList::evict_pool_full(event_tracker, &mut self.limits, o);
}
```

**注意**：驱逐是**整组驱逐**，即删除该地址的所有交易，而不是只删除一部分。