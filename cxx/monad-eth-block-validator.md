# 以太坊区块验证模块 (monad-eth-block-validator)

## 模块概述

`monad-eth-block-validator` 是 Monad BFT 项目中负责**以太坊区块验证**的核心模块。它实现了 `BlockValidator` trait，负责验证共识层提交的区块是否符合以太坊的规则，产生 `EthValidatedBlock` 供后续的策略检查使用。

### 核心职责

1. **区块头验证**：验证区块头与区块体的一致性、检查签名、验证以太坊特定字段
2. **区块体验证**：验证交易列表、计算手续费、追踪 nonce 使用
3. **交易静态验证**：验证单个交易的签名、Gas 限制、EIP 合规性
4. **输出验证结果**：生成 `EthValidatedBlock` 供 `EthBlockPolicy` 使用

---

## 核心类型

### EthBlockValidator<ST, SCT> - 以太坊区块验证器

**定义位置**：`lib.rs:69-72`

```rust
pub struct EthBlockValidator<ST, SCT>(PhantomData<(ST, SCT)>)
where
    ST: CertificateSignatureRecoverable,
    SCT: SignatureCollection<NodeIdPubKey = CertificateSignaturePubKey<ST>>;
```

**职责**：验证以太坊兼容区块的完整性和有效性

**特点**：

- 无状态验证器，所有状态通过方法参数传入
- 泛型参数支持不同的签名方案和签名集合
- 实现 `BlockValidator` trait，与共识层解耦

---

## 验证流程

### 整体流程 (`validate` 方法)

**签名**：`lib.rs:103-174`

```rust
fn validate(
    &self,
    header: ConsensusBlockHeader<ST, SCT, EthExecutionProtocol>,
    body: ConsensusBlockBody<EthExecutionProtocol>,
    author_pubkey: Option<&SignatureCollectionPubKeyType<SCT>>,
    chain_config: &CCT,
    metrics: &mut Metrics,
) -> Result<EthValidatedBlock<ST, SCT>, EthBlockValidationError>
```

**验证步骤**：

```txt
┌─────────────────────────────────────────────────────────────┐
│                    validate()                                │
├─────────────────────────────────────────────────────────────┤
│  1. validate_block_header()                                  │
│     ├── 检查 header.body_id == body.get_id()                 │
│     ├── 验证 Randao 签名 (author_pubkey)                     │
│     └── 验证所有以太坊头部字段                                │
├─────────────────────────────────────────────────────────────┤
│  2. validate_block_body()                                    │
│     ├── 检查 ommers/withdrawals 为空                         │
│     ├── 检查交易数量限制                                      │
│     ├── 检查总 gas 限制                                      │
│     ├── 恢复交易签名者                                        │
│     ├── 提取系统交易 vs 用户交易                              │
│     ├── 静态验证每笔交易                                      │
│     ├── 验证 nonce 连续性                                    │
│     ├── 处理 EIP-7702 授权列表                               │
│     └── 计算 TxnFees                                         │
├─────────────────────────────────────────────────────────────┤
│  3. 构建 EthValidatedBlock                                   │
│     ├── 组合 header + body = ConsensusFullBlock              │
│     ├── 附加 system_txns, validated_txns                     │
│     ├── 附加 nonce_usages, txn_fees                          │
└─────────────────────────────────────────────────────────────┘
```

---

## 区块头验证详解 (`validate_block_header`)

**签名**：`lib.rs:196-311`

验证以下所有字段：

### 1. 基础一致性检查

```txt

| 检查项 | 错误类型 | 说明 |
|--------|----------|------|
| Header-Body ID 匹配 | `HeaderError::HeaderPayloadMismatch` | 确保区块头引用的区块体是实际的区块体 |
| Randao 签名 | `HeaderError::RandaoError` | 验证区块提议者的随机数签名 |
```

### 2. 以太坊 PoS 兼容性检查（必须满足）

