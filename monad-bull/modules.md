# 模块

## 共识与区块产生

| 模块 | 功能 |
| :--- | :--- |  
| monad-consensus | 共识核心引擎 - 收集交易、生产区块、实现 BFT 共识算法 |
| monad-consensus-state | 共识状态管理 |
| monad-consensus-types | 共识状态相关的类型定义 |
| monad-validator | 验证者相关功能（签名收集、验证者集管理、领导者选举） |
| monad-system-calls | 系统调用生成与验证（验证者奖励、验证者集变更等） |

## 数据存储

| 模块 | 功能 |
| :--- | :--- |  
| monad-triedb | Merkle Trie 数据库 - 存储区块信息和区块链状态 |
| monad-tiredb-cache | Tire缓存层 |
| monad-triedb-utils | Tire工具函数 |
| ***monad-blocktree | 区块树数据结构 |
| monad-block-persist | 区块持久化 |
| monad-blocksync | 区块同步 |
| monad-statsync | 状态同步 |
| monad-archive | 区块归档功能 |
| monad-ledger | 账本管理 |
| monad-wal | Write-Ahead Log（预写日志） |

## 以太坊兼容层

| 模块 | 功能 |
| :--- | :--- |
| ***monad-eth-txpool | 交易池 - 管理待处理的以太坊交易 |
| monad-eth-txpool-types | 交易池类型定义 |
| ***monad-eth-txpool-executor | 交易池执行器 |
| monad-eth-txpool-ipc | 交易池 IPC 接口 |
| *****monad-eth-block-policy | 区块策略（Gas 计算、区块限制） |
| *****monad-eth-block-validator | 区块验证 |
| monad-eth-ledger | 以太坊账本 |
| monad-eth-types | 以太坊相关类型定义 |
| monad-ethcall | 以太坊调用封装 |
| monad-eth-testutil | 以太坊测试工具 |

## 网络层

| 模块 | 功能 |
| :--- | :--- |
| ***monad-peer-discovery | 对等节点发现（DNS 种子、Kademlia 等） |
| monad-peer-disc-swarm | 对等节点 Swarm 管理 |
| monad-raptor | RaptorQ 纠删码 - 高效数据传输 |
| monad-raptorcast | 基于 Raptor 的广播协议 - 区块/投票传播 |
| monad-wireauth | 消息认证（签名验证） |
| monad-router-multi | 多路路由器 |
| monad-router-scheduler | 路由调度器 |
| monad-mock-swarm | Mock 网络用于测试 |

## 加密与安全

| 模块 | 功能 |
| :--- | :--- |
| monad-crypto | 加密原语 |
| monad-bls | BLS 签名算法 |
| monad-secp | secp256k1 椭圆曲线（以太坊密钥） |
| monad-keystore | 密钥库管理（本地密钥安全存储） |
| monad-multi-sig | 多重签名 |

## 状态管理

| 模块 | 功能 |
| :--- | :--- |
| monad-state | 状态管理 - 管理区块链世界状态 |
| monad-state-backend | 状态后端接口 |

## RPC 服务

| 模块 | 功能 |
| :--- | :--- |
| monad-rpc | JSON-RPC 服务器 - 提供以太坊兼容 API |

## 配置与管理

| 模块 | 功能 |
| :--- | :--- |
| monad-node | 完整节点程序入口 |
| monad-node-config | 节点配置解析 |
| monad-chain-config | 区块链配置（链 ID、Epoch 长度等） |
| monad-control-panel | 控制面板 IPC |
| monad-debug-node | 调试节点 |
| monad-debugger | 调试器前端/后端 |
| monad-bull | 性能测试工具（Bull 基准测试） |

## 工具与实用程序

| 模块 | 功能 |
| :--- | :--- |
| monad-merkle | Merkle 树实现 |
| monad-compress | 压缩库（zstd、brotli） |
| monad-event-ring | 事件环（高性能队列） |
| monad-testutil | 测试工具函数 |
| monad-testground | 测试框架 |
| monad-randomized-tests | 随机测试 |
| monad-twins | 双胞胎测试工具（对比两个节点行为） |
| monad-pprof | 性能分析（pprof 集成） |
| monad-perf-util | 性能工具 |
| monad-version | 版本管理 |
| monad-updaters | 更新相关功能 |
| monad-transformer | 转换器 |
| monad-tfm | 交易费用市场（Transaction Fee Market） |

## 公共类型

| 模块 | 功能 |
| :--- | :--- |
| monad-types | 公共类型定义（Epoch、Round、SeqNum、NodeId 等） |

## 编译与 FFI

| 模块 | 功能 |
| :--- | :--- |
| monad-cxx | Rust/C++ FFI 绑定 |
| monad-system-calls | 系统调用接口 |

