use clap::Parser;
use env_logger::{Builder, Env};
use log::{debug, error, info, warn};
use rups::blocking::Connection;
use std::net::SocketAddr;
use std::time::Duration;
use std::{process, thread};

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
    let rups_config = rups::ConfigBuilder::new()
        .with_host((args.ups_host.clone(), args.ups_port).try_into().unwrap_or_default())
        .with_timeout(Duration::from_secs(args.poll_rate - 1))
        .build();
    let mut conn = Connection::new(&rups_config).unwrap_or_else(|err| {
        error!("Failed to connect to the UPS: {err}");
        process::exit(1);
    });

    // Get list of available UPS vars
    let ups_vars = pistachio::get_ups_vars(&mut conn, args.ups_name.as_str()).unwrap_or_else(|err| {
        error!("Could not get list of available variables from the UPS: {err}");
        process::exit(1);
    });

    // Create Prometheus metrics from available ups variables
    let metrics = pistachio::Metrics::build(&ups_vars).unwrap_or_else(|err| {
        error!("Could not create prometheus gauges from UPS variables: {err}");
        process::exit(1);
    });
    info!("{} gauges will be exported", metrics.count());

    // Start prometheus exporter
    let bind_addr = SocketAddr::new(args.bind_ip, args.bind_port);
    prometheus_exporter::start(bind_addr).unwrap_or_else(|err| {
        error!("Failed to start prometheus exporter: {err}");
        process::exit(1);
    });

    // Main loop that polls the NUT server and updates associated gauges
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(args.ups_name.as_str()) {
            Ok(var_list) => {
                metrics.update(&var_list);
                debug!("Metrics updated");
            }
            Err(err) => {
                // Log warning and set gauges to 0 to indicate failure
                warn!("Failed to connect to the UPS: {err}");
                metrics.reset().unwrap_or_else(|err| {
                    warn!("Failed to reset gauges to zero: {err}")
                });
                debug!("Reset gauges to zero because the UPS was unreachable");
            }
        }
        thread::sleep(Duration::from_secs(args.poll_rate));
    }
}
