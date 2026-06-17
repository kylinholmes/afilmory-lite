use std::path::PathBuf;

use clap::{Parser, Subcommand};

use afilmory_lite::builder::{BuildOptions, Builder};
use afilmory_lite::config::Config;

#[derive(Parser)]
#[command(name = "afilmory-lite", version, about = "Afilmory Lite — 静态画廊 manifest 构建与服务")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 从存储构建 manifest 与缩略图（默认增量更新）
    Build {
        /// 配置文件路径
        #[arg(long, default_value = "afilmory.toml")]
        config: PathBuf,
        /// 强制全量重建（忽略已有 manifest 与缩略图）
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build { config, force } => {
            let config = Config::load(&config)?;
            let builder = Builder::from_config(config)?;
            let r = builder.build(BuildOptions { force }).await?;
            println!(
                "build done: new={} processed={} skipped={} failed={} deleted={} total={}",
                r.new_count, r.processed_count, r.skipped_count, r.failed_count, r.deleted_count, r.total
            );
            println!("manifest: {}", builder.manifest_path().display());
        }
    }
    Ok(())
}
