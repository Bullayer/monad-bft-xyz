//! 简化的 Proposal 提案演示

use std::fmt::Debug;
use tracing::info;

const VALIDATOR: &str = "0x13edc33d36086422d547693c21b365259e4823bc";

#[derive(Debug)]
#[allow(dead_code)]
struct BlockData<T> {
    block_height: u64,
    block_hash: String,
    parent_hash: String,
    txs: Vec<T>,
}

impl<T> BlockData<T> {
    fn new(block_height: u64, block_hash: String, parent_hash: String, txs: Vec<T>) -> Self {
        Self {
            block_height,
            block_hash,
            parent_hash,
            txs,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct QuorumCertificate<ST> {
    round: u64,
    epoch: u64,
    block_hash: String,
    signatures: Vec<ST>,
}

impl<ST> QuorumCertificate<ST> {
    fn new(round: u64, epoch: u64, block_hash: String, signatures: Vec<ST>) -> Self {
        Self {
            round,
            epoch,
            block_hash,
            signatures,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct Proposal<ST, T> {
    proposer: String,
    round: u64,
    epoch: u64,
    qc: QuorumCertificate<ST>,
    block_data: BlockData<T>,
    signature: String,
}

impl<ST, T> Proposal<ST, T> {
    fn new(
        proposer: String,
        round: u64,
        epoch: u64,
        qc: QuorumCertificate<ST>,
        block_data: BlockData<T>,
        signature: String,
    ) -> Self {
        Self {
            proposer,
            round,
            epoch,
            qc,
            block_data,
            signature,
        }
    }
}

// 主函数 - 验证者提案
fn main() {
    tracing_subscriber::fmt::init();

    info!("start create a new proposal");

    // 验证者
    let validator = VALIDATOR;
    info!(?validator, "Validator:");

    // 提案 Round 1
    let proposal = create_proposal();

    assert!(proposal.proposer == validator);
    assert!(proposal.round == 102);

    info!(?proposal, "Proposal:");
}

// 构建proposal
fn create_proposal() -> Proposal<String, String> {
    // 1. 创建 QuorumCertificate
    let qc = QuorumCertificate::<String>::new(
        101,
        1,
        "0x456ef06527d887cd11fc11f1990bf8cb0d8adaa15c3ddca252258c1b622d0f2a".to_string(),
        vec!["0x0102030405".to_string()],
    );

    // 2. 创建 BlockData (用于 Proposal)
    let proposal_block_data = BlockData::<String>::new(
        102,
        "0x3840323385af75b8ba820eb690152717c0e565a2fb30f439ce33b6080ec56e83".to_string(),
        "0x456ef06527d887cd11fc11f1990bf8cb0d8adaa15c3ddca252258c1b622d0f2a".to_string(),
        vec![],
    );

    // 标准 Secp256k1 签名: 65 字节 (r: 32字节, s: 32字节, v: 1字节)
    let secp256k1_sig = "0xa5b9c8d3e4f7a1b2c6d8e0f4a2b1c3d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9";
    Proposal::<String, String>::new(
        VALIDATOR.to_string(),
        102,
        1,
        qc,
        proposal_block_data,
        secp256k1_sig.to_string(),
    )
}