```txt
| 字段 | 预期值 | 错误类型 | 说明 |
|------|--------|----------|------|
| `ommers_hash` | `EMPTY_OMMER_ROOT_HASH` | `NonEmptyOmmersHash` | 合并后不再使用叔块 |
| `withdrawals_root` | `EMPTY_WITHDRAWALS` | `NonEmptyWithdrawalsRoot` | 当前未启用提款功能 |
| `difficulty` | `0` | `NonZeroDifficulty` | PoS 链难度为 0 |
| `number` | `header.seq_num.0` | `InvalidHeaderNumber` | 区块号必须与共识序号一致 |
| `gas_limit` | `chain_params.proposal_gas_limit` | `InvalidGasLimit` | Gas 限制必须符合链配置 |
| `timestamp` | `header.timestamp_ns / 1_000_000_000` | `InvalidTimestamp` | 时间戳必须一致（秒级） |
| `mix_hash` | `header.round_signature.get_hash().0` | `InvalidRoundSignatureHash` | 必须等于轮次签名的哈希 |
| `nonce` | `[0_u8; 8]` | `NonEmptyHeaderNonce` | PoS 链 nonce 为 0 |
| `extra_data` | `[0_u8; 32]` | `NonEmptyExtraData` | 必须为零填充 |
| `blob_gas_used` | `0` | `NonZeroBlockGasUsed` | 当前未启用 blob 交易 |
| `excess_blob_gas` | `0` | `NonZeroExcessBlobGas` | 当前未启用 blob 交易 |
| `parent_beacon_block_root` | `[0_u8; 32]` | `NonEmptyParentBeaconRoot` | 当前未启用信标链功能 |
```

### 3. 交易根验证

```txt
| 检查项 | 说明 |
|--------|------|
| `transactions_root` | 必须等于 `calculate_transaction_root(body.execution_body.transactions)` |
```

### 4. Prague 兼容性

```txt
| 检查项 | 说明 |
|--------|------|
| `requests_hash` | 如果 `prague_enabled`，必须为 `[0_u8; 32]`（Monad 暂不使用请求哈希） |
```

---

## 区块体验证详解 (`validate_block_body`)

**签名**：`lib.rs:347-650`

### 1. 基础结构验证

```rust
// 检查叔块和提款列表必须为空
if !ommers.is_empty() { return Err(PayloadError::NonEmptyOmmers(...)); }
if !withdrawals.is_empty() { return Err(PayloadError::NonEmptyWithdrawals(...)); }
```

### 2. 区块限制检查

```txt
| 检查项 | 配置项 | 错误类型 |
|--------|--------|----------|
| 交易数量 | `chain_params.tx_limit` | `PayloadError::ExceededNumTxnLimit` |
| 总 Gas 消耗 | `chain_params.proposal_gas_limit` | `PayloadError::ExceededBlockGasLimit` |
| 提案大小 | `chain_params.proposal_byte_limit` | `PayloadError::ExceededBlockSizeLimit` |
```

### 3. 交易签名恢复

```rust
let recovered_txns: VecDeque<Recovered<TxEnvelope>> = transactions
    .into_par_iter()
    .map(|tx| {
        let signer = tx.secp256k1_recover()?;
        Ok(Recovered::new_unchecked(tx.clone(), signer))
    })
    .collect()
```

- 使用 **并行迭代** 加速处理
- 验证 secp256k1 签名有效性
- 失败时返回 `TxnError::SignerRecoveryError`

### 4. 系统交易提取

```rust
let (system_txns, eth_txns) = SystemTransactionValidator::extract_system_transactions(
    header,
    recovered_txns,
    chain_config,
)?;
```

- 系统交易优先处理
- 验证系统交易的签名和格式
- 返回 `SystemTransactionValidationError`

### 5. 交易静态验证

```rust
for eth_txn in eth_txns.iter() {
    static_validate_transaction(eth_txn, chain_id, chain_params, execution_chain_params)?;
}
```

调用 `monad-eth-block-policy` 的验证逻辑，验证：

- Chain ID 有效性
- EIP-1559 参数正确性
- Init code 大小限制
- Gas 限制检查（intrinsic, floor data gas, TFM, proposal）
- 签名有效性
- 授权列表长度限制

### 6. Nonce 连续性验证

```rust
// 系统交易
for sys_txn in &system_txns {
    nonce_usages.add_known(sys_txn.signer(), sys_txn.nonce())?;
}

// 用户交易
for eth_txn in validated_txns.iter() {
    let maybe_old_nonce_usage = nonce_usages.add_known(eth_txn.signer(), eth_txn.nonce())?;
    // 如果已有 nonce，必须是递增的
}
```

**关键规则**：

- 同一发送者的交易 nonce 必须严格递增
- 不能有间隔（gap）
- 失败返回 `TxnError::InvalidNonce`
- nonce 溢出返回 `TxnError::NonceOverflow`

