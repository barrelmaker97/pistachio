use std::{env, thread, time};
use std::convert::TryInto;
use std::net::SocketAddr;
use std::net::{IpAddr, Ipv4Addr};
use std::collections::HashMap;

use rups::blocking::Connection;
use rups::ConfigBuilder;

use prometheus_exporter::prometheus::register_gauge;
use prometheus_exporter::prometheus::register_gauge_vec;

use env_logger::{Builder, Env};
use log::{debug, info};

const STATUSES: &[&str] = &["OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD"];
const BEEPER_STATUSES: &[&str] = &["enabled", "disabled", "muted"];
const BIND_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
const BIND_PORT: u16 = 9120;

fn main() -> rups::Result<()> {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Read config from the environment
    let ups_name = env::var("UPS_NAME").unwrap_or_else(|_| "ups".into());
    let ups_host = env::var("UPS_HOST").unwrap_or_else(|_| "localhost".into());
    let ups_port = env::var("UPS_PORT")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(3493);
    let poll_rate = env::var("POLL_RATE")
        .ok()
        .map(|s| s.parse::<u64>().ok())
        .flatten()
        .unwrap_or(10);

    // Log config info
    info!("UPS to be checked: {ups_name}@{ups_host}:{ups_port}");
    info!("Poll Rate: Every {poll_rate} seconds");

    // Create connection to UPS
    let config = ConfigBuilder::new()
        .with_host((ups_host, ups_port).try_into().unwrap_or_default())
        .with_debug(false) // Turn this on for debugging network chatter
        .build();
    let mut conn = Connection::new(&config)?;

    // Get list of available UPS variables
    let mut gauges = HashMap::new();
    let available_vars = conn.list_vars(&ups_name).expect("Failed to connect to the UPS");
    for var in available_vars {
        if let Ok(_) = var.value().parse::<f64>() {
            let var_desc = conn.get_var_description(&ups_name, &var.name()).expect("Failed to get variable description");
            let mut var_name = var.name().replace(".", "_");
            if !var_name.starts_with("ups") {
                var_name.insert_str(0, "ups_");
            }
            let gauge = register_gauge!(var_name, var_desc).expect("Could not create gauge");
            gauges.insert(String::from(var.name()), gauge);
        }
    }

    // Create label gauges
    let status_gauge = register_gauge_vec!("ups_status", "UPS Status Code", &["status"]).expect("Cannot create gauge");
    let beeper_status_gauge = register_gauge_vec!("ups_beeper_status", "Beeper Status", &["status"]).expect("Cannot create gauge");

    // Start exporter
    let addr = SocketAddr::new(BIND_ADDR, BIND_PORT);
    prometheus_exporter::start(addr).expect("Cannot start exporter");

    // Print a list of all UPS devices
    loop {
        debug!("Polling UPS...");
        for var in conn.list_vars(&ups_name)? {
            if let Ok(value) = var.value().parse::<f64>() {
                // Update basic gauges
                match gauges.get(var.name().into()) {
                    Some(gauge) => gauge.set(value),
                    None => info!("Failed to update a gauge")
                }
            } else if var.name() == "ups.status" {
                // Update status label gauge
                for state in STATUSES {
                    let gauge = status_gauge.get_metric_with_label_values(&[state]).unwrap();
                    if var.value().contains(state) {
                        gauge.set(1.0);
                    } else {
                        gauge.set(0.0);
                    }
                }
            } else if var.name() == "ups.beeper.status" {
                // Update beeper status label gauge
                for state in BEEPER_STATUSES {
                    let gauge = beeper_status_gauge.get_metric_with_label_values(&[state]).unwrap();
                    if var.value().contains(state) {
                        gauge.set(1.0);
                    } else {
                        gauge.set(0.0);
                    }
                }
            }
        }
        thread::sleep(time::Duration::from_secs(poll_rate));
    }
}
