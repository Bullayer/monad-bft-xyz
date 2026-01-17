use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    marker::PhantomData,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    process,
    sync::Arc,
    time::{Duration, Instant},
};

use clap::CommandFactory;
use tokio::signal::unix::{signal, SignalKind};
use self::{cli::Cli, error::NodeSetupError, state::NodeState};

mod cli;
mod error;
mod state;

const MONAD_NODE_VERSION: Option<&str> = option_env!("MONAD_VERSION");

fn main() {
    let mut cmd = Cli::command();

    let node_state = NodeState::setup(&mut cmd).unwrap_or_else(|e| cmd.error(e.kind(), e).exit());

    // thread.txt
    rayon::ThreadPoolBuilder::new()           // 创建一个新的线程池构建器
        .num_threads(8)                        // 设置线程池中有 8 个工作线程
        .thread_name(|i| format!("monad-bft-rn-{}", i))  // 为每个线程命名，格式为 "monad-bft-rn-{index}"
        .build_global()                        // 将此线程池设置为全局默认线程池
        .map_err(Into::into)                   // 将错误转换为 NodeSetupError 类型
        .unwrap_or_else(|e: NodeSetupError|    // 如果设置失败，使用 clap 的错误处理退出程序
            cmd.error(e.kind(), e).exit()
        );

    // thread.txt
    let runtime = tokio::runtime::Builder::new_multi_thread()  // 创建多线程运行时构建器
        .enable_all()                                           // 启用所有 I/O 驱动和时间驱动
        .build()                                               // 构建运行时实例
        .map_err(Into::into)                                   // 将构建错误转换为 NodeSetupError
        .unwrap_or_else(|e: NodeSetupError|                    // 如果构建失败，使用 clap 错误处理退出
            cmd.error(e.kind(), e).exit()
        );

    drop(cmd);
}