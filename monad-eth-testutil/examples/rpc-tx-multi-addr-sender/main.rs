// Demo: 通过 IPC 根据 epoch 并发生成地址并发向 Monad Devnet 发送交易
//
// 用法:
//   1. 确保 devnet 节点已启动，并获取 mempool IPC 路径
//   2. 运行: cargo run --package monad-eth-testutil --example rpc-tx-multi-addr-sender
//
// 交易策略（基于 epoch % 3）:
//   - epoch % 3 == 0: 不发送交易
//   - epoch % 3 == 1: 发送随机低负载消息（短交易、少输入数据）
//   - epoch % 3 == 2: 发送均衡高负载消息（中等交易量、适中输入数据）

use monad_eth_testutil::{make_legacy_tx_with_chain_id, secret_to_eth_address};
use monad_eth_txpool_ipc::EthTxPoolIpcClient;
use alloy_consensus::TxEnvelope;
use alloy_primitives::B256;
use futures::SinkExt;
use rand::Rng;
use serde::Deserialize;
use std::path::PathBuf;
use toml;

// 基础私钥（用于派生）
const BASE_SECRET: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

// 交易策略参数
#[derive(Clone, Debug)]
struct TxStrategy {
    /// 是否启用交易发送
    enabled: bool,
    /// 每个地址发送的交易数量
    num_txs: usize,
    /// 输入数据长度（字节）
    input_len: usize,
    /// Gas limit
    gas_limit: u64,
    /// 描述
    description: String,
}

