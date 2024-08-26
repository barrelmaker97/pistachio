use env_logger::{Builder, Env};
use log::{debug, info, warn};
use prometheus_exporter::prometheus;
use prometheus_exporter::prometheus::core::{AtomicF64, GenericGauge, GenericGaugeVec};
use prometheus_exporter::prometheus::{register_gauge, register_gauge_vec};
use rups::blocking::Connection;
use rups::ConfigBuilder;
use std::collections::HashMap;
use std::convert::TryInto;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::{env, thread, time};

const BIND_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));

fn main() {
    // Initialize logging
    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    // Declare state arrays for UPS status and beeper status
    let statuses: &[&str] = &["OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD"];
    let beeper_statuses: &[&str] = &["enabled", "disabled", "muted"];

    // Read config from the environment
    let (ups_name, ups_host, ups_port, bind_port, poll_rate) = parse_config();

    // Log config info
    info!("UPS to be checked: {ups_name}@{ups_host}:{ups_port}");
    info!("Poll Rate: Every {poll_rate} seconds");

    // Create connection to UPS
    let config = ConfigBuilder::new()
        .with_host((ups_host, ups_port).try_into().unwrap_or_default())
        .with_debug(false) // Turn this on for debugging network chatter
        .with_timeout(time::Duration::from_secs(poll_rate - 1))
        .build();
    let mut conn = Connection::new(&config).expect("Failed to connect to the UPS");

    // Get list of available UPS variables and map them to a tuple of their values and descriptions
    let available_vars = conn
        .list_vars(&ups_name)
        .expect("Failed to get available variables from the UPS");
    let mut ups_vars = HashMap::new();
    for var in &available_vars {
        let raw_name = var.name();
        let description = conn
            .get_var_description(&ups_name, raw_name)
            .expect("Failed to get description for a variable");
        ups_vars.insert(raw_name.to_string(), (var.value(), description));
    }

    // Use map of available UPS variables to create a map of associated prometheus gauges
    // Gauges must be floats, so this will only create gauges for variables that are numbers
    let gauges = create_gauges(&ups_vars).expect("Could not create gauges");

    // Create label gauges
    let status_gauge = register_gauge_vec!("ups_status", "UPS Status Code", &["status"])
        .expect("Cannot create status gauge");
    let beeper_gauge = register_gauge_vec!("beeper_status", "Beeper Status", &["status"])
        .expect("Cannot create beeper status gauge");
    info!("{} basic gauges and 2 labeled gauges will be exported", gauges.len());

    // Start prometheus exporter
    let addr = SocketAddr::new(BIND_ADDR, bind_port);
    prometheus_exporter::start(addr).expect("Failed to start prometheus exporter");

    // Main loop that polls for variables and updates associated gauges
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(&ups_name) {
            Ok(var_list) => {
                for var in var_list {
                    if let Ok(value) = var.value().parse::<f64>() {
                        // Update basic gauges
                        if let Some(gauge) = gauges.get(var.name()) {
                            gauge.set(value);
                        } else {
                            warn!("Gauge does not exist for variable {}", var.name());
                        }
                    } else if var.name() == "ups.status" {
                        update_label_gauge(&status_gauge, statuses, &var.value());
                    } else if var.name() == "ups.beeper.status" {
                        update_label_gauge(&beeper_gauge, beeper_statuses, &var.value());
                    } else {
                        debug!("Variable {} does not have an associated gauge to update", var.name());
                    }
                }
            }
            Err(err) => {
                // Log warning and set gauges to 0 to indicate failure
                warn!("Failed to connect to the UPS");
                debug!("Err: {err}");
                for gauge in gauges.values() {
                    gauge.set(0.0);
                }
                for state in statuses {
                    status_gauge
                        .get_metric_with_label_values(&[state])
                        .unwrap()
                        .set(0.0);
                }
                for state in beeper_statuses {
                    beeper_gauge
                        .get_metric_with_label_values(&[state])
                        .unwrap()
                        .set(0.0);
                }
                debug!("Reset gauges to zero because the UPS was unreachable");
            }
        }
        thread::sleep(time::Duration::from_secs(poll_rate));
    }
}

