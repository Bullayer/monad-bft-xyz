# 以太坊区块策略模块 (monad-eth-block-policy)

## 模块概述

`monad-eth-block-policy` 是 Monad BFT 项目中负责**以太坊区块策略管理**的核心模块。它实现了 `BlockPolicy` trait，为共识层提供以太坊特定的区块处理逻辑，包括：

- 账户 nonce 和余额状态管理
- 区块费用计算和应用
- 交易验证（nonce 检查、余额检查）
- 区块提交缓冲和状态追踪

### 核心职责

1. **状态追踪**：追踪最后 `execution_delay` 个已提交区块的 nonce 和 余额状态
2. **交易验证**：验证交易是否符合以太坊的 nonce 和余额约束
3. **费用计算**：计算并应用区块手续费到账户余额
4. **区块连贯性检查**：确保新区块与链状态一致

---

## 核心类型

### 1. EthValidatedBlock<ST, SCT> - 已验证区块

```rust
pub struct EthValidatedBlock<ST, SCT> {
    pub block: ConsensusFullBlock<ST, SCT, EthExecutionProtocol>,
    pub system_txns: Vec<ValidatedTx>,
    pub validated_txns: Vec<ValidatedTx>,
    pub nonce_usages: NonceUsageMap,
    pub txn_fees: TxnFees,
}
```

**职责**：封装经过（monad-eth-block-validator）验证的以太坊区块，包含：

- `block`: 完整的共识区块
- `system_txns`: 系统交易列表
- `validated_txns`: 已验证的用户交易列表
- `nonce_usages`: 区块中各账户的 nonce 使用情况
- `txn_fees`: 区块手续费详情

**关键方法**：

- `get_validated_txn_hashes()`: 获取已验证交易的哈希列表
- `get_total_gas()`: 计算区块总 gas 使用量

---

### 2. CommittedBlkBuffer - 已提交区块缓冲区

```rust
struct CommittedBlkBuffer<ST, SCT, CCT, CRT> {
    blocks: SortedVectorMap<SeqNum, CommittedBlock>,
    min_buffer_size: usize, // should be 2 * execution delay
    _phantom: PhantomData<(ST, SCT, fn(&CCT, &CRT))>,
}
```

**职责**：缓存最近 `2 * execution_delay` 个已提交区块，用于：

- 检测**清空交易 (emptying transactions)**
- 检查**储备余额 (reserve balance)**
- 回溯查询历史区块状态

**关键方法**：

#### `update_account_balance()`

- **签名**：`fn update_account_balance(...) -> Result<SeqNum, BlockPolicyError>`
- **职责**：根据已提交区块更新账户余额状态
- **检查范围**：
  - **清空交易检查** `[N - 2k + 2, N - k + 1)`: 检测可能耗尽账户余额的交易，在`储备余额检查`的前 k - 1 个区块
  - **储备余额检查** `[N - k + 1, N)`: 验证账户是否有足够余额支付费用

```txt
N为区块号，当 k = 3

[N-2k+2, N-k+1)   [N-k+1, N)
        │               │
   "之前2个区块"     "最近2个区块"
   有交易吗？         有交易吗？
        │               │
        ↓               ↓
   没交易 → 确认    没交易 → 可能是清空
   有交易 → 不是    有交易 → 正常交易
        │               │
        └───────┬───────┘
                ↓
          综合判断结果
```

#### `update_committed_block()`

- **签名**：`fn update_committed_block(&mut self, block: &EthValidatedBlock<ST, SCT>)`
- **职责**：将新区块添加到缓冲区，并清理过期区块
- **维护不变量**：保持至少 `min_buffer_size` 个区块（实际存储 `2 * execution_delay` 个）

**核心概念**：

- `execution_delay (k)`: 区块从提议到最终确定之间的延迟
- `emptying transaction`: 没有"先前交易"的交易，即在 `[N - k + 1, N)` 范围内没有来自同一发送者的交易
- `reserve balance`: 账户必须保留的最低余额，用于支付未来交易费用

---

