use clap::Parser;
use env_logger::{Builder, Env};
use log::{debug, error, info, warn};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use std::process;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::oneshot;
use tokio::time;

async fn monitor_ups(
    mut conn: rups::tokio::Connection,
    args: pistachio::Args,
    metrics: pistachio::Metrics,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let mut is_failing = false;
    let mut interval = time::interval(Duration::from_secs(args.poll_rate));

    tokio::select! {
        _ = async {
            loop {
                interval.tick().await;
                debug!("Polling UPS...");
                match conn.list_vars(args.ups_name.as_str()).await {
                    Ok(var_list) => {
                        metrics.update(&var_list);
                        debug!("Metrics updated");
                        if is_failing {
                            info!("Connection with the UPS has been reestablished");
                            is_failing = false;
                        }
                    }
                    Err(err) => {
                        // Log warning and set gauges to 0 to indicate failure
                        warn!("Failed to connect to the UPS: {err}");
                        metrics.reset();
                        debug!("Reset gauges to zero because the UPS was unreachable");
                        is_failing = true;

                        // IO errors can cause the connection to continue failing,
                        // even once the UPS is back online. Recreating the connection
                        // resolves the issue
                        if let rups::ClientError::Io(_) = err {
                            debug!("Attempting to recreate connection due to IO error...");
                            //Creating new connection
                            match pistachio::create_connection(&args).await {
                                Ok(new_conn) => {
                                    conn = new_conn;
                                    debug!("Connection recreated successfully");
                                },
                                Err(err) => {
                                    error!("Failed to recreate connection: {err}");
                                }
                            };
                        }
                    }
                }
            }
        } => {},
        _ = shutdown_rx => {
            info!("Attempting graceful shutdown");
            conn.close().await.unwrap();
        }
    }
}

async fn handle_signals(shutdown_tx: oneshot::Sender<()>) {
    let mut sigint = signal(SignalKind::interrupt()).unwrap();
    let mut sigterm = signal(SignalKind::terminate()).unwrap();

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
    info!(
        "UPS {}@{}:{} will be checked every {} seconds",
        args.ups_name, args.ups_host, args.ups_port, args.poll_rate
    );

    // Create connection to UPS
    let mut conn = pistachio::create_connection(&args).await.unwrap_or_else(|err| {
        error!("Could not connect to the UPS: {err}");
        process::exit(1);
    });

    // Get list of available UPS vars
    let ups_vars = pistachio::get_ups_vars(&args, &mut conn).await.unwrap_or_else(|err| {
        error!("Could not get list of available variables from the UPS: {err}");
        process::exit(1);
    });

    // Start prometheus exporter
    let bind_addr = SocketAddr::new(args.bind_ip, args.bind_port);
    PrometheusBuilder::new().with_http_listener(bind_addr).install().unwrap_or_else(|err| {
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
    monitor_ups(conn, args, metrics, shutdown_rx).await;

    info!("Shutdown complete, goodbye");
}
