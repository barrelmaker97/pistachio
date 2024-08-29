use env_logger::{Builder, Env};
use log::{debug, info, warn, error};
use prometheus_exporter::prometheus::{register_gauge_vec};
use rups::blocking::Connection;
use std::collections::HashMap;
use std::{time, thread, process};

fn main() {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Declare state arrays for UPS status and beeper status
    let statuses: &[&str] = &["OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD"];
    let beeper_statuses: &[&str] = &["enabled", "disabled", "muted"];

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

    // Get list of available UPS variables and map them to a tuple of their values and descriptions
    let ups_vars = pistachio::get_available_vars(&mut conn, config.ups_name()).unwrap_or_else(|err| {
        error!("Could not get available variables from the UPS: {err}");
        process::exit(1);
    });

    // Use map of available UPS variables to create a map of associated prometheus gauges
    // Gauges must be floats, so this will only create gauges for variables that are numbers
    let basic_gauges = pistachio::create_basic_gauges(&ups_vars).unwrap_or_else(|err| {
        error!("Could not create basic gauges: {err}");
        process::exit(1)
    });

    // Create label gauges
    let mut label_gauges = HashMap::new();
    let status_gauge = register_gauge_vec!("ups_status", "UPS Status Code", &["status"])
        .expect("Cannot create status gauge");
    label_gauges.insert("ups.status", (status_gauge, statuses));
    let beeper_gauge = register_gauge_vec!("ups_beeper_status", "Beeper Status", &["status"])
        .expect("Cannot create beeper status gauge");
    label_gauges.insert("ups.beeper.status", (beeper_gauge, beeper_statuses));
    info!("{} basic gauges and {} labeled gauges will be exported", basic_gauges.len(), label_gauges.len());

    // Start prometheus exporter
    prometheus_exporter::start(*config.bind_addr()).expect("Failed to start prometheus exporter");

    // Main loop that polls for variables and updates associated gauges
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(config.ups_name()) {
            Ok(var_list) => {
                for var in var_list {
                    if let Some(gauge) = basic_gauges.get(var.name()) {
                        // Update basic gauges
                        if let Ok(value) = var.value().parse::<f64>() {
                            gauge.set(value);
                        } else {
                            warn!("Value of variable {} is not a float", var.name());
                        }
                    } else if let Some((label_gauge, states)) = label_gauges.get(var.name()) {
                        pistachio::update_label_gauge(&label_gauge, states, &var.value());
                    } else {
                        debug!("Variable {} does not have an associated gauge to update", var.name());
                    }
                }
            }
            Err(err) => {
                // Log warning and set gauges to 0 to indicate failure
                warn!("Failed to connect to the UPS");
                debug!("Err: {err}");
                for gauge in basic_gauges.values() {
                    gauge.set(0.0);
                }
                for (label_gauge, states) in label_gauges.values() {
                    for state in *states {
                        label_gauge
                            .get_metric_with_label_values(&[state])
                            .unwrap()
                            .set(0.0);
                    }
                }
                debug!("Reset gauges to zero because the UPS was unreachable");
            }
        }
        thread::sleep(time::Duration::from_secs(*config.poll_rate()));
    }
}
