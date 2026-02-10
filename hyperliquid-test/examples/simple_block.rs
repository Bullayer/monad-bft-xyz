//! 简化的 BFT 出块流程演示
//!
//! 参考 Monad 共识设计，实现一个最小化的出块过程：
//! 1. 创世区块 (Genesis Block)
//! 2. 提议区块 (Proposal)
//! 3. 投票 (Vote)
//! 4. 生成 Quorum Certificate (QC)
//! 5. 提交区块 (Commit)

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::{Duration, SystemTime};

// ============================================
// 基础类型定义 (参考 monad-types)
// ============================================

/// 区块唯一标识符 (Hash)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct BlockId([u8; 32]);

impl BlockId {
    fn new(data: &[u8]) -> Self {
        let mut hash = [0u8; 32];
        let mut i = 0;
        for byte in data.iter() {
            hash[i % 32] ^= *byte;
            i += 1;
        }
        BlockId(hash)
    }
}

/// 轮次 (Round) - 每轮可能产生一个区块
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Round(pub u64);

/// 纪元 (Epoch) - 验证者集合切换的周期
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Epoch(pub u64);

/// 区块序号 (SeqNum)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SeqNum(pub u64);

// ============================================
// 签名相关 (参考 monad-crypto)
// ============================================

/// 简化的公钥/私钥对
#[derive(Clone, Debug)]
struct KeyPair {
    private: [u8; 32],
    public: [u8; 33],
}

/// 节点 ID
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct NodeId {
    public: [u8; 33],
}

impl NodeId {
    fn new(public: [u8; 33]) -> Self {
        NodeId { public }
    }
}

/// 签名
#[derive(Clone, Copy, Debug)]
struct Signature([u8; 64]);

// ============================================
// Quorum Certificate (QC) - 参考 monad-consensus-types
// ============================================

/// 投票信息
#[derive(Clone, Debug)]
struct Vote {
    id: BlockId,
    round: Round,
    epoch: Epoch,
}

/// 投票消息
#[derive(Clone, Debug)]
struct VoteMessage {
    vote: Vote,
    signature: Signature,
    voter: NodeId,
}

/// Quorum Certificate - 表示多数验证者对某个区块的认可
#[derive(Clone, Debug)]
struct QuorumCertificate<SCT> {
    vote_info: Vote,
    signatures: Vec<SCT>,  // 收集的签名
    sig_count: usize,      // 签名数量
}

impl<SCT> QuorumCertificate<SCT> {
    fn new(vote_info: Vote, signatures: Vec<SCT>) -> Self {
        Self {
            vote_info,
            signatures,
            sig_count: signatures.len(),
        }
    }

    fn is_quorum(&self, threshold: usize) -> bool {
        self.sig_count >= threshold
    }
}

/// Genesis QC (创世区块的 QC)
fn genesis_qc() -> QuorumCertificate<()> {
    QuorumCertificate {
        vote_info: Vote {
            id: BlockId::new(b"genesis"),
            round: Round(0),
            epoch: Epoch(0),
        },
        signatures: vec![],
        sig_count: 0,
    }
}

// ============================================
// 区块定义 - 参考 monad-consensus-types/block.rs
// ============================================

/// 区块头
#[derive(Clone, Debug)]
struct BlockHeader {
    block_round: Round,      // 首次提议的轮次
    epoch: Epoch,           // 纪元
    qc: QuorumCertificate<()>, // 父区块的 QC
    author: NodeId,          // 提议者
    seq_num: SeqNum,        // 区块序号
    timestamp_ns: u128,     // 时间戳 (纳秒)
    block_body_id: BlockId, // 交易体的 ID
    base_fee: Option<u64>,  // 基础费用 (可选)
}

/// 交易体 (简化版)
#[derive(Clone, Debug)]
struct BlockBody {
    transactions: Vec<Transaction>,
}

impl BlockBody {
    fn new(transactions: Vec<Transaction>) -> Self {
        Self { transactions }
    }

    fn get_id(&self) -> BlockId {
        let data = format!("{:?}", self.transactions);
        BlockId::new(data.as_bytes())
    }
}

