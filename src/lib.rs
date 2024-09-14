#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use log::{debug, warn};
use prometheus_exporter::prometheus;
use prometheus_exporter::prometheus::core::{AtomicF64, GenericGauge, GenericGaugeVec};
use prometheus_exporter::prometheus::{register_gauge, register_gauge_vec};
use rups::blocking::Connection;
use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Default bind address for the Prometheus exporter
const DEFAULT_UPS_NAME: &str = "ups";
const DEFAULT_UPS_HOST: &str = "127.0.0.1";
const DEFAULT_UPS_PORT: u16 = 3493;
const DEFAULT_BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
const DEFAULT_BIND_PORT: u16 = 9120;
const DEFAULT_POLL_RATE: u64 = 10;

/// An array of possible UPS system states
const STATUSES: &[&str] = &["OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD"];

/// An array of possible UPS beeper states
const BEEPER_STATUSES: &[&str] = &["enabled", "disabled", "muted"];

/// Configuration for connecting to and polling a NUT server as well as the bind address of a
/// Prometheus exporter.
#[derive(Debug)]
pub struct Config {
    ups_name: String,
    ups_host: String,
    ups_port: u16,
    bind_addr: SocketAddr,
    poll_rate: u64,
    rups_config: rups::Config,
}

impl Config {
    /// A builder that creates a configuration by reading config values from the environment
    /// The following table shows which env vars are read and their default values:
    ///
    /// | Parameter   | Description                                                      | Default     |
    /// |-------------|------------------------------------------------------------------|-------------|
    /// | `UPS_NAME`  | Name of the UPS to monitor.                                      | `ups`       |
    /// | `UPS_HOST`  | Hostname of the NUT server to monitor.                           | `127.0.0.1` |
    /// | `UPS_PORT`  | Port of the NUT server to monitor.                               | `3493`      |
    /// | `BIND_IP`   | IP address on which the exporter will serve metrics.             | `0.0.0.0`   |
    /// | `BIND_PORT` | Port on which the exporter will serve metrics.                   | `9120`      |
    /// | `POLL_RATE` | Time in seconds between requests to the NUT server. Must be < 1. | `10`        |
    pub fn build() -> Result<Config, &'static str> {
        let ups_name = env::var("UPS_NAME").unwrap_or_else(|_| DEFAULT_UPS_NAME.into());
        let ups_host = env::var("UPS_HOST").unwrap_or_else(|_| DEFAULT_UPS_HOST.into());
        let ups_port = env::var("UPS_PORT")
            .and_then(|s| s.parse::<u16>().map_err(|_| env::VarError::NotPresent))
            .unwrap_or(DEFAULT_UPS_PORT);
        let bind_ip = env::var("BIND_IP")
            .and_then(|s| s.parse::<IpAddr>().map_err(|_| env::VarError::NotPresent))
            .unwrap_or(DEFAULT_BIND_IP);
        let bind_port = env::var("BIND_PORT")
            .and_then(|s| s.parse::<u16>().map_err(|_| env::VarError::NotPresent))
            .unwrap_or(DEFAULT_BIND_PORT);
        let mut poll_rate = env::var("POLL_RATE")
            .and_then(|s| s.parse::<u64>().map_err(|_| env::VarError::NotPresent))
            .unwrap_or(DEFAULT_POLL_RATE);
        if poll_rate < 2 {
            warn!("POLL_RATE is too low, increasing to minimum of 2 seconds");
            poll_rate = 2;
        }
        let rups_config = rups::ConfigBuilder::new()
            .with_host((ups_host.clone(), ups_port).try_into().unwrap_or_default())
            .with_timeout(Duration::from_secs(poll_rate - 1))
            .build();
        let bind_addr = SocketAddr::new(bind_ip, bind_port);

