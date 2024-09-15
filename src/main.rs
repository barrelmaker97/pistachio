use clap::Parser;
use env_logger::{Builder, Env};
use log::{debug, error, info, warn};
use rups::blocking::Connection;
use std::{process, thread, time};

fn main() {
    let args = pistachio::Args::parse();

    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Read config from the environment
    let config = pistachio::Config::build().unwrap_or_else(|err| {
        error!("Could not load configuration: {err}");
        process::exit(1);
    });
    info!("UPS to be checked: {}", config.ups_fullname());
    info!("Poll Rate: Every {} seconds", config.poll_rate());

    // Create connection to UPS
    let mut conn = Connection::new(config.rups_config()).unwrap_or_else(|err| {
        error!("Failed to connect to the UPS: {err}");
        process::exit(1);
    });

    // Get list of available UPS vars
    let ups_vars = pistachio::get_ups_vars(&mut conn, config.ups_name()).unwrap_or_else(|err| {
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
    prometheus_exporter::start(*config.bind_addr()).unwrap_or_else(|err| {
        error!("Failed to start prometheus exporter: {err}");
        process::exit(1);
    });

    // Main loop that polls the NUT server and updates associated gauges
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(config.ups_name()) {
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
        thread::sleep(time::Duration::from_secs(*config.poll_rate()));
    }
}
