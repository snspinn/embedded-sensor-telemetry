mod pipeline;
mod serial;
mod store;

use clap::Parser;
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

    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    tokio::spawn(pipeline::run(rx, store));

    serial::run(args.port, move |received| match tx.try_send(received) {
        Ok(()) => {}
        Err(TrySendError::Full(received)) => {
            eprintln!("Pipeline full, dropping seq {}", received.frame.seq.0)
        }
        Err(TrySendError::Closed(_)) => eprintln!("Pipeline stopped, dropping frame"),
    })
    .await
}
