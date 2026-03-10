# 查看日志
export RUST_LOG=info

# --debug 查看详细日志
# --package
# --bin
# --validators-path 验证者配置文件
# --secp-identity Secp256k1 密钥文件，节点身份密钥（P2P 网络认证）
# --bls-identity BLS 密钥文件，BLS 签名密钥（共识签名）
# --node-config 节点配置文件
# --forkpoint-config 共识起始状态
# --wal-path WAL 日志
# --mempool-ipc-path 内存池 IPC 路径
# --control-panel-ipc-path 控制面板 IPC 路径
# --ledger-path 账本路径
# --statesync-ipc-path 状态同步 IPC 路径
cargo run \
--package monad-bull \
--bin monad-bull \
-- \
--validators-path monad-bull/config/validators.toml \
--secp-identity monad-bull/config/id-secp \
--bls-identity monad-bull/config/id-bls \
--node-config monad-bull/config/node.toml \
--forkpoint-config monad-bull/config/forkpoint.toml \
--wal-path monad-bull/config/wal \
--mempool-ipc-path monad-bull/config/mempool.sock \
--control-panel-ipc-path monad-bull/config/controlpanel.sock \
--ledger-path monad-bull/config/ledger \
--statesync-ipc-path monad-bull/config/statesync.sock