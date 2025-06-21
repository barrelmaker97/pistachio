use clap::Parser;
use env_logger::{Builder, Env};
use log::{error, info, debug, warn};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use std::process;
use std::thread;
use std::time::Duration;

fn main() {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse configuration
    let args = pistachio::Args::parse();
    info!(
        "UPS {}@{}:{} will be checked every {} seconds",
        args.ups_name, args.ups_host, args.ups_port, args.poll_rate
    );

    // Create connection to UPS
    let mut conn = pistachio::create_connection(&args).unwrap_or_else(|err| {
        error!("Could not connect to the UPS: {err}");
        process::exit(1);
    });

    // Get list of available UPS vars
    let ups_vars = pistachio::get_ups_vars(&args, &mut conn).unwrap_or_else(|err| {
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

    // Run pistachio
    let mut is_failing = false;
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(args.ups_name.as_str()) {
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

                match err {
                    rups::ClientError::Nut(nut_error) => {
                        debug!("NUT error");
                    }

                    rups::ClientError::Io(io_error) => {
                        debug!("I/O error. Tearing down and recreating connection.");
                        conn = pistachio::create_connection(&args).unwrap_or_else(|err| {
                            error!("Failed to recreate connection: {err}");
                            process::exit(1);
                        });
                    }
                }
            }
        }
        thread::sleep(Duration::from_secs(args.poll_rate));
    }
}
