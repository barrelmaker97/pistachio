use clap::Parser;
use env_logger::{Builder, Env};
use log::{debug, error, info};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::process;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::oneshot;

async fn handle_signals(shutdown_tx: oneshot::Sender<()>) {
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");

    tokio::select! {
        _ = sigint.recv() => {
            debug!("Received SIGINT, sending shutdown signal");
        }
        _ = sigterm.recv() => {
            debug!("Received SIGTERM, sending shutdown signal");
        }
    };

    // Send the shutdown signal
    if shutdown_tx.send(()).is_err() {
        error!("Failed to send shutdown signal: the receiver may have dropped");
    }
}

#[tokio::main]
async fn main() {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse configuration
    let args = pistachio::Args::parse();
    info!("UPS {}@{}:{} will be checked every {} seconds", args.ups_name, args.ups_host, args.ups_port, args.poll_rate);

    // Create connection to UPS
    let mut conn = args.create_connection().await.unwrap_or_else(|err| {
        error!("Could not connect to the UPS: {err}");
        process::exit(1);
    });

    // Get list of available UPS vars
    let ups_vars = args.get_ups_vars(&mut conn).await.unwrap_or_else(|err| {
        error!("Could not get list of available variables from the UPS: {err}");
        process::exit(1);
    });

    // Start prometheus exporter
    PrometheusBuilder::new().with_http_listener(args.bind_addr()).install().unwrap_or_else(|err| {
        error!("Failed to create prometheus exporter: {err}");
        process::exit(1);
    });

    // Create Prometheus metrics from available ups variables
    let metrics = pistachio::Metrics::build(&ups_vars);
    info!("{} gauges will be exported", metrics.count());

    // Create a channel for shutdown signaling
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Start watching for signals
    tokio::spawn(handle_signals(shutdown_tx));

    // Start monitoring
    pistachio::monitor_ups(conn, args, metrics, shutdown_rx).await;

    info!("Shutdown complete, goodbye");
}
