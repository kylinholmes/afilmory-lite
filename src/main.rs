use std::path::PathBuf;

use clap::{Parser, Subcommand};

use afilmory_lite::builder::BuildOptions;
use afilmory_lite::config::Config;
use afilmory_lite::scheduler::{self, BuildCoordinator};
use afilmory_lite::server::build_router;
use afilmory_lite::state::AppState;

#[derive(Parser)]
#[command(
    name = "afilmory-lite",
    version,
    about = "Afilmory Lite — 静态画廊 manifest 构建与服务"
)]
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
    /// 启动常驻服务：serve 预构建 SPA + 运行时注入数据 + 触发器（轮询/webhook/S3 事件/手动）
    Serve {
        /// 配置文件路径
        #[arg(long, default_value = "afilmory.toml")]
        config: PathBuf,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Build { config, force } => {
            let config = Config::load(&config)?;
            let state = AppState::new(config)?;
            let r = state.run_build(BuildOptions { force }).await?;
            println!(
                "build done: new={} processed={} skipped={} failed={} deleted={} total={}",
                r.new_count,
                r.processed_count,
                r.skipped_count,
                r.failed_count,
                r.deleted_count,
                r.total
            );
            println!("manifest: {}", state.builder.manifest_path().display());
        }
        Command::Serve { config } => {
            let config = Config::load(&config)?;
            let listen = config.server.listen.clone();
            let state = AppState::new(config)?;
            let coord = BuildCoordinator::start(state.clone());
            scheduler::spawn_poll(coord.clone(), state.config.triggers.poll_interval_secs);
            coord.trigger(false); // 启动时跑一次增量
            let app = build_router(state, coord);
            let listener = tokio::net::TcpListener::bind(&listen).await?;
            println!("afilmory-lite serving on http://{listen}  (Ctrl-C 退出)");
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}
