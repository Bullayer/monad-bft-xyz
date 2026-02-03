// Demo: 通过 IPC 并发生成地址并发向 Monad Devnet 发送交易
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
    // 默认参数 - 调整为本地内存池文件路径
    let ipc_path = "/Users/lewis/RustroverProjects/monad-bft-xyz/monad-bull/config/mempool.sock".to_string();
    // 默认参数 - 链ID
    let chain_id: u64 = 20143;
    // 默认参数 - 使用信号量控制并发数为10
    let concurrent = 10;
    // 命令行显示传入的第 1 个参数 - 交易地址数量，默认1
    let address_count: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    // 命令行显示传入的第 2 个参数 - 每个地址发送的交易数量，默认1
    let num_txs: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    // 验证 address_count 不超过 10000
    // let address_count = std::cmp::min(address_count, 10000);

    // 验证路径存在
    if !PathBuf::from(&ipc_path).exists() {
        eprintln!("错误: IPC socket 不存在: {}", ipc_path);
        std::process::exit(1);
    }

    // ========== Step 2: 并发生成 address_count 个不同的 secret ==========
    // 使用基础私钥的低 30 字节与索引组合，生成确定性派生密钥
    let base_bytes: [u8; 32] = hex::decode(BASE_SECRET).unwrap().try_into().unwrap();

    // 开始计时
    let start_time = std::time::Instant::now();

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

                println!("   [{}] 0x{:?}", i, sender);

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

    // 提取 secrets
    let mut secrets: Vec<B256> = Vec::with_capacity(results.len());
    for (_i, derived_secret, _sender) in &results {
        secrets.push(*derived_secret);
    }

    // 结束计时并打印耗时
    let elapsed = start_time.elapsed();
    let rate = address_count as f64 / elapsed.as_secs_f64();

    println!("\n");
    println!("🔐 生成 {}/{} 个发送方地址: 耗时 {:.3}s ({:.0} addr/s)", secrets.len(), address_count, elapsed.as_secs_f64(), rate);

    // ========== Step 3: 创建 IPC 客户端 ==========
    let (client, snapshot) = EthTxPoolIpcClient::new(&ipc_path).await?;
    println!("\n🔗 已连接 mempool，交易池快照: {} 笔待处理交易", snapshot.txs.len());

    // ========== Step 4: 生成并发送交易 ==========
    let total_txs = address_count * num_txs;
    println!("\n🚀 开始发送交易: {} 个地址 × {} 笔 = {} 笔交易", address_count, num_txs, total_txs);

    // 交易参数
    let gas_price: u128 = 100_000_000_000u128.into();
    let gas_limit: u64 = 21_000;
    let input_len: usize = 0;

    // 并发信号量
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(std::cmp::min(address_count, concurrent)));
    let client = std::sync::Arc::new(tokio::sync::Mutex::new(client));

    // 开始计时
    let send_start_time = std::time::Instant::now();

    // 并发发送交易：10个用户并发，每个用户串行发送自己的交易
    let send_tasks: Vec<_> = secrets
        .into_iter()
        .enumerate()
        .map(|(addr_idx, secret)| {
            let semaphore = std::sync::Arc::clone(&semaphore);
            let client = std::sync::Arc::clone(&client);
            tokio::task::spawn(async move {
                // 获取并发许可
                let permit = semaphore.acquire().await.unwrap();

                let sender = secret_to_eth_address(secret);
                let mut local_tx_index = 0;

                for nonce in 0..num_txs {
                    let tx = make_legacy_tx_with_chain_id(
                        secret,
                        gas_price,
                        gas_limit,
                        nonce as u64,
                        input_len,
                        chain_id,
                    );
                    println!("\n✈️ processing {} 已发送 {} / {} 笔交易", sender, local_tx_index + 1, num_txs);
                    local_tx_index += 1;

                    let mut guard = client.lock().await;
                    guard.send(&tx).await?;
                    drop(guard);
                }

                drop(permit);
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((addr_idx, sender, local_tx_index))
            })
        })
        .collect();

    // 等待所有发送任务完成
    let results: Vec<Result<Result<(usize, alloy_primitives::Address, usize), Box<dyn std::error::Error + Send + Sync>>, tokio::task::JoinError>> =
        futures::future::join_all(send_tasks).await;
    for result in results {
        match result {
            Ok(Ok((addr_idx, sender, count))) => {
                println!("\n✅ [{}] 0x{:?} 完成发送 {} 笔", addr_idx, sender, count);
            }
            Ok(Err(e)) => {
                eprintln!("\n❌ 发送任务出错: {}", e);
            }
            Err(e) => {
                eprintln!("\n❌ 任务 join 失败: {}", e);
            }
        }
    }

    // 结束计时
    let send_elapsed = send_start_time.elapsed();
    let send_rate = total_txs as f64 / send_elapsed.as_secs_f64();

    // ========== Step 5: 刷新 ==========
    {
        let mut guard = client.lock().await;
        guard.flush().await?;
    }
    println!(
        "\n✅ 已完成: 共发送 {} 笔交易 ({} 个地址 × {} 笔) | 耗时 {:.3}s ({:.0} tx/s)",
        total_txs, address_count, num_txs, send_elapsed.as_secs_f64(), send_rate
    );

    Ok(())
}