/// 交易
#[derive(Clone, Debug)]
struct Transaction {
    from: NodeId,
    to: NodeId,
    value: u64,
    data: Vec<u8>,
}

/// 完整区块
#[derive(Clone, Debug)]
struct Block {
    header: BlockHeader,
    body: BlockBody,
}

impl Block {
    fn new(
        author: NodeId,
        epoch: Epoch,
        round: Round,
        parent_qc: QuorumCertificate<()>,
        transactions: Vec<Transaction>,
        seq_num: SeqNum,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let body = BlockBody::new(transactions);
        let block_body_id = body.get_id();

        let header = BlockHeader {
            block_round: round,
            epoch,
            qc: parent_qc,
            author,
            seq_num,
            timestamp_ns: timestamp,
            block_body_id,
            base_fee: Some(100_000_000), // 100 Gwei
        };

        Self { header, body }
    }

    fn get_id(&self) -> BlockId {
        // 简化：基于区块头生成 ID
        let data = format!(
            "{:?}{:?}{:?}{:?}",
            self.header.block_round,
            self.header.author,
            self.header.seq_num,
            self.header.block_body_id
        );
        BlockId::new(data.as_bytes())
    }
}

// ============================================
// 提议 (Proposal) - 参考 monad-consensus/src/messages/message.rs
// ============================================

/// 提议消息
#[derive(Debug)]
struct Proposal {
    proposal_round: Round,
    proposal_epoch: Epoch,
    block: Block,
    last_round_tc: Option<()>, // Timeout Certificate (可选)
}

impl Proposal {
    fn new(
        round: Round,
        epoch: Epoch,
        block: Block,
        last_round_tc: Option<()>,
    ) -> Self {
        Self {
            proposal_round: round,
            proposal_epoch: epoch,
            block,
            last_round_tc,
        }
    }
}

// ============================================
// 验证者集合
// ============================================

struct ValidatorSet {
    validators: Vec<NodeId>,
    threshold: usize, //  quorum 阈值 (如: 2/3 + 1)
}

impl ValidatorSet {
    fn new(validators: Vec<NodeId>) -> Self {
        // Quorum 阈值: 2/3 + 1
        let threshold = (validators.len() * 2 / 3) + 1;
        Self { validators, threshold }
    }

    fn get_validator(&self, index: usize) -> Option<&NodeId> {
        self.validators.get(index)
    }

    fn size(&self) -> usize {
        self.validators.len()
    }
}

// ============================================
// 模拟 BFT 共识流程
// ============================================

/// BFT 节点状态
struct BftNode {
    node_id: NodeId,
    keypair: KeyPair,
    validators: ValidatorSet,
    current_round: Round,
    current_epoch: Epoch,
    /// 链: BlockId -> Block
    chain: HashMap<BlockId, Block>,
    /// 最新的 QC
    high_qc: QuorumCertificate<()>,
    /// 待处理的提案
    pending_proposals: HashMap<Round, Proposal>,
    /// 已投票的区块
    voted_blocks: HashMap<BlockId, Vote>,
}

impl BftNode {
    fn new(node_id: NodeId, keypair: KeyPair, validators: ValidatorSet) -> Self {
        Self {
            node_id,
            keypair,
            validators,
            current_round: Round(0),
            current_epoch: Epoch(0),
            chain: HashMap::new(),
            high_qc: genesis_qc(),
            pending_proposals: HashMap::new(),
            voted_blocks: HashMap::new(),
        }
    }

    /// 处理收到的提案
    fn receive_proposal(&mut self, proposal: Proposal) {
        println!(
            "[Node {:?}] 收到提案 - Round: {:?}, Block: {:?}",
            self.node_id.public[0..4].to_vec(),
            proposal.proposal_round,
            proposal.block.get_id().0[0..4].to_vec()
        );

        // 验证提案
        if !self.validate_proposal(&proposal) {
            println!("[Node {:?}] 提案验证失败!", self.node_id.public[0..4].to_vec());
            return;
        }

        // 存储提案
        self.pending_proposals
            .insert(proposal.proposal_round, proposal.clone());

        // 发起投票
        self.vote(&proposal);
    }

