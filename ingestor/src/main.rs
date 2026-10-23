mod api;
mod pipeline;
mod serial;
mod store;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

const CHANNEL_CAPACITY: usize = 256;

#[derive(Parser)]
struct Args {
    /// Serial device, e.g. /dev/ttyACM0. Defaults to the first port matching the board's VID/PID.
    #[arg(long)]
    port: Option<String>,

    /// Postgres connection string.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Address for the HTTP API.
    #[arg(long, default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let store = match store::Store::connect(&args.database_url).await {
        Ok(store) => store,
        Err(e) => {
            eprintln!("Database error: {e}");
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(args.listen).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("Failed to listen on {}: {e}", args.listen);
            std::process::exit(1);
        }
    };
    eprintln!("Serving API on http://{}", args.listen);

    let serial_connected = Arc::new(AtomicBool::new(false));
    let app = api::router(api::AppState {
        store: store.clone(),
        serial_connected: serial_connected.clone(),
    });
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("API server error: {e}");
        }
    });

    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    tokio::spawn(pipeline::run(rx, store));

    serial::run(args.port, serial_connected, move |received| {
        match tx.try_send(received) {
            Ok(()) => {}
            Err(TrySendError::Full(received)) => {
                eprintln!("Pipeline full, dropping seq {}", received.frame.seq.0)
            }
            Err(TrySendError::Closed(_)) => eprintln!("Pipeline stopped, dropping frame"),
        }
    })
    .await
}