        Ok(Config {
            ups_name,
            ups_host,
            ups_port,
            bind_addr,
            poll_rate,
            rups_config,
        })
    }

    /// Returns the full name of the UPS defined in the configuration. The full name uses the
    /// format of `ups@host:port`.
    #[must_use]
    pub fn ups_fullname(&self) -> String {
        format!("{}@{}:{}", self.ups_name, self.ups_host, self.ups_port)
    }

    /// Returns the name of the UPS defined in the configuration.
    #[must_use]
    pub fn ups_name(&self) -> &str {
        self.ups_name.as_str()
    }

    /// Returns the rups configuration which is used to create a connection to the UPS.
    #[must_use]
    pub fn rups_config(&self) -> &rups::Config {
        &self.rups_config
    }

    /// Returns the rate at which the NUT server will be polled for data, in seconds.
    #[must_use]
    pub fn poll_rate(&self) -> &u64 {
        &self.poll_rate
    }

    /// Returns the address at which the Prometheus exprter will run.
    #[must_use]
    pub fn bind_addr(&self) -> &SocketAddr {
        &self.bind_addr
    }
}

/// A collection of all registered Prometheus metrics, mapped to the name of the UPS variable they represent.
#[derive(Debug)]
pub struct Metrics {
    basic_gauges: HashMap<String, GenericGauge<AtomicF64>>,
    label_gauges: HashMap<String, (GenericGaugeVec<AtomicF64>, &'static [&'static str])>,
}

impl Metrics {
    /// A builder that creates metrics from a map of variable names, values, and descriptions.
    pub fn build(ups_vars: &HashMap<String, (String, String)>) -> Result<Metrics, prometheus::Error> {
        let basic_gauges = create_basic_gauges(ups_vars)?;
        let label_gauges = create_label_gauges()?;

        Ok(Metrics {
            basic_gauges,
            label_gauges,
        })
    }

    /// Returns the number of all gauges registered.
    #[must_use]
    pub fn count(&self) -> usize {
        self.basic_gauges.len() + self.label_gauges.len()
    }

    /// Takes a list of variable names and values to update all associated Prometheus metrics.
    pub fn update(&self, var_list: &Vec<rups::Variable>) {
        for var in var_list {
            if let Some(gauge) = self.basic_gauges.get(var.name()) {
                // Update basic gauges
                if let Ok(value) = var.value().parse::<f64>() {
                    gauge.set(value);
                } else {
                    warn!("Failed to update variable {} because the value was not a float", var.name());
                }
            } else if let Some((label_gauge, states)) = self.label_gauges.get(var.name()) {
                update_label_gauge(label_gauge, states, &var.value());
            } else {
                debug!("Variable {} does not have an associated gauge to update", var.name());
            }
        }
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) -> Result<(), prometheus::Error> {
        for gauge in self.basic_gauges.values() {
            gauge.set(0.0);
        }
        for (label_gauge, states) in self.label_gauges.values() {
            for state in *states {
                let gauge = label_gauge.get_metric_with_label_values(&[state])?;
                gauge.set(0.0);
            }
        }
        Ok(())
    }
}

/// Connects to the NUT server to produce a map of all available UPS variables, along with their
/// values and descriptions.
pub fn get_ups_vars(conn: &mut Connection, ups_name: &str) -> Result<HashMap<String, (String, String)>, rups::ClientError> {
    let available_vars = conn.list_vars(ups_name)?;
    let mut ups_vars = HashMap::new();
    for var in &available_vars {
        let raw_name = var.name();
        let description = conn.get_var_description(ups_name, raw_name)?;
        ups_vars.insert(raw_name.to_string(), (var.value(), description));
    }
    Ok(ups_vars)
}