    /// 验证提案
    fn validate_proposal(&self, proposal: &Proposal) -> bool {
        // 1. 检查轮次
        if proposal.proposal_round < self.current_round {
            return false;
        }

        // 2. 检查父区块
        let parent_id = proposal.block.header.qc.vote_info.id;
        if !self.chain.contains_key(&parent_id) {
            println!(
                "[Node {:?}] 未知父区块: {:?}",
                self.node_id.public[0..4].to_vec(),
                parent_id.0[0..4].to_vec()
            );
            // 允许 (可能是新节点)
        }

        // 3. 检查序号连续性
        let expected_seq = self.high_qc.vote_info.id.0[0] as u64 + 1;
        if proposal.block.header.seq_num.0 != expected_seq {
            println!(
                "[Node {:?}] 区块序号不连续: expected {}, got {}",
                self.node_id.public[0..4].to_vec(),
                expected_seq,
                proposal.block.header.seq_num.0
            );
        }

        true
    }

    /// 投票
    fn vote(&mut self, proposal: &Proposal) {
        let vote = Vote {
            id: proposal.block.get_id(),
            round: proposal.proposal_round,
            epoch: proposal.proposal_epoch,
        };

        // 签名
        let signature = self.sign_vote(&vote);

        println!(
            "[Node {:?}] 投票给区块 {:?} (Round: {:?})",
            self.node_id.public[0..4].to_vec(),
            vote.id.0[0..4].to_vec(),
            vote.round.0
        );

        // 存储投票
        self.voted_blocks.insert(vote.id, vote);
    }

    /// 对投票签名 (简化)
    fn sign_vote(&self, vote: &Vote) -> Signature {
        let mut sig = [0u8; 64];
        for (i, byte) in vote.id.0.iter().enumerate() {
            sig[i % 64] ^= *byte ^ self.keypair.private[i % 32];
        }
        Signature(sig)
    }

    /// 收集投票并生成 QC
    fn collect_votes(&self, block_id: BlockId, round: Round) -> QuorumCertificate<()> {
        // 简化：假设收集到足够的签名
        let sig_count = self.validators.threshold;

        let vote_info = Vote {
            id: block_id,
            round,
            epoch: self.current_epoch,
        };

        println!(
            "[Node {:?}] 生成 QC - Block: {:?}, Signatures: {} / {}",
            self.node_id.public[0..4].to_vec(),
            block_id.0[0..4].to_vec(),
            sig_count,
            self.validators.size()
        );

        QuorumCertificate::new(vote_info, vec![(); sig_count])
    }

    /// 提交区块 (Commit)
    fn commit_block(&mut self, block_id: BlockId, qc: QuorumCertificate<()>) {
        if let Some(proposal) = self.pending_proposals.get(&qc.vote_info.round) {
            // 将区块添加到链
            self.chain.insert(block_id, proposal.block.clone());

            // 更新 high QC
            self.high_qc = qc;

            // 更新轮次
            self.current_round = Round(qc.vote_info.round.0 + 1);

            println!(
                "[Node {:?}] 提交区块 {:?} (SeqNum: {})",
                self.node_id.public[0..4].to_vec(),
                block_id.0[0..4].to_vec(),
                self.chain.get(&block_id).unwrap().header.seq_num.0
            );
        }
    }
}

// ============================================
// 主函数 - 模拟出块流程
// ============================================

