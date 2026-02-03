// Demo: 通过 IPC 线性生成地址线性向 Monad Devnet 发送交易
//
// 用法:
//   1. 确保 devnet 节点已启动，并获取 mempool IPC 路径
//   2. 运行: cargo run --package monad-eth-testutil --example rpc-tx-multi-addr-sender <address_count> <num_txs>

use monad_eth_testutil::{make_legacy_tx_with_chain_id, secret_to_eth_address};
use monad_eth_txpool_ipc::EthTxPoolIpcClient;
use alloy_primitives::B256;
use futures::SinkExt;
use std::path::PathBuf;

// 基础私钥（用于派生）
const BASE_SECRET: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // ========== Step 1: 获取参数 ==========
    let args: Vec<String> = std::env::args().collect();
    // 调整为本地内存池文件路径
    let ipc_path = "/Users/lewis/RustroverProjects/monad-bft-xyz/monad-bull/config/mempool.sock".to_string();
    // 默认
    let chain_id: u64 = 20143;
    // 命令行显示传入的第 1 个参数 - 交易地址数量，默认1
    let address_count: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    // 命令行显示传入的第 2 个参数 - 每个地址发送的交易数量，默认1
    let num_txs: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    // 验证 address_count 不超过 10000
    let address_count = std::cmp::min(address_count, 10000);

    // 验证路径存在
    if !PathBuf::from(&ipc_path).exists() {
        eprintln!("错误: IPC socket 不存在: {}", ipc_path);
        std::process::exit(1);
    }

    // ========== Step 2: 并发生成 address_count 个不同的 secret ==========
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
                (i, derived_secret, sender)
            })
        })
        .collect();

    // 等待所有任务完成并收集结果
    let mut results: Vec<(usize, B256, alloy_primitives::Address)> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();

    // 按索引排序以保持确定性顺序
    results.sort_by_key(|(idx, _, _)| *idx);

    // 提取 secrets 和打印地址
    let mut secrets: Vec<B256> = Vec::with_capacity(results.len());
    for (i, derived_secret, sender) in &results {
        secrets.push(*derived_secret);
        println!("   [{}] 0x{:?}", i, sender);
    }

    println!("\n");
    println!("🔐 生成 {}/{} 个发送方地址:", secrets.len(), address_count);

    // ========== Step 3: 创建 IPC 客户端 ==========
    let (mut client, snapshot) = EthTxPoolIpcClient::new(&ipc_path).await?;
    println!("\n🔗 已连接 mempool，交易池快照: {} 笔待处理交易", snapshot.txs.len());

    // ========== Step 4: 生成并发送交易 ==========
    let total_txs = address_count * num_txs;
    println!("\n开始发送交易: {} 个地址 × {} 笔 = {} 笔交易", address_count, num_txs, total_txs);

    // 交易参数
    let gas_price: u128 = 100_000_000_000u128.into();
    let gas_limit: u64 = 21_000;
    let input_len: usize = 0;

    // TODO 支持并发发送交易
    let mut tx_index = 0;
    for (addr_idx, &secret) in secrets.iter().enumerate() {
        let sender = secret_to_eth_address(secret);
        print!("\n📤 地址 [{}] 0x{:?} 开始发送 {} 笔交易...", addr_idx, sender, num_txs);

        for nonce in 0..num_txs {
            let tx = make_legacy_tx_with_chain_id(
                secret,
                gas_price,
                gas_limit,
                nonce as u64,
                input_len,
                chain_id,
            );
            client.send(&tx).await?;
            tx_index += 1;

            if tx_index % 10 == 0 || tx_index == total_txs {
                println!("   已发送 {} / {} 笔交易", tx_index, total_txs);
            }
        }
    }

    // ========== Step 5: 刷新 ==========
    client.flush().await?;
    println!("\n✅ 已完成: 共发送 {} 笔交易 ({} 个地址 × {} 笔)", total_txs, address_count, num_txs);

    Ok(())
}