fn parse_config() -> (String, String, u16, u16, u64) {
    let ups_name = env::var("UPS_NAME").unwrap_or_else(|_| "ups".into());
    let ups_host = env::var("UPS_HOST").unwrap_or_else(|_| "localhost".into());
    let ups_port = env::var("UPS_PORT")
        .and_then(|s| s.parse::<u16>().map_err(|_| env::VarError::NotPresent))
        .unwrap_or(3493);
    let bind_port = env::var("BIND_PORT")
        .and_then(|s| s.parse::<u16>().map_err(|_| env::VarError::NotPresent))
        .unwrap_or(9120);
    let mut poll_rate = env::var("POLL_RATE")
        .and_then(|s| s.parse::<u64>().map_err(|_| env::VarError::NotPresent))
        .unwrap_or(10);
    if poll_rate < 2 {
        warn!("POLL_RATE is too low, increasing to minimum of 2 seconds");
        poll_rate = 2;
    }
    (ups_name, ups_host, ups_port, bind_port, poll_rate)
}

fn create_gauges(vars: &HashMap<String, (String, String)>) -> Result<HashMap<String,GenericGauge<AtomicF64>>, prometheus::Error> {
    let mut gauges = HashMap::new();
    for (raw_name, (value, description)) in vars {
        match value.parse::<f64>() {
            Ok(_) => {
                let mut gauge_name = raw_name.replace('.', "_");
                if !gauge_name.starts_with("ups") {
                    gauge_name.insert_str(0, "ups_");
                }
                let gauge = register_gauge!(gauge_name, description)?;
                gauges.insert(raw_name.to_string(), gauge);
                debug!("Gauge created for variable {raw_name}");
            }
            Err(_) => {
                debug!("Not creating a gauge for variable {raw_name} since it is not a number");
            }
        }
    }
    Ok(gauges)
}

fn update_label_gauge(label_gauge: &GenericGaugeVec<AtomicF64>, states: &[&str], value: &str) {
    for state in states {
        if let Ok(gauge) = label_gauge.get_metric_with_label_values(&[state]) {
            if value.contains(state) {
                gauge.set(1.0);
            } else {
                gauge.set(0.0);
            }
        } else {
            warn!("Failed to update label gauge for {} state", state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_exporter::prometheus::core::Collector;

    #[test]
    fn create_gauges_multiple() {
        // Create variable map
        let mut variables = HashMap::new();
        variables.insert(
            "ups.var1".to_string(),
            ("20".to_string(), "Variable1".to_string()),
        );
        variables.insert(
            "ups.var2".to_string(),
            ("20".to_string(), "Variable2".to_string()),
        );
        variables.insert(
            "ups.var3".to_string(),
            ("20".to_string(), "Variable3".to_string()),
        );
        variables.insert(
            "ups.var4".to_string(),
            ("20".to_string(), "Variable4".to_string()),
        );

        // Test creation function
        let gauges = create_gauges(&variables).unwrap();
        assert_eq!(gauges.len(), variables.len());
        for (name, gauge) in &gauges {
            let gauge_desc = &gauge.desc().pop().unwrap().help;
            let gauge_name = &gauge.desc().pop().unwrap().fq_name;
            let (expected_name, (_, expected_desc)) =
                variables.get_key_value(name.as_str()).unwrap();
            assert_eq!(name, expected_name);
            assert_eq!(gauge_desc, expected_desc);
            assert!(gauge_name.starts_with("ups"));
            assert!(!gauge_name.contains("."));
        }
        dbg!(gauges);
    }

    #[test]
    fn create_gauges_no_ups() {
        // Create variable map
        let mut variables = HashMap::new();
        variables.insert(
            "battery.charge".to_string(),
            ("20".to_string(), "Battery Charge".to_string()),
        );

        // Test creation function
        let gauges = create_gauges(&variables).unwrap();
        assert_eq!(gauges.len(), variables.len());
        for (name, gauge) in &gauges {
            let gauge_desc = &gauge.desc().pop().unwrap().help;
            let gauge_name = &gauge.desc().pop().unwrap().fq_name;
            let (expected_name, (_, expected_desc)) =
                variables.get_key_value(name.as_str()).unwrap();
            assert_eq!(name, expected_name);
            assert_eq!(gauge_desc, expected_desc);
            assert!(gauge_name.starts_with("ups"));
            assert!(!gauge_name.contains("."));
        }
        dbg!(gauges);
    }

    #[test]
    fn create_gauges_skip_non_float() {
        // Create variable map
        let mut variables = HashMap::new();
        variables.insert(
            "ups.mfr".to_string(),
            ("CyberPower".to_string(), "Manufacturer".to_string()),
        );

        // Test creation function
        let gauges = create_gauges(&variables).unwrap();
        assert_eq!(gauges.len(), 0);
        dbg!(gauges);
    }
}