### 7. EIP-7702 授权处理(可跳过)

```rust
if eth_txn.is_eip7702() {
    for recovered_auth in eth_txn.authorizations_7702.iter() {
        // 验证授权不是系统账户
        if authority == SYSTEM_SENDER_ETH_ADDRESS {
            return Err(TxnError::InvalidSystemAccountAuthorization);
        }

        // 标记授权账户为 delegated
        txn_fees.entry(authority).and_modify(|e| e.is_delegated = true);

        // 验证授权 nonce 连续性
        // 只跟踪链 ID 匹配的授权
        if recovered_auth.chain_id() == 0 || recovered_auth.chain_id() == chain_id {
            nonce_usages.add_known(authority, recovered_auth.nonce())?;
        }
    }
}
```

### 8. 交易费用计算(可跳过)

```rust
let block_base_fee = header.base_fee.unwrap_or(PRE_TFM_BASE_FEE);

// 验证 max_fee >= base_fee
if eth_txn.max_fee_per_gas() < block_base_fee.into() {
    return Err(TxnError::MaxFeeLessThanBaseFee);
}

// 计算费用
txn_fees.entry(eth_txn.signer()).or_insert(TxnFee {
    first_txn_value: eth_txn.value(),
    first_txn_gas: compute_txn_max_gas_cost(eth_txn, block_base_fee),
    max_gas_cost: compute_txn_max_gas_cost(eth_txn, block_base_fee),
    max_txn_cost: pre_tfm_compute_max_txn_cost(eth_txn),
    is_delegated: false,
    delegation_before_first_txn: false,
});
```

**TxnFee 结构**：

```rust
pub struct TxnFee {
    pub first_txn_value: Balance,      // 第一笔交易的价值
    pub first_txn_gas: Balance,        // 第一笔交易的 gas 成本
    pub max_gas_cost: Balance,         // 最大 gas 成本
    pub max_txn_cost: Balance,         // 最大交易成本
    pub is_delegated: bool,            // 是否已委托（EIP-7702）
    pub delegation_before_first_txn: bool, // 是否在第一笔交易前已委托
}
```

---

## 错误类型

### EthBlockValidationError (顶层错误)

```rust
pub enum EthBlockValidationError {
    HeaderError(HeaderError),           // 区块头验证失败
    PayloadError(PayloadError),         // 区块体验证失败
    SystemTxnError(SystemTransactionValidationError), // 系统交易验证失败
    TxnError(TxnError),                 // 交易验证失败
}
```

### HeaderError (区块头错误)

```txt
| 变体 | 含义 |
|------|------|
| `HeaderPayloadMismatch` | 区块头引用的区块体 ID 不匹配 |
| `RandaoError` | Randao 签名验证失败 |
| `NonEmptyOmmersHash` | Ommers 哈希非空（合并后应为 0） |
| `InvalidTransactionsRoot` | 交易根哈希不匹配 |
| `NonEmptyWithdrawalsRoot` | 提款根哈希非空 |
| `NonZeroDifficulty` | 难度值非 0（PoS 要求） |
| `InvalidHeaderNumber` | 区块号不匹配 |
| `InvalidGasLimit` | Gas 限制不符合链配置 |
| `InvalidTimestamp` | 时间戳不一致 |
| `InvalidRoundSignatureHash` | Round 签名哈希不匹配 |
| `NonEmptyHeaderNonce` | Nonce 非零 |
| `NonEmptyExtraData` | ExtraData 非零填充 |
| `NonZeroBlockGasUsed` | Blob Gas Used 非零 |
| `NonZeroExcessBlobGas` | Excess Blob Gas 非零 |
| `NonEmptyParentBeaconRoot` | 父信标区块根非零 |
| `InvalidRequestsHash` | 请求哈希不匹配（Prague） |
```

### PayloadError (区块体错误)

```txt
| 变体 | 含义 |
|------|------|
| `NonEmptyOmmers` | 叔块列表非空 |
| `NonEmptyWithdrawals` | 提款列表非空 |
| `ExceededNumTxnLimit` | 交易数量超过限制 |
| `ExceededBlockGasLimit` | 总 Gas 超过限制 |
| `ExceededBlockSizeLimit` | 提案大小超过限制 |
```

### TxnError (交易错误)