/// Takes a map of UPS variables, values, and descriptions to create Prometheus gauges. Gauges are
/// only created for variables with values that can be parsed as floats, since Prometheus gauges can
/// only have floats as values.
fn create_basic_gauges(vars: &HashMap<String, (String, String)>) -> Result<HashMap<String,GenericGauge<AtomicF64>>, prometheus::Error> {
    let mut gauges = HashMap::new();
    for (raw_name, (_, description)) in vars.iter().filter(|(_, (y, _))| y.parse::<f64>().is_ok()) {
        let mut gauge_name = raw_name.replace('.', "_");
        if !gauge_name.starts_with("ups") {
            gauge_name.insert_str(0, "ups_");
        }
        let gauge = register_gauge!(gauge_name, description)?;
        gauges.insert(raw_name.to_string(), gauge);
        debug!("Gauge created for variable {raw_name}");
    }
    Ok(gauges)
}

/// Creates label gauges in Prometheus for UPS variables that represent a set of potential status.
/// This currently only includes overall UPS status and beeper status.
fn create_label_gauges() -> Result<HashMap<String,(GenericGaugeVec<AtomicF64>, &'static [&'static str])>, prometheus::Error> {
    let mut label_gauges = HashMap::new();
    let status_gauge = register_gauge_vec!("ups_status", "UPS Status Code", &["status"])?;
    let beeper_gauge = register_gauge_vec!("ups_beeper_status", "Beeper Status", &["status"])?;
    label_gauges.insert(
        String::from("ups.status"),
        (status_gauge, STATUSES),
    );
    label_gauges.insert(
        String::from("ups.beeper.status"),
        (beeper_gauge, BEEPER_STATUSES),
    );
    Ok(label_gauges)
}

/// Takes a label gauge, all of it's possible states, and the current value of the variable from
/// the UPS. Each label of the gauge is updated to reflect all current states present in the
/// value from the UPS.
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

    #[test]
    fn create_config() {
        let config = Config::build().unwrap();
        dbg!(&config);
        assert_eq!(config.ups_fullname(), format!("{DEFAULT_UPS_NAME}@{DEFAULT_UPS_HOST}:{DEFAULT_UPS_PORT}"));
        assert_eq!(config.ups_name(), DEFAULT_UPS_NAME);
        assert_eq!(*config.poll_rate(), DEFAULT_POLL_RATE);
        assert_eq!(*config.bind_addr(), SocketAddr::new(BIND_IP, DEFAULT_BIND_PORT));
    }

    #[test]
    fn create_metrics() {
        // Setup
        let registry = prometheus::default_registry();
        let mut variables = HashMap::new();
        variables.insert(
            "ups.var5".to_string(),
            ("20".to_string(), "Variable5".to_string()),
        );

        // Create metrics instance
        let metrics = Metrics::build(&variables).unwrap();
        assert_eq!(3, metrics.count()); // Will have 3 since 2 label gauges are always created

        // Update metrics
        let basic_var: rups::Variable = rups::Variable::parse("ups.var5", String::from("30"));
        let label_var: rups::Variable = rups::Variable::parse("ups.status", String::from("OL"));
        let var_list = vec![basic_var, label_var];
        metrics.update(&var_list);

        // Check updated metric values
        for metric_family in registry.gather() {
            if metric_family.get_name() == "ups_var5" {
                let gauge = metric_family.get_metric()[0].get_gauge();
                dbg!(gauge);
                assert_eq!(30.0, gauge.get_value());
            } else if metric_family.get_name() == "ups_status" {
                for metric in metric_family.get_metric() {
                    if metric.get_label()[0].get_value() == "OL" {
                        assert_eq!(1.0, metric.get_gauge().get_value());
                    } else {
                        assert_eq!(0.0, metric.get_gauge().get_value());
                    }
                }
            }
        }

        // Reset metrics
        metrics.reset().unwrap();

        // Check reset metric values
        for metric_family in registry.gather() {
            if metric_family.get_name() == "ups_var5" {
                let gauge = metric_family.get_metric()[0].get_gauge();
                dbg!(gauge);
                assert_eq!(0.0, gauge.get_value());
            } else if metric_family.get_name() == "ups_status" {
                for metric in metric_family.get_metric() {
                    assert_eq!(0.0, metric.get_gauge().get_value());
                }
            }
        }
    }
}
