use log::{debug, warn};
use prometheus_exporter::prometheus::core::{AtomicF64, GenericGauge, GenericGaugeVec};
use prometheus_exporter::prometheus::{register_gauge};
use prometheus_exporter::prometheus;
use std::collections::HashMap;
use std::{env, time};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use rups::blocking::Connection;

const BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

pub struct Config {
    ups_name: String,
    ups_host: String,
    ups_port: u16,
    bind_addr: SocketAddr,
    poll_rate: u64,
    rups_config: rups::Config,
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
        let rups_config = rups::ConfigBuilder::new()
            .with_host((ups_host.clone(), ups_port).try_into().unwrap_or_default())
            .with_debug(false) // Turn this on for debugging network chatter
            .with_timeout(time::Duration::from_secs(poll_rate - 1))
            .build();
        let bind_addr = SocketAddr::new(BIND_IP, bind_port);

        Ok(Config {
            ups_name,
            ups_host,
            ups_port,
            bind_addr,
            poll_rate,
            rups_config,
        })
    }

    pub fn ups_fullname(&self) -> String {
        format!("{}@{}:{}", self.ups_name, self.ups_host, self.ups_port)
    }

    pub fn ups_name(&self) -> &str {
        self.ups_name.as_str()
    }

    pub fn rups_config(&self) -> &rups::Config {
        &self.rups_config
    }

    pub fn poll_rate(&self) -> &u64 {
        &self.poll_rate
    }

    pub fn bind_addr(&self) -> &SocketAddr {
        &self.bind_addr
    }
}

pub fn get_available_vars(conn: &mut Connection, ups_name: &str) -> Result<HashMap<String, (String, String)>, rups::ClientError> {
    let available_vars = conn.list_vars(ups_name)?;
    let mut ups_vars = HashMap::new();
    for var in &available_vars {
        let raw_name = var.name();
        let description = conn.get_var_description(ups_name, raw_name)?;
        ups_vars.insert(raw_name.to_string(), (var.value(), description));
    }
    Ok(ups_vars)
}

pub fn create_basic_gauges(vars: &HashMap<String, (String, String)>) -> Result<HashMap<String,GenericGauge<AtomicF64>>, prometheus::Error> {
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
    fn create_basic_gauges_multiple() {
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
        let gauges = create_basic_gauges(&variables).unwrap();
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
    fn create_basic_gauges_no_ups() {
        // Create variable map
        let mut variables = HashMap::new();
        variables.insert(
            "battery.charge".to_string(),
            ("20".to_string(), "Battery Charge".to_string()),
        );

        // Test creation function
        let gauges = create_basic_gauges(&variables).unwrap();
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
    fn create_basic_gauges_skip_non_float() {
        // Create variable map
        let mut variables = HashMap::new();
        variables.insert(
            "ups.mfr".to_string(),
            ("CyberPower".to_string(), "Manufacturer".to_string()),
        );

        // Test creation function
        let gauges = create_basic_gauges(&variables).unwrap();
        assert_eq!(gauges.len(), 0);
        dbg!(gauges);
    }
}