### 3. EthBlockPolicyBlockValidator<CRT> - 策略验证器

**定义位置**：`lib.rs:367-376`

```rust
pub struct EthBlockPolicyBlockValidator<CRT> {
    block_seq_num: SeqNum,
    execution_delay: SeqNum,
    base_fee: u64,
    chain_revision: CRT,
    execution_chain_revision: MonadExecutionRevision,
    _phantom: PhantomData<CRT>,
}
```

**职责**：为单个区块的费用计算和交易验证提供验证逻辑

**关键方法**：

#### `try_apply_block_fees()`

- **签名**：`pub fn try_apply_block_fees(...)`
- **职责**：应用区块手续费到账户余额
- **逻辑**：
  1. 检查账户是否为"清空交易"（在 `[N - 2k + 2, N - k + 1)` 范围内无交易）
  2. 如果是清空交易，检查储备余额是否充足
  3. 应用区块费用，扣除余额

#### `try_add_transaction()`

- **签名**：`pub fn try_add_transaction(...)`
- **职责**：验证单笔交易是否符合策略约束
- **验证项**：
  - 交易签名
  - Chain ID 验证
  - Nonce 正确性
  - EIP-7702 授权列表验证

---

### 4. EthBlockPolicy<ST, SCT, CCT, CRT> - 以太坊策略实现

**定义位置**：`lib.rs:722-733`

```rust
pub struct EthBlockPolicy<ST, SCT, CCT, CRT> {
    last_commit: SeqNum,                    // 最后提交的区块序号
    committed_cache: CommittedBlkBuffer<ST, SCT, CCT, CRT>,  // 已提交区块缓存
    execution_delay: SeqNum,                // 执行延迟
}
```

**职责**：实现 `BlockPolicy` trait，提供完整的以太坊区块策略

**关键方法**：

#### `new()`

- **签名**：`pub fn new(last_commit: SeqNum, execution_delay: u64) -> Self`
- **初始化**：创建策略实例，缓冲区大小为 `2 * execution_delay`

#### `get_account_base_nonces()`

- **签名**：`pub fn get_account_base_nonces(...) -> Result<BTreeMap<&'a Address, Nonce>, StateBackendError>`
- **职责**：计算在共识块开始执行时各账户应有的 nonce 值
- **访问层级**：
  1. `extending_blocks`: 块树中的扩展块
  2. `committed_cache`: 最后 `delay` 个已提交块的 nonce 缓存
  3. triedb 的 LRU 缓存
  4. triedb 直接查询

#### `get_block_index()`

- **签名**：`fn get_block_index(...) -> Result<BlockLookupIndex, StateBackendError>`
- **职责**：查询指定序号的区块信息（BlockId, Round, Epoch, 是否最终确认）

#### `compute_account_base_balances()`

- **签名**：`pub fn compute_account_base_balances(...) -> Result<BTreeMap<&'a Address, AccountBalanceState>, BlockPolicyError>`
- **职责**：计算账户在区块执行时的基础余额状态
- **核心逻辑**：
  1. 从状态后端获取账户当前余额
  2. 计算 `base_max_reserve_balance`
  3. 遍历已提交和扩展区块，应用区块手续费
  4. 检测清空交易并验证储备余额约束

#### `get_parent_base_fee_fields()`

- **签名**：`fn get_parent_base_fee_fields<B>(&self, extending_blocks: &[B]) -> (Round, u64, u64, u64, u64)`
- **职责**：获取父区块的 base fee 相关字段，用于 EIP-1559 费用计算

#### `nonce_check_and_update()`

- **签名**：`fn nonce_check_and_update(...) -> Result<(), BlockPolicyError>`
- **职责**：验证交易的 nonce 是否与预期匹配，并更新账户 nonce
- **EIP-7702 支持**：特殊处理授权列表的 nonce 更新

---

## 费用计算详解

### 交易最大价值计算

**`pre_tfm_compute_max_txn_cost()`**

```rust
pub fn pre_tfm_compute_max_txn_cost(txn: &TxEnvelope) -> U256 {
    txn.value() + txn.gas_limit() * txn.max_fee_per_gas()
}
```

