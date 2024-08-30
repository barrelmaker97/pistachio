use env_logger::{Builder, Env};
use log::{debug, info, warn, error};
use rups::blocking::Connection;
use std::{time, thread, process};

fn main() {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Read config from the environment
    let config = pistachio::Config::build().unwrap_or_else(|err| {
        error!("Could not load configuration: {err}");
        process::exit(1);
    });

    // Log config info
    info!("UPS to be checked: {}", config.ups_fullname());
    info!("Poll Rate: Every {} seconds", config.poll_rate());

    // Create connection to UPS
    let mut conn = Connection::new(config.rups_config()).expect("Failed to connect to the UPS");

    // Get list of available UPS vars
    let ups_vars = pistachio::get_available_vars(&mut conn, config.ups_name()).unwrap_or_else(|err| {
        error!("Could not get list of available variables from the UPS: {err}");
        process::exit(1);
    });

    let metrics = pistachio::Metrics::build(ups_vars).unwrap_or_else(|err| {
        error!("Could not create prometheus gauges from UPS variables: {err}");
        process::exit(1);
    });

    info!("{} gauges will be exported", metrics.count());

    // Start prometheus exporter
    prometheus_exporter::start(*config.bind_addr()).expect("Failed to start prometheus exporter");

    // Main loop that polls for variables and updates associated gauges
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
