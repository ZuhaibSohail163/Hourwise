//! `hourwise-backend` binary.
//!
//! ```text
//! hourwise-backend                 # run the API server (default)
//! hourwise-backend serve           # same
//! hourwise-backend migrate         # apply pending migrations and exit
//! hourwise-backend check-db        # test connectivity + credentials and exit
//! hourwise-backend verify          # read-only data-integrity report (exit 1 on problems)
//! hourwise-backend healthcheck     # exit 0 if the local API answers /api/health (Docker)
//! hourwise-backend healthcheck --ready   # same, but also requires the database (/api/health/ready)
//! hourwise-backend reset-password <email>  # new password on stdin; signs the account out everywhere
//! ```
//!
//! `reset-password` is the supported way to recover a locked-out administrator; it never runs on
//! its own (see docs/DEPLOYMENT.md → Recovering an administrator account).
//!
//! Backups use PostgreSQL's own tools (`pg_dump` / `pg_restore`); see the README.

use std::{process::ExitCode, time::Duration};

use hourwise_backend::{auth, config, db, migrate, password_reset, verify};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_subscriber::EnvFilter;

fn usage() -> ExitCode {
    eprintln!(
        "usage: hourwise-backend [serve | migrate | check-db | verify | healthcheck [--ready]]\n       \
         hourwise-backend reset-password <email>   (new password on standard input)"
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
        "migrate" | "check-db" | "verify" => db_command(command).await,
        "healthcheck" => return healthcheck(args.get(1).is_some_and(|a| a == "--ready")).await,
        "reset-password" => reset_password(&args[1..]).await,
        "-h" | "--help" | "help" => {
            // Print the usage text, but asking for help is a success (exit 0), not an error (2).
            let _ = usage();
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
    // Retry for ~1 minute: PostgreSQL may still be starting when the container boots.
    let state = hourwise_backend::init_state(config, 12)
        .await
        .map_err(|e| format!("startup failed: {}", e.detail()))?;
    let db = state.db.clone();
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
    tracing::info!("shutting down: closing database connections");
    db.close().await;
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

async fn db_command(command: &str) -> Result<ExitCode, String> {
    let settings = config::database_from_env()?;
    eprintln!("database: {}", settings.describe());
    let database = db::connect_with_retry(&settings, 3)
        .await
        .map_err(|e| format!("cannot connect: {}", e.detail()))?;
    let err = |e: hourwise_backend::error::AppError| e.detail();

    match command {
        "check-db" => {
            let applied = migrate::applied(&database).await.map_err(err)?;
            eprintln!(
                "connection OK; {} of {} migrations applied",
                applied.len(),
                migrate::known()
            );
        }
        "migrate" => {
            let n = migrate::run(&database).await.map_err(err)?;
            eprintln!("applied {n} migration(s); schema is up to date");
        }
        "verify" => {
            let report = verify::verify(&database).await.map_err(err)?;
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
        _ => return Ok(usage()),
    }
    database.close().await;
    Ok(ExitCode::SUCCESS)
}

/// `reset-password <email>`: reads the new password from stdin (one line), then updates only that
/// account in one transaction. Prints the outcome, never the password, the hash or credentials.
async fn reset_password(rest: &[String]) -> Result<ExitCode, String> {
    use std::io::{IsTerminal, Read};

    let [email] = rest else {
        eprintln!(
            "usage: hourwise-backend reset-password <email>\n\
             The new password is read from standard input, never from an argument."
        );
        return Ok(ExitCode::from(2));
    };
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err("refusing to read a password typed into this terminal (it would be shown). \
                    Pipe it in instead; see docs/DEPLOYMENT.md → Recovering an administrator account"
            .into());
    }
    let mut input = Vec::new();
    stdin
        .lock()
        .take(password_reset::MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|e| format!("cannot read standard input: {e}"))?;
    let password = password_reset::password_from_input(&input);
    input.fill(0);
    let password = password?;
    // Fail fast, before connecting; `reset_password` checks again for its other callers.
    auth::validate_password(&password).map_err(|e| e.detail())?;

    // The same DATABASE_URL as the server. No migrations: this only updates an existing row.
    let settings = config::database_from_env()?;
    eprintln!("database: {}", settings.describe());
    let database = db::connect_with_retry(&settings, 3)
        .await
        .map_err(|e| format!("cannot connect: {}", e.detail()))?;
    let result = password_reset::reset_password(&database, email, password).await;
    database.close().await;
    let outcome = result.map_err(|e| format!("{}; nothing was changed", e.detail()))?;

    eprintln!(
        "password reset for {} (role: {}); every existing session of this account is signed out",
        outcome.email, outcome.role
    );
    if !outcome.is_active {
        eprintln!("note: this account is deactivated, so it still cannot sign in");
    }
    Ok(ExitCode::SUCCESS)
}

/// Minimal HTTP probe of the local server, so the runtime image needs no curl/wget.
async fn healthcheck(ready: bool) -> ExitCode {
    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let port = bind.rsplit(':').next().unwrap_or("8080");
    let path = if ready {
        "/api/health/ready"
    } else {
        "/api/health"
    };
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port.parse().ok()?))
            .await
            .ok()?;
        let request = format!(
            "GET {path} HTTP/1.0
Host: localhost
Connection: close

"
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await.ok()?;
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        Some(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"))
    };
    match tokio::time::timeout(Duration::from_secs(4), probe).await {
        Ok(Some(true)) => ExitCode::SUCCESS,
        _ => {
            eprintln!("unhealthy: {path} on port {port} did not return 200");
            ExitCode::FAILURE
        }
    }
}