```txt
| 变体 | 含义 |
|------|------|
| `SignerRecoveryError` | 签名恢复失败 |
| `StaticValidationError` | 静态验证失败 |
| `MaxFeeLessThanBaseFee` | max_fee < base_fee |
| `InvalidSystemAccountAuthorization` | 系统账户发送 EIP-7702 授权 |
| `NonceOverflow` | Nonce 溢出 |
| `InvalidNonce` | Nonce 不连续或间隔 |
```

---

## 与其他模块的交互

```txt
monad-eth-block-validator
    │
    ├── monad-consensus-types     (BlockValidator trait, ConsensusBlockHeader, ConsensusBlockBody)
    │   │
    │   └── BlockValidator trait
    │       validate() -> EthValidatedBlock
    │
    ├── monad-eth-block-policy    (EthBlockPolicy, EthValidatedBlock)
    │   │
    │   ├── static_validate_transaction() - 交易静态验证
    │   ├── compute_txn_max_gas_cost() - 费用计算
    │   ├── pre_tfm_compute_max_txn_cost() - 最大交易成本
    │   └── EthValidatedBlock - 输出类型
    │
    ├── monad-eth-types           (EthBlockBody, ValidatedTx, ProposedEthHeader)
    │   └── EthExecutionProtocol - 执行协议标记
    │
    ├── monad-chain-config        (ChainConfig, ChainParams)
    │   └── 获取链参数用于验证
    │
    ├── monad-sec                 (secp256k1_recover)
    │   └── 交易签名恢复
    │
    └── monad-system-calls        (SystemTransactionValidator)
        └── 系统交易提取和验证
```

---

## 文件结构

```txt
monad-eth-block-validator/
├── Cargo.toml
├── src/
│   ├── lib.rs              
│   │   ├── EthBlockValidator
│   │   ├── validate() - 主入口
│   │   ├── validate_block_header() - 区块头验证
│   │   └── validate_block_body() - 区块体验证
│   │       ├── 系统交易提取
│   │       ├── 交易静态验证
│   │       ├── nonce 验证
│   │       ├── EIP-7702 处理
│   │       └── 费用计算
│   └── error.rs            (错误类型定义)
│       ├── EthBlockValidationError
│       ├── HeaderError
│       ├── PayloadError
│       └── TxnError
└── tests/
    └── ...                 (集成测试)
```

---

## 代码示例

```rust
// 创建验证器
let validator = EthBlockValidator::default();

// 验证区块
let validated_block = validator.validate(
    header,
    body,
    Some(&author_pubkey),
    &chain_config,
    &mut metrics,
)?;

match validated_block {
    Ok(block) => {
        // block 包含:
        // - block.block: ConsensusFullBlock
        // - block.system_txns: 系统交易列表
        // - block.validated_txns: 已验证的用户交易列表
        // - block.nonce_usages: Nonce 使用情况
        // - block.txn_fees: 交易费用详情
    }
    Err(error) => {
        // 处理验证错误
        match error {
            EthBlockValidationError::HeaderError(e) => { ... }
            EthBlockValidationError::PayloadError(e) => { ... }
            EthBlockValidationError::TxnError(e) => { ... }
            EthBlockValidationError::SystemTxnError(e) => { ... }
        }
    }
}
```

---

## 性能考量

1. **并行处理**：
   - 交易签名恢复使用 `rayon::par_iter`
   - EIP-7702 授权列表处理使用并行迭代

2. **早期退出**：
   - 区块限制检查先于交易验证
   - 失败时快速返回

3. **内存效率**：
   - `NonceUsageMap` 只存储涉及的地址
   - 使用 `VecDeque` 处理可能的 nonce 值

---

## 与 monad-eth-block-policy 的关系

```txt
┌──────────────────────────────────────────────────────────────────┐
│                     共识层 (Consensus)                            │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│              EthBlockValidator (本模块)                           │
│  验证: 签名、区块结构、交易格式、nonce连续性                           │
│  输出: EthValidatedBlock                                          │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│              EthBlockPolicy (monad-eth-block-policy)              │
│  检查: 区块连贯性、储备余额、清空交易检测                          │
│  提交: CommittedBlkBuffer                                         │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                     执行层 (Execution)                            │
└──────────────────────────────────────────────────────────────────┘
```

**职责划分**：

- **Validator**：验证区块结构正确性、交易格式、nonce 连续性
- **Policy**：检查账户状态相关的约束（余额、储备金、清空交易）