fn main() {
    println!("=== 简化 BFT 出块流程演示 ===\n");

    // 1. 初始化验证者集合 (4 个节点)
    println!("1. 初始化验证者集合 (4 个节点)\n");

    let validator_keys: Vec<KeyPair> = (0..4)
        .map(|i| KeyPair {
            private: [i as u8; 32],
            public: [0; 33],
        })
        .collect();

    let validators: Vec<NodeId> = validator_keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let mut public = k.public;
            public[0] = i as u8;
            NodeId::new(public)
        })
        .collect();

    let validator_set = ValidatorSet::new(validators.clone());

    println!("   验证者数量: {}", validator_set.size());
    println!("   Quorum 阈值: {}\n", validator_set.threshold);

    // 2. 初始化 BFT 节点
    println!("2. 初始化 BFT 节点\n");

    let mut nodes: Vec<BftNode> = validator_keys
        .into_iter()
        .enumerate()
        .map(|(i, key)| {
            let mut public = key.public;
            public[0] = i as u8;
            BftNode::new(NodeId::new(public), key, validator_set.clone())
        })
        .collect();

    // 3. 创建创世区块
    println!("3. 创建创世区块\n");

    let genesis_block = Block::new(
        validators[0],      // 第一个验证者是创世区块作者
        Epoch(0),
        Round(0),
        genesis_qc(),
        vec![],            // 创世区块没有交易
        SeqNum(0),
    );

    let genesis_id = genesis_block.get_id();
    println!("   Genesis Block ID: {:?}", hex::encode(&genesis_id.0[..8]));
    println!("   Genesis Author: {:?}", hex::encode(&genesis_block.header.author.public[..8]));

    // 将创世区块添加到所有节点的链中
    for node in &mut nodes {
        node.chain.insert(genesis_id, genesis_block.clone());
    }

    // 4. 提议新区块 (Round 1)
    println!("\n4. Round 1 - 提议新区块\n");

    let round = Round(1);
    let seq_num = SeqNum(1);

    // 创建一些模拟交易
    let transactions: Vec<Transaction> = (0..3)
        .map(|i| Transaction {
            from: validators[i % validators.len()],
            to: validators[(i + 1) % validators.len()],
            value: 100 * (i + 1) as u64,
            data: vec![0x01, 0x02, 0x03],
        })
        .collect();

    // 验证者 0 提议区块
    let proposer = validators[0];
    let proposal_block = Block::new(
        proposer,
        Epoch(0),
        round,
        genesis_qc(),
        transactions.clone(),
        seq_num,
    );

    let proposal = Proposal::new(round, Epoch(0), proposal_block, None);

    println!("   提议者: {:?}", hex::encode(&proposer.public[..8]));
    println!("   区块 ID: {:?}", hex::encode(&proposal.block.get_id().0[..8]));
    println!("   交易数量: {}", transactions.len());

    // 5. 广播提案并收集投票
    println!("\n5. 广播提案并收集投票\n");

    // 所有节点收到提案
    for node in &mut nodes {
        node.receive_proposal(proposal.clone());
    }

    // 模拟：节点 0 作为 leader 收集投票并生成 QC
    println!("   收集投票...");
    std::thread::sleep(Duration::from_millis(100));

    let qc = nodes[0].collect_votes(proposal.block.get_id(), round);

    // 6. 提交区块
    println!("\n6. 提交区块\n");

    for node in &mut nodes {
        node.commit_block(proposal.block.get_id(), qc.clone());
    }

    // 7. Round 2 - 继续出块
    println!("\n7. Round 2 - 继续出块\n");

    let round = Round(2);
    let seq_num = SeqNum(2);

    let transactions: Vec<Transaction> = (0..2)
        .map(|i| Transaction {
            from: validators[(i + 2) % validators.len()],
            to: validators[(i + 3) % validators.len()],
            value: 50 * (i + 1) as u64,
            data: vec![0x04, 0x05],
        })
        .collect();

    // 验证者 1 提议区块
    let proposer = validators[1];
    let proposal_block = Block::new(
        proposer,
        Epoch(0),
        round,
        qc, // 使用上一轮的 QC
        transactions.clone(),
        seq_num,
    );

    let proposal = Proposal::new(round, Epoch(0), proposal_block, None);

    println!("   提议者: {:?}", hex::encode(&proposer.public[..8]));
    println!("   区块 ID: {:?}", hex::encode(&proposal.block.get_id().0[..8]));

    // 所有节点处理提案
    for node in &mut nodes {
        node.receive_proposal(proposal.clone());
    }

    // 生成 QC 并提交
    let qc = nodes[1].collect_votes(proposal.block.get_id(), round);

    for node in &mut nodes {
        node.commit_block(proposal.block.get_id(), qc.clone());
    }

    // 8. 最终状态
    println!("\n8. 最终链状态\n");

    for (i, node) in nodes.iter().enumerate() {
        println!("   Node {}: 高度 = {}, 最新 QC Round = {}",
            i,
            node.chain.len(),
            node.high_qc.vote_info.round.0);
    }

    println!("\n=== 出块流程完成 ===");
}
