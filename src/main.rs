use clap::Parser;
use env_logger::{Builder, Env};
use log::{error, info};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use std::process;

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

    // Create Prometheus metrics from available ups variables
    let metrics = pistachio::Metrics::build(&ups_vars);
    info!("{} gauges will be exported", metrics.count());

    // Start prometheus exporter
    let bind_addr = SocketAddr::new(args.bind_ip, args.bind_port);
    PrometheusBuilder::new().with_http_listener(bind_addr).install().unwrap_or_else(|err| {
        error!("Failed to start prometheus exporter: {err}");
        process::exit(1);
    });

    // Run pistachio
    pistachio::run(&args, &mut conn, &metrics);
}
