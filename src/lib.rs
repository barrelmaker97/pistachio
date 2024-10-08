#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use clap::Parser;
use log::{debug, info, warn};
use prometheus_exporter::prometheus;
use prometheus_exporter::prometheus::core::{AtomicF64, GenericGauge, GenericGaugeVec};
use prometheus_exporter::prometheus::{register_gauge, register_gauge_vec};
use rups::blocking::Connection;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::thread;
use std::time::Duration;

/// Default configuration options
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

/// A collection of arguments to be parsed from the command line or environment.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Name of the UPS to monitor. Default is `ups`.
    #[arg(long, env, default_value_t = String::from(DEFAULT_UPS_NAME))]
    pub ups_name: String,
    /// Hostname of the NUT server to monitor. Default is `127.0.0.1`.
    #[arg(long, env, default_value_t = String::from(DEFAULT_UPS_HOST))]
    pub ups_host: String,
    /// Port of the NUT server to monitor. Default is `3493`.
    #[arg(long, env, default_value_t = DEFAULT_UPS_PORT)]
    pub ups_port: u16,
    /// IP address on which the exporter will serve metrics. Default is `0.0.0.0`.
    #[arg(long, env, default_value_t = DEFAULT_BIND_IP)]
    pub bind_ip: IpAddr,
    /// Port on which the exporter will serve metrics. Default is `9120`.
    #[arg(long, env, default_value_t = DEFAULT_BIND_PORT)]
    pub bind_port: u16,
    /// Time in seconds between requests to the NUT server. Must be at least 1 second. Default is `10`.
    #[arg(long, env, default_value_t = DEFAULT_POLL_RATE, value_parser = clap::value_parser!(u64).range(1..))]
    pub poll_rate: u64,
}

/// A collection of all registered Prometheus metrics, mapped to the name of the UPS variable they represent.
#[derive(Debug)]
pub struct Metrics {
    basic_gauges: HashMap<String, GenericGauge<AtomicF64>>,
    label_gauges: HashMap<String, (GenericGaugeVec<AtomicF64>, &'static [&'static str])>,
}

impl Metrics {
    /// A builder that creates a Metrics instance from a map of variable names, values, and descriptions.
    ///
    /// # Errors
    ///
    /// An error will be returned if any of metrics cannot be created and registered with the
    /// Prometheus expoter, such as if two metrics attempt to use the same name.
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
                    warn!("Failed to update gauge {} because the value was not a float", var.name());
                }
            } else if let Some((label_gauge, states)) = self.label_gauges.get(var.name()) {
                update_label_gauge(label_gauge, states, &var.value());
            } else {
                debug!("Variable {} does not have an associated gauge to update", var.name());
            }
        }
    }

    /// Resets all metrics to zero.
    ///
    /// # Errors
    ///
    /// An error will be returned if any of the metrics to be reset cannot be accessed.
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

/// Creates a connection for communicating with the NUT server.
///
/// # Errors
///
/// An error will be returned if the UPS host and port in the provided [Args] cannot be used to
/// create a valid [`rups::Host`].
pub fn create_connection(args: &Args) -> Result<Connection, rups::ClientError> {
    // Create connection to UPS
    let rups_host = rups::Host::try_from((args.ups_host.clone(), args.ups_port))?;
    let rups_config = rups::ConfigBuilder::new().with_host(rups_host).build();
    Connection::new(&rups_config)
}

/// Connects to the NUT server to produce a map of all available UPS variables, along with their
/// values and descriptions.
///
/// # Errors
///
/// An error will be returned if the list of variables or their descriptions cannot be retrieved
/// from the NUT server, such as if connection to the server is lost.
pub fn get_ups_vars(args: &Args, conn: &mut Connection) -> Result<HashMap<String, (String, String)>, rups::ClientError> {
    // Get available vars
    let ups_name = args.ups_name.as_str();
    let available_vars = conn.list_vars(ups_name)?;
    let mut ups_vars = HashMap::new();
    for var in &available_vars {
        let description = conn.get_var_description(ups_name, var.name())?;
        ups_vars.insert(var.name().to_string(), (var.value(), description));
    }
    Ok(ups_vars)
}

/// Main loop that polls the NUT server and updates associated gauges
pub fn run(args: &Args, conn: &mut Connection, metrics: &Metrics) {
    let mut is_failing = false;
    loop {
        debug!("Polling UPS...");
        match conn.list_vars(args.ups_name.as_str()) {
            Ok(var_list) => {
                metrics.update(&var_list);
                debug!("Metrics updated");
                if is_failing {
                    info!("Connection with the UPS has been reestablished");
                    is_failing = false;
                }
            }
            Err(err) => {
                // Log warning and set gauges to 0 to indicate failure
                warn!("Failed to connect to the UPS: {err}");
                metrics.reset().unwrap_or_else(|err| {
                    warn!("Failed to reset gauges to zero: {err}");
                });
                debug!("Reset gauges to zero because the UPS was unreachable");
                is_failing = true;
            }
        }
        thread::sleep(Duration::from_secs(args.poll_rate));
    }
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
    fn parse_default_args() {
        let args = Args::parse();
        assert_eq!(args.ups_name, DEFAULT_UPS_NAME);
        assert_eq!(args.ups_host, DEFAULT_UPS_HOST);
        assert_eq!(args.ups_port, DEFAULT_UPS_PORT);
        assert_eq!(args.bind_ip, DEFAULT_BIND_IP);
        assert_eq!(args.bind_port, DEFAULT_BIND_PORT);
        assert_eq!(args.poll_rate, DEFAULT_POLL_RATE);
    }

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