**`compute_txn_max_value()` / `compute_txn_max_gas_cost()`**

- Legacy/EIP-2930 交易：`gas_limit * max_fee_per_gas`
- EIP-1559 交易：`gas_limit * min(max_fee, base_fee + priority_fee)`

### 区块费用应用逻辑

```txt
对于每个地址 A 在区块 N 中：
1. 检查 A 在 [N-2k+2, N-k+1) 范围内是否有交易
   - 如果没有：A 是"清空交易"发送者，需要检查储备余额
2. 计算费用：fee_A = sum(txns from A in block N)
3. 应用费用：
   - if is_emptying(A) && balance_A < reserve_balance + fee_A:
     return Err(InsufficientReserveBalance)
   - balance_A -= fee_A
```

---

## 关键概念

### 1. 执行延迟 (Execution Delay, k)

- 区块从被共识接受到实际执行之间的延迟（以区块数为单位）
- 用于处理 optimistic 执行场景
- 典型值：k = 1 或 k = 2 (monad-bull 目前配置为 k = 3)

### 2. 清空交易 (Emptying Transaction)

交易 T 是清空交易的条件：

- 在 `[block_number(T) - k + 1, block_number(T))` 范围内没有来自同一发送者的交易

为什么重要：

- 发送清空交易的账户可能在下一个区块耗尽余额
- 需要额外检查储备余额是否充足

### 3. 储备余额 (Reserve Balance)

- 账户必须保留的最低余额
- 由链配置 `max_reserve_balance` 定义
- 用于确保账户始终能支付未来的交易费用
- 计算方式：`min(account_balance, max_reserve_balance)`

### 4. 区块连贯性 (Block Coherency)

新区块必须满足：

1. **序号连续**：第一个区块序号 = last_commit + 1
2. **时间单调**：区块时间戳 >= 父区块时间戳
3. **Gas 限制**：总 gas 使用 <= 父区块 gas 限制 * multiplier
4. **Nonce 正确**：所有交易的 nonce 按顺序递增
5. **EIP-7702 授权**：授权列表的 nonce 更新必须有效

---

## 与其他模块的交互

```txt
monad-eth-block-policy
    │
    ├── monad-consensus-types     (BlockPolicy trait, ConsensusFullBlock)
    │
    ├── monad-state-backend       (StateBackend - 账户状态查询)
    │
    ├── monad-chain-config        (ChainConfig, ChainRevision)
    │
    ├── monad-eth-types           (EthAccount, ValidatedTx, EthExecutionProtocol)
    │
    ├── monad-eth-block-validator (EthValidatedBlock 验证)
    │
    └── monad-system-calls        (SystemTransactionValidator)
```

---

## 代码示例

```rust
// 创建策略实例
let policy = EthBlockPolicy::new(last_commit, execution_delay);

// 获取账户基础 nonce
let nonces = policy.get_account_base_nonces(
    consensus_block_seq_num,
    state_backend,
    extending_blocks,
    addresses,
)?;

// 计算账户基础余额
let balances = policy.compute_account_base_balances(
    consensus_block_seq_num,
    state_backend,
    chain_config,
    extending_blocks,
    addresses,
)?;

// 提交区块
policy.commit_block(&validated_block)?;
```

---

## 错误处理

主要错误类型（`BlockPolicyError`）：

- `BlockNotCoherent`: 区块不连贯（nonce 不正确等）
- `InsufficientReserveBalance`: 储备余额不足
- `EmptiedAccount`: 账户余额已耗尽
- `StateBackendError`: 状态后端错误

---

## 性能考量

1. **缓存策略**：
   - `CommittedBlkBuffer` 保持 `2 * execution_delay` 个区块
   - 避免重复查询状态后端

2. **nonce_usages 优化**：
   - 只存储涉及的地址，减少内存占用
   - 使用 `NonceUsageMap` 高效合并多个区块的 nonce 状态

3. **SortedVectorMap**：
   - 区块按序号排序，便于范围查询
   - 支持高效的范围遍历
