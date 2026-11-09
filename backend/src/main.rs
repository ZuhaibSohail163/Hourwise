//! `hourwise-backend` binary.
//!
//! ```text
//! hourwise-backend                 # run the API server (default)
//! hourwise-backend serve           # same
//! hourwise-backend migrate         # apply pending migrations and exit
//! hourwise-backend check-db        # test connectivity + credentials and exit
//! hourwise-backend verify          # data-integrity report (exit code 1 on problems)
//! hourwise-backend backup <file>   # write a logical SQL backup (use `-` for stdout)
//! hourwise-backend restore <file>  # restore a backup into an EMPTY database
//! ```

use std::process::ExitCode;

use hourwise_backend::{backup, config, db, migrate};
use tracing_subscriber::EnvFilter;

fn usage() -> ExitCode {
    eprintln!(
        "usage: hourwise-backend [serve | migrate | check-db | verify | backup <file|-> | restore <file>]"
    );
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("hourwise_backend=info,tower_http=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");
    let result = match command {
        "serve" => serve().await,
        "migrate" | "check-db" | "verify" | "backup" | "restore" => {
            db_command(command, args.get(1).map(String::as_str)).await
        }
        "-h" | "--help" | "help" => {
            usage();
            return ExitCode::SUCCESS;
        }
        _ => return usage(),
    };
    match result {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<ExitCode, String> {
    let config = config::Config::from_env();
    let bind_addr = config.bind_addr.clone();
    // Retry for ~1 minute: Turso Cloud or the network may be briefly unavailable at boot.
    let state = hourwise_backend::init_state(config, 12)
        .await
        .map_err(|e| format!("startup failed: {}", e.detail()))?;
    let app = hourwise_backend::app(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("failed to bind {bind_addr}: {e}"))?;
    tracing::info!("Hourwise API listening on {bind_addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| format!("server error: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn db_command(command: &str, arg: Option<&str>) -> Result<ExitCode, String> {
    let target = config::database_from_env()?;
    eprintln!("database: {}", target.describe());
    let database = db::open_with_retry(&target, 3)
        .await
        .map_err(|e| format!("cannot connect: {}", e.detail()))?;
    let err = |e: hourwise_backend::error::AppError| e.detail();

    match command {
        "check-db" => {
            let conn = database.conn().await.map_err(err)?;
            let applied = migrate::applied(&conn).await.map_err(err)?;
            eprintln!(
                "connection OK; {} of {} migrations applied",
                applied.len(),
                migrate::MIGRATIONS.len()
            );
        }
        "migrate" => {
            let n = migrate::run(&database).await.map_err(err)?;
            eprintln!("applied {n} migration(s); schema is up to date");
        }
        "verify" => {
            let report = backup::verify(&database).await.map_err(err)?;
            for (table, n) in &report.counts {
                eprintln!("  {table:<16} {n}");
            }
            for note in &report.notes {
                eprintln!("  note: {note}");
            }
            if report.ok() {
                eprintln!("OK: no integrity problems found");
            } else {
                for p in &report.problems {
                    eprintln!("  PROBLEM: {p}");
                }
                return Ok(ExitCode::FAILURE);
            }
        }
        "backup" => {
            let path = arg.ok_or("backup needs a file path (or `-` for stdout)")?;
            let text = backup::export(&database).await.map_err(err)?;
            if path == "-" {
                use std::io::Write;
                std::io::stdout()
                    .write_all(text.as_bytes())
                    .map_err(|e| e.to_string())?;
            } else {
                std::fs::write(path, &text).map_err(|e| format!("cannot write {path}: {e}"))?;
                eprintln!("backup written to {path} ({} bytes)", text.len());
            }
        }
        "restore" => {
            let path = arg.ok_or("restore needs a backup file path")?;
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let counts = backup::import(&database, &text).await.map_err(err)?;
            eprintln!("restore complete:");
            for (table, n) in &counts {
                eprintln!("  {table:<16} {n}");
            }
        }
        _ => return Ok(usage()),
    }
    Ok(ExitCode::SUCCESS)
}
