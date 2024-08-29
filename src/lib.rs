use log::{debug, warn};
use prometheus_exporter::prometheus::core::{AtomicF64, GenericGauge, GenericGaugeVec};
use prometheus_exporter::prometheus::{register_gauge};
use prometheus_exporter::prometheus;
use std::collections::HashMap;
use std::env;

pub struct Config {
    pub ups_name: String,
    pub ups_host: String,
    pub ups_port: u16,
    pub bind_port: u16,
    pub poll_rate: u64,
}

impl Config {
    pub fn build() -> Result<Config, &'static str> {
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
        Ok(Config {
            ups_name,
            ups_host,
            ups_port,
            bind_port,
            poll_rate
        })
    }
}

pub fn create_gauges(vars: &HashMap<String, (String, String)>) -> Result<HashMap<String,GenericGauge<AtomicF64>>, prometheus::Error> {
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

pub fn update_label_gauge(label_gauge: &GenericGaugeVec<AtomicF64>, states: &[&str], value: &str) {
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
