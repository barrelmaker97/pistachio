use std::{env, thread, time};
use std::convert::TryInto;
use std::net::SocketAddr;
use std::collections::HashMap;

use rups::blocking::Connection;
use rups::{ConfigBuilder};

use prometheus_exporter::prometheus::register_gauge;

use env_logger::{Builder, Env};
use log::info;

fn main() -> rups::Result<()> {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Read config from the environment
    let addr_raw = "0.0.0.0:9120";
    let addr: SocketAddr = addr_raw.parse().expect("Cannot parse listen address");
    let ups_name = env::var("UPS_NAME").unwrap_or_else(|_| "ups".into());
    let host = env::var("UPS_HOST").unwrap_or_else(|_| "localhost".into());
    let port = env::var("UPS_PORT")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(3493);
    let poll_rate = env::var("POLL_RATE")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(10);

    // Log config info
    info!("UPS to be checked: {ups_name}@{host}:{port}");
    info!("Poll Rate: Every {poll_rate} seconds");

    // Create connection to UPS
    let config = ConfigBuilder::new()
        .with_host((host, port).try_into().unwrap_or_default())
        .with_debug(false) // Turn this on for debugging network chatter
        .build();
    let mut conn = Connection::new(&config)?;

    // Get list of available UPS variables
    let mut metrics = HashMap::new();
    let available_vars = conn.list_vars(&ups_name).expect("Failed to connect to the UPS");
    for var in available_vars {
        let var_desc = conn.get_var_description(&ups_name, &var.name()).expect("Failed to get variable description");
        let mut var_name = var.name().replace(".", "_");
        if !var_name.starts_with("ups") {
            var_name.insert_str(0, "ups_");
        }
        let gauge = register_gauge!(&var_name, &var_desc).expect("Could not create gauge");
        metrics.insert(var_name, gauge);
    }
    info!("{} metrics available to be exported", metrics.len());

    // Create test metric and start exporter
    let test_metric = register_gauge!("test_gauge", "test description").expect("Cannot create gauge");
    prometheus_exporter::start(addr).expect("Cannot start exporter");

    // TEST METRIC
    test_metric.set(42.0);

    // Print a list of all UPS devices
    let mut counter = 0;
    loop {
        if counter % poll_rate == 0 {
            // List UPS variables (key = val)
            for var in conn.list_vars(&ups_name)? {
                println!("\t\t- {}", var);
            }
        }
        counter += 1;
        thread::sleep(time::Duration::from_secs(1));
    }
}
