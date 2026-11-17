use anyhow::Result;
use clap::Parser;
use rustysoul::{
    api::{ApiState, router},
    config::{Config, Secrets},
    engine,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "RustySoul — Linux Soulseek daemon")]
struct Args {
    #[arg(short, long, default_value = "rustysoul.toml")]
    config: PathBuf,
    /// Validate configuration and environment without opening sockets.
    #[arg(long)]
    check: bool,
    /// Exit successfully if the local daemon's authenticated status endpoint responds.
    #[arg(long)]
    healthcheck: bool,
    /// Save an encrypted application backup while the daemon is stopped.
    #[arg(long, conflicts_with_all=["inspect_backup","restore_backup"])]
    create_backup: Option<PathBuf>,
    /// Validate an encrypted backup without changing application state.
    #[arg(long, conflicts_with_all=["create_backup","restore_backup"])]
    inspect_backup: Option<PathBuf>,
    /// Restore application state while the daemon is stopped. Media is not changed.
    #[arg(long, conflicts_with_all=["create_backup","inspect_backup"])]
    restore_backup: Option<PathBuf>,
    /// Read the backup passphrase from this file (one optional trailing newline).
    #[arg(long, conflicts_with = "backup_passphrase_stdin")]
    backup_passphrase_file: Option<PathBuf>,
    /// Read the backup passphrase from standard input, never a command-line value.
    #[arg(long)]
    backup_passphrase_stdin: bool,
    #[arg(long)]
    confirm_restore: bool,
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<std::process::ExitCode> {
    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustysoul=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(rustysoul::activity::TraceLayer)
        .init();
    let args = Args::parse();
    let config = Config::load(&args.config)?;
    if args.create_backup.is_some()
        || args.inspect_backup.is_some()
        || args.restore_backup.is_some()
    {
        use std::io::Read;
        let mut pass = String::new();
        if let Some(path) = &args.backup_passphrase_file {
            std::fs::File::open(path)?
                .take(1026)
                .read_to_string(&mut pass)?;
        } else if args.backup_passphrase_stdin {
            std::io::stdin().take(1026).read_to_string(&mut pass)?;
        } else {
            anyhow::bail!("Use --backup-passphrase-file or --backup-passphrase-stdin");
        }
        if pass.ends_with('\n') {
            pass.pop();
            if pass.ends_with('\r') {
                pass.pop();
            }
        }
        rustysoul::backup::passphrase(&pass)?;
        if let Some(path) = &args.create_backup {
            rustysoul::backup::export_file(&config, &pass, path)?;
            println!("Encrypted backup created. Keep the file and passphrase separately.");
        }
        if let Some(path) = &args.inspect_backup {
            let summary = rustysoul::backup::inspect_file(&config, path, &pass)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        if let Some(path) = &args.restore_backup {
            anyhow::ensure!(
                args.confirm_restore,
                "Add --confirm-restore after validating the backup. Current application state will be replaced."
            );
            let id = rustysoul::backup::restore_file(path, &pass, &config)?;
            println!(
                "Application state restored; rollback {id} retained. Start RustySoul and sign in with the backup account. Soulseek starts disconnected."
            );
        }
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let secrets = Secrets::from_env()?;
    if args.healthcheck {
        healthcheck(&config, &secrets.api_token)?;
        return Ok(std::process::ExitCode::SUCCESS);
    }
    if args.check {
        println!("Configuration and environment are valid");
        return Ok(std::process::ExitCode::SUCCESS);
    }
    rustysoul::maintenance::apply_pending(&config)?;
    rustysoul::backup::apply_pending(&config)?;
    let bind = config.bind;
    let token = secrets.api_token.clone();
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let handle = engine::start(config.clone(), secrets)?;
    if let Err(error) = rustysoul::backup::cleanup_temporary(&config.data_dir) {
        tracing::warn!("Could not clean leftover backup workspace: {error}");
    }
    let _activity = rustysoul::activity::start(&config.data_dir.join("rustysoul.sqlite3"))?;
    soulseek_rs::utils::logger::set_observer(std::sync::Arc::new(|level, message| {
        if !matches!(
            level,
            soulseek_rs::utils::logger::LogLevel::Error
                | soulseek_rs::utils::logger::LogLevel::Warn
        ) {
            return;
        }
        rustysoul::activity::record(
            match level {
                soulseek_rs::utils::logger::LogLevel::Error => "error",
                soulseek_rs::utils::logger::LogLevel::Warn => "warn",
                soulseek_rs::utils::logger::LogLevel::Info => "info",
                soulseek_rs::utils::logger::LogLevel::Debug => "debug",
                soulseek_rs::utils::logger::LogLevel::Trace => "trace",
            },
            "protocol",
            message,
        )
    }));
    let auth = rustysoul::auth::Auth::open(&config.data_dir.join("rustysoul.sqlite3"))?;
    let restart = std::sync::Arc::new(tokio::sync::Notify::new());
    let restarting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let app = router(
        ApiState::new(
            &config,
            &token,
            handle.commands.clone(),
            handle.snapshot.clone(),
        )
        .with_auth(auth)
        .with_restart(restart.clone()),
    );
    tracing::info!(%bind, "RustySoul is listening");
    let stop = handle.stop.clone();
    use std::future::IntoFuture;
    let requested = restarting.clone();
    let (draining, mut drain) = tokio::sync::watch::channel(false);
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        tokio::select! { _=shutdown()=>{}, _=restart.notified()=>{requested.store(true,std::sync::atomic::Ordering::Release);} }
        let _=stop.send(());let _=draining.send(true);
    }).into_future();
    tokio::pin!(server);
    let result = tokio::select! {
        result=&mut server=>result,
        _=drain.changed()=>match tokio::time::timeout(std::time::Duration::from_secs(10),&mut server).await {
            Ok(result)=>result,Err(_)=>{tracing::warn!("HTTP shutdown drain deadline reached");Ok(())}
        }
    };
    let _ = handle.stop.send(());
    // Joining the protocol worker must never occupy an HTTP runtime worker.
    tokio::task::spawn_blocking(move || handle.thread.join())
        .await?
        .map_err(|_| anyhow::anyhow!("Protocol worker panicked"))?;
    result?;
    Ok(if restarting.load(std::sync::atomic::Ordering::Acquire) {
        std::process::ExitCode::from(75)
    } else {
        std::process::ExitCode::SUCCESS
    })
}

fn healthcheck(config: &Config, _token: &str) -> Result<()> {
    use std::io::{BufRead, Read, Write};
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
        time::Duration,
    };
    let ip = match config.bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let address = SocketAddr::new(ip, config.bind.port());
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )?;
    let mut line = String::new();
    std::io::BufReader::new(stream)
        .take(128)
        .read_line(&mut line)?;
    anyhow::ensure!(
        line.starts_with("HTTP/1.1 200 "),
        "Daemon health check failed"
    );
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
