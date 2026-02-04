// Demo: 通过 IPC 直接向 Monad Devnet 发送交易（不经过 RPC）
//
// 用法:
//   1. 确保 devnet 节点已启动，并获取 mempool IPC 路径
//   2. 运行: cargo run --package monad-eth-testutil --example rpc-tx-sender <num_txs>

use monad_eth_testutil::{make_legacy_tx_with_chain_id, secret_to_eth_address};
use monad_eth_txpool_ipc::EthTxPoolIpcClient;
use alloy_consensus::TxEnvelope;
use alloy_primitives::B256;
use futures::{SinkExt, StreamExt};
use std::path::PathBuf;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // ========== Step 1: 获取参数 ==========
    let args: Vec<String> = std::env::args().collect();
    // 调整为本地内存池文件路径
    let ipc_path = "/Users/lewis/RustroverProjects/monad-bft-xyz/monad-bull/config/mempool.sock".to_string();
    // 默认
    let chain_id: u64 = 20143;
    // 命令行显示传入的第 1 个参数 - 交易数量，默认1
    let num_txs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

    // 验证路径存在
    if !PathBuf::from(&ipc_path).exists() {
        eprintln!("错误: IPC socket 不存在: {}", ipc_path);
        std::process::exit(1);
    }

    // ========== Step 2: 生成密钥对 ==========
    // 使用 Foundry 标准测试账户 #1 (地址: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
    // 私钥: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    let secret = B256::new(hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").unwrap().try_into().unwrap());
    let sender = secret_to_eth_address(secret);
    println!("🔐 发送方地址: 0x{:?}", sender);

    // ========== Step 3: 创建 IPC 客户端 ==========
    let (mut client, snapshot) = EthTxPoolIpcClient::new(&ipc_path).await?;
    println!("✅ 已连接");
    println!("   交易池快照: {} 笔待处理交易", snapshot.txs.len());

    // ========== Step 4: 生成交易 ==========
    println!("\n生成 {} 笔交易...", num_txs);

    // 交易参数
    let gas_price: u128 = 100_000_000_000u128.into();
    let gas_limit: u64 = 21_000;
    let input_len: usize = 0;

    // ========== Step 5: 通过 IPC 批量发送交易 ==========
    println!("\n通过 IPC 发送交易...");

    for nonce in 0..num_txs {
        let tx = make_legacy_tx_with_chain_id(secret, gas_price, gas_limit, nonce as u64, input_len, chain_id);
        send_tx(&mut client, &tx, nonce).await?;
    }

    // ========== Step 6: 刷新 ==========
    client.flush().await?;
    println!("\n已发送 {} 笔交易", num_txs);

    // 监听事件（可选：等待交易被打包）
    println!("\n监听交易池事件...");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

    for _ in 0..3 {
        interval.tick().await;
        match client.next().await {
            Some(events) => {
                println!("   收到 {} 个事件", events.len());
                for event in events {
                    println!("   - {:?}", event);
                }
            }
            None => {
                println!("   连接已关闭");
                break;
            }
        }
    }

    Ok(())
}

async fn send_tx(
    client: &mut EthTxPoolIpcClient,
    tx: &TxEnvelope,
    nonce: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    client.send(tx).await?;
    if nonce % 10 == 0 {
        println!("   交易 #{} 已发送: 0x{:?}", nonce, *tx.tx_hash());
    }
    Ok(())
}