/// 从 forkpoint.toml 读取 epoch
fn read_epoch_from_forkpoint(forkpoint_path: &String) -> Result<u64, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(forkpoint_path)?;

    // 方法1: 尝试直接使用 toml crate 解析
    if let Ok(value) = content.parse::<toml::Value>() {
        // 尝试多种路径获取 epoch
        if let Some(epoch) = value.get("high_certificate")
            .and_then(|v| v.get("Qc"))
            .and_then(|v| v.get("info"))
            .and_then(|v| v.get("epoch"))
            .and_then(|v| v.as_integer())
        {
            return Ok(epoch as u64);
        }

        if let Some(epoch) = value.get("validator_sets")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("epoch"))
            .and_then(|v| v.as_integer())
        {
            return Ok(epoch as u64);
        }
    }

    // 方法2: 使用 serde 解析为结构体（更可靠）
    #[derive(Debug, serde::Deserialize)]
    struct ForkpointToml {
        high_certificate: Option<HighCert>,
        validator_sets: Option<Vec<ValidatorSet>>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct HighCert {
        Qc: Option<QcInfo>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct QcInfo {
        info: Option<InfoBlock>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct InfoBlock {
        epoch: Option<i64>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct ValidatorSet {
        epoch: Option<i64>,
    }

    let forkpoint: ForkpointToml = toml::from_str(&content)?;

    if let Some(high_cert) = forkpoint.high_certificate {
        if let Some(qc) = high_cert.Qc {
            if let Some(info) = qc.info {
                if let Some(epoch) = info.epoch {
                    return Ok(epoch as u64);
                }
            }
        }
    }

    if let Some(validator_sets) = forkpoint.validator_sets {
        if let Some(first) = validator_sets.first() {
            if let Some(epoch) = first.epoch {
                return Ok(epoch as u64);
            }
        }
    }

    Err("❌ 无法在 forkpoint.toml 中找到 epoch".into())
}

/// 根据 epoch 获取交易策略
fn get_tx_strategy(epoch: u64) -> TxStrategy {
    match epoch % 3 {
        0 => TxStrategy {
            enabled: false,
            num_txs: 0,
            input_len: 0,
            gas_limit: 21_000,
            description: "跳过（epoch % 3 == 0）".to_string(),
        },
        1 => TxStrategy {
            enabled: true,
            num_txs: rand::thread_rng().gen_range(1..=300),
            input_len: 16,  // 短输入数据
            gas_limit: 50_000,  // 低负载
            description: "低负载（epoch % 3 == 1）".to_string(),
        },
        _ => TxStrategy {  // 2
            enabled: true,
            num_txs: rand::thread_rng().gen_range(1000..=5000),
            input_len: 256,  // 适中输入数据
            gas_limit: 100_000,  // 高负载
            description: "高负载（epoch % 3 == 2）".to_string(),
        },
    }
}

async fn generate_secrets(address_count: usize) -> Vec<(alloy_primitives::Address, B256)> {

    // 开始计时
    let start_time = std::time::Instant::now();

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

                println!("   [{}] 0x{:?}", i, sender);

                (sender, derived_secret)
            })
        })
        .collect();

    // 等待所有任务完成并收集结果
    let results: Vec<(alloy_primitives::Address, B256)> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect();

    // 结束计时并打印耗时
    let elapsed = start_time.elapsed();
    let rate = address_count as f64 / elapsed.as_secs_f64();

    println!("\n");
    println!("🔐 生成 {}/{} 个发送方地址: 耗时 {:.3}s ({:.0} addr/s)", results.len(), address_count, elapsed.as_secs_f64(), rate);

    results
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    // ========== Step 1: 获取参数 ==========
    // 默认参数 - 项目根目录
    let base_path = "/Users/lewis/RustroverProjects/monad-bft-xyz/monad-bull/config";
    // 默认参数 - 调整为本地内存池文件路径
    let ipc_path = format!("{}/mempool.sock", base_path);
    // 默认参数 - 共识起始状态文件路径
    let forkpoint_path = format!("{}/forkpoint.toml", base_path);

    // 默认参数 - 链ID
    let chain_id: u64 = 20143;
    // 默认参数 - 5000
    let address_count: usize = 5000;

    // 验证 address_count 不超过 10000
    // let address_count = std::cmp::min(address_count, 10000);

    // 验证路径存在
    if !PathBuf::from(&ipc_path).exists() {
        eprintln!("错误: IPC socket 不存在: {}", ipc_path);
        std::process::exit(1);
    }

    // ========== Step 2: 并发生成 address_count 个不同的 secret ==========
    let secrets = generate_secrets(address_count).await;

    // ========== Step 3: 创建 IPC 客户端 ==========

    // ========== Step 3.5: 读取 epoch 并确定交易策略 ==========
    let epoch = read_epoch_from_forkpoint(&forkpoint_path)?;
    let strategy = get_tx_strategy(epoch);

    println!("\n📊 Epoch: {}", epoch);
    println!("📋 交易策略: {}", strategy.description);

    // 如果策略禁用交易发送，则直接退出
    if !strategy.enabled {
        println!("\n🛑 策略禁用交易发送，程序退出");
        return Ok(());
    }

    // ========== Step 4: 生成并发送交易 ==========
    let (mut client, snapshot) = EthTxPoolIpcClient::new(&ipc_path).await?;
    println!("\n🔗 已连接 mempool，交易池快照: {} 笔待处理交易", snapshot.txs.len());

    let total_txs = strategy.num_txs;
    println!("\n🚀 执行 {} 策略， 发送交易: 总计 {} 笔 (输入数据: {} bytes, gas_limit: {})\n",
        strategy.description, total_txs, strategy.input_len, strategy.gas_limit);

    // 交易参数（使用策略配置）
    let gas_price: u128 = 100_000_000_000u128.into();
    let input_len = strategy.input_len;
    let gas_limit = strategy.gas_limit;

    // 构建地址池：地址 -> (secret, nonce)
    // 使用 Arc<Mutex> 保证线程安全
    let address_pool: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<alloy_primitives::Address, (B256, u64)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(secrets.into_iter().map(|(addr, secret)| (addr, (secret, 0))).collect()));

    // 开始计时
    let send_start_time = std::time::Instant::now();

    // 预收集地址列表
    let addresses: Vec<alloy_primitives::Address> = address_pool.lock().unwrap().keys().cloned().collect();

    if total_txs > addresses.len() {
        eprintln!("错误: 交易数量大于地址数量");
        std::process::exit(1);
    }

    // 批量发送交易
    let mut success_count = 0;
    let mut error_count = 0;

    // 批量参数配置
    let batch_size = std::env::var("TX_BATCH_SIZE")
        .unwrap_or_else(|_| "4000".to_string())
        .parse::<usize>()
        .unwrap_or(10);
    let batch_timeout_secs = std::env::var("TX_BATCH_TIMEOUT")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
        .unwrap_or(30);
    let max_retries = std::env::var("TX_MAX_RETRIES")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .unwrap_or(3);
    let retry_delay_ms = std::env::var("TX_RETRY_DELAY_MS")
        .unwrap_or_else(|_| "1000".to_string())
        .parse::<u64>()
        .unwrap_or(1000);

    println!("\n🚀 开始批量发送 - 每批: {} 笔, 超时: {} 秒, 重试: {} 次\n", batch_size, batch_timeout_secs, max_retries);

    for batch_start in (0..total_txs).step_by(batch_size) {
        let batch_end = std::cmp::min(batch_start + batch_size, total_txs);
        let batch_size_actual = batch_end - batch_start;

        // 收集批次交易
        let mut batch_txs: Vec<TxEnvelope> = Vec::with_capacity(batch_size_actual);
        let mut batch_senders: Vec<(alloy_primitives::Address, u64)> = Vec::with_capacity(batch_size_actual);

        for tx_idx in batch_start..batch_end {
            let sender = addresses[tx_idx];

            // 获取并递增 nonce
            let (secret, nonce) = {
                let mut pool = address_pool.lock().unwrap();
                let (secret, nonce) = pool.get_mut(&sender).expect("地址不存在");
                let current_nonce = *nonce;
                *nonce += 1;
                (*secret, current_nonce)
            };

            // 构建交易
            let tx = make_legacy_tx_with_chain_id(
                secret,
                gas_price,
                gas_limit,
                nonce,
                input_len,
                chain_id,
            );

            batch_txs.push(tx);
            batch_senders.push((sender, nonce));
        }

        // 带重试的批量发送
        let mut send_success = false;
        let mut last_error = None;

        for retry in 0..=max_retries {
            // 检测是否需要重新连接（Broken pipe 等连接错误）
            let need_reconnect = retry > 0 ||
                last_error.as_ref().map(|e: &String| e.contains("Broken pipe") || e.contains("Connection reset")).unwrap_or(false);

            if need_reconnect {
                println!("🔄 批次 [{}-{}] 尝试重新连接... (重试 {}/{})", batch_start, batch_end - 1, retry, max_retries);
                tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms * (retry as u64 + 1))).await;

                // 重新连接
                match EthTxPoolIpcClient::new(&ipc_path).await {
                    Ok((new_client, snapshot)) => {
                        client = new_client;
                        println!("✅ 重新连接成功, 快照: {} 笔待处理交易", snapshot.txs.len());
                    }
                    Err(e) => {
                        last_error = Some(format!("重新连接失败: {}", e));
                        continue;
                    }
                }
            }

            // 尝试发送
            let send_result = tokio::time::timeout(
                std::time::Duration::from_secs(batch_timeout_secs),
                async {
                    for tx in &batch_txs {
                        client.feed(tx).await?;
                    }
                    client.flush().await?;
                    Ok::<(), std::io::Error>(())
                }
            ).await;

            match send_result {
                Ok(Ok(_)) => {
                    success_count += batch_size_actual;
                    println!("✅ 批次 [{}-{}] 成功发送 {} 笔", batch_start, batch_end - 1, batch_size_actual);
                    send_success = true;
                    break;
                }
                Ok(Err(e)) => {
                    last_error = Some(format!("{}", e));
                    if retry < max_retries {
                        eprintln!("⚠️ 批次 [{}-{}] 发送失败, 准备重试: {}", batch_start, batch_end - 1, e);
                        continue;
                    }
                    error_count += batch_size_actual;
                    eprintln!("❌ 批次 [{}-{}] 发送失败: {}", batch_start, batch_end - 1, e);
                }
                Err(_) => {
                    last_error = Some("超时".to_string());
                    if retry < max_retries {
                        eprintln!("⚠️ 批次 [{}-{}] 发送超时, 准备重试", batch_start, batch_end - 1);
                        continue;
                    }
                    error_count += batch_size_actual;
                    eprintln!("❌ 批次 [{}-{}] 发送超时", batch_start, batch_end - 1);
                }
            }

            // 回滚该批次所有 nonce
            {
                let mut pool = address_pool.lock().unwrap();
                for (sender, _) in &batch_senders {
                    if let Some((_, n)) = pool.get_mut(sender) {
                        *n -= 1;
                    }
                }
            }
            break; // 重试次数用完，放弃该批次
        }
    }

    // 结束计时
    let send_elapsed = send_start_time.elapsed();
    let send_rate = success_count as f64 / send_elapsed.as_secs_f64();

    println!(
        "\n✅ 已完成: 共发送 {} 笔 (成功: {}, 失败: {}) | 耗时 {:.3}s ({:.0} tx/s)",
        total_txs, success_count, error_count, send_elapsed.as_secs_f64(), send_rate
    );

    Ok(())
}