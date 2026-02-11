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

    info!("start create new proposal");

    let validator = VALIDATOR;
    info!(?validator, "Validator:");

    let proposal = create_proposal();

    assert!(proposal.proposer == validator);
    assert!(proposal.round == 102);

    info!(?proposal, "Proposal:");
}

// 构建proposal
fn create_proposal() -> Proposal<String, String> {
    // 1. 创建 QuorumCertificate
    let qc_round = 101;
    let qc_epoch = 1;
    let qc_block_hash = "0x456ef06527d887cd11fc11f1990bf8cb0d8adaa15c3ddca252258c1b622d0f2a";
    let qc_sig1 = "0x1234";
    let qc_sig2 = "0x5678";
    let signatures = vec![qc_sig1.to_string(), qc_sig2.to_string()];
    let qc = QuorumCertificate::<String>::new(
        qc_round,
        qc_epoch,
        qc_block_hash.to_string(),
        signatures,
    );

    // 2. 创建 BlockData (用于 Proposal)
    let block_height = 102;
    let block_hash = "0x3840323385af75b8ba820eb690152717c0e565a2fb30f439ce33b6080ec56e83";
    let parent_hash = "0x456ef06527d887cd11fc11f1990bf8cb0d8adaa15c3ddca252258c1b622d0f2a";
    let txs = vec![];
    let proposal_block_data = BlockData::<String>::new(
        block_height,
        block_hash.to_string(),
        parent_hash.to_string(),
        txs,
    );

    // 标准 Secp256k1
    let round = 102;
    let epoch = 1;
    let secp256k1_sig = "0xa5b9c8d3e4f7a1b2c6d8e0f4a2b1c3d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9";
    Proposal::<String, String>::new(
        VALIDATOR.to_string(),
        round,
        epoch,
        qc,
        proposal_block_data,
        secp256k1_sig.to_string(),
    )
}
