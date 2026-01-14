// Copyright (C) 2025 Category Labs, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::process;

use clap::{error::ErrorKind, ArgAction, Parser, Subcommand};
use eyre::{eyre, Result};
use monad_archive::cli::{ArchiveArgs, BlockDataReaderArgs};

#[derive(Debug)]
pub struct Cli {
    pub block_data_source: BlockDataReaderArgs,
    pub fallback_block_data_source: Option<BlockDataReaderArgs>,
    pub archive_sink: ArchiveArgs,
    pub async_backfill: bool,
    pub max_blocks_per_iteration: u64,
    pub max_concurrent_blocks: usize,
    pub reset_index: bool,
    pub stop_block: Option<u64>,
    pub otel_endpoint: Option<String>,
    pub otel_replica_name_override: Option<String>,
    pub max_inline_encoded_len: usize,
    pub skip_connectivity_check: bool,
    pub enable_logs_indexing: bool,
}

pub enum ParsedCli {
    Command { command: Commands, args: CliArgs },
    Daemon(Cli),
}

impl Cli {
    pub fn parse() -> ParsedCli {
        let mut args = match CliArgs::try_parse() {
            Ok(args) => args,
            Err(err) => match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = err.print();
                    process::exit(0);
                }
                _ => {
                    eprintln!("failed to parse monad-indexer args: {err:?}");
                    process::exit(2);
                }
            },
        };

        if let Some(command) = std::mem::take(&mut args.command) {
            return ParsedCli::Command { command, args };
        }

        match args.into_cli() {
            Ok(cli) => ParsedCli::Daemon(cli),
            Err(err) => {
                eprintln!("failed to load monad-indexer configuration: {err:?}");
                process::exit(2);
            }
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "monad-indexer", about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Source to read block data that will be indexed
    #[arg(long, value_parser = clap::value_parser!(BlockDataReaderArgs))]
    pub block_data_source: Option<BlockDataReaderArgs>,

    /// If reading from --block-data-source fails, attempts to read from
    /// this optional fallback
    #[arg(long, value_parser = clap::value_parser!(BlockDataReaderArgs))]
    pub fallback_block_data_source: Option<BlockDataReaderArgs>,

    /// Where archive data is written to
    /// For aws: 'aws <bucket_name> <concurrent_requests>'
    #[arg(long, global = true, value_parser = clap::value_parser!(ArchiveArgs))]
    pub archive_sink: Option<ArchiveArgs>,

    /// If set, indexer will perform an asynchronous backfill of the index
    /// This allows a second indexer to backfill a range while the first indexer is running
    #[arg(long, default_value_t = false)]
    pub async_backfill: bool,

    #[arg(long, default_value_t = 50)]
    pub max_blocks_per_iteration: u64,

    #[arg(long, default_value_t = 10)]
    pub max_concurrent_blocks: usize,

    /// Resets the latest indexed entry
    #[arg(long, default_value_t = false)]
    pub reset_index: bool,

    /// Override block number to stop at
    #[arg(long)]
    pub stop_block: Option<u64>,

    /// Endpoint to push metrics to
    #[arg(long)]
    pub otel_endpoint: Option<String>,

    #[arg(long)]
    pub otel_replica_name_override: Option<String>,

    /// Maximum size of an encoded inline tx index entry
    /// If an entry is larger than this, it is stored as a reference pointing to
    /// the block level data store
    #[arg(long, default_value_t = 350 * 1024)]
    pub max_inline_encoded_len: usize,

    #[arg(long, default_value_t = false)]
    pub skip_connectivity_check: bool,

    /// Enable eth_getLogs indexing (disabled by default)
    #[arg(long, default_value_t = false)]
    pub enable_logs_indexing: bool,
}

impl CliArgs {
    pub fn into_cli(self) -> Result<Cli> {
        let CliArgs {
            command: _,
            block_data_source,
            fallback_block_data_source,
            archive_sink,
            async_backfill,
            max_blocks_per_iteration,
            max_concurrent_blocks,
            reset_index,
            stop_block,
            otel_endpoint,
            otel_replica_name_override,
            max_inline_encoded_len,
            skip_connectivity_check,
            enable_logs_indexing,
        } = self;

        Ok(Cli {
            block_data_source: block_data_source
                .ok_or_else(|| eyre!("block_data_source must be provided"))?,
            fallback_block_data_source,
            archive_sink: archive_sink.ok_or_else(|| eyre!("archive_sink must be provided"))?,
            async_backfill,
            max_blocks_per_iteration,
            max_concurrent_blocks,
            reset_index,
            stop_block,
            otel_endpoint,
            otel_replica_name_override,
            max_inline_encoded_len,
            skip_connectivity_check,
            enable_logs_indexing,
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Migrate logs index
    MigrateLogs {
        /// Block number to start migration from (default: 0)
        #[arg(long, default_value_t = 0)]
        start_block: u64,
        /// Block number to stop migration at (default: latest indexed)
        #[arg(long)]
        stop_block: Option<u64>,
    },
    /// Migrate capped collection to uncapped
    MigrateCapped {
        /// Database name
        #[arg(long)]
        db_name: String,
        /// Collection name to migrate
        #[arg(long)]
        coll_name: String,
        /// Batch size for copying
        #[arg(long, default_value_t = 2000)]
        batch_size: u32,
        /// Free space factor
        #[arg(long, default_value_t = 1.5)]
        free_factor: f64,
    },
    /// Set the start block marker in the archive and exit.
    /// Use this instead of --start-block to safely configure the starting point.
    SetStartBlock {
        /// Block number to set as the latest indexed marker
        #[arg(long)]
        block: u64,

        /// Set the async-backfill marker instead of the primary marker
        #[arg(long, action = ArgAction::SetTrue)]
        async_backfill: bool,
    },
}
