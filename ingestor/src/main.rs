mod serial;

use std::time::UNIX_EPOCH;

use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Serial device, e.g. /dev/ttyACM0. Defaults to the first port matching the board's VID/PID.
    #[arg(long)]
    port: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    serial::run(args.port, |received| {
        let ms = received
            .received_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        println!("{ms} {}", received.frame);
    })
    .await
}
