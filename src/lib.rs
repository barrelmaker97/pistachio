#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use clap::Parser;
use log::{debug, info, warn};
use metrics::{describe_gauge, gauge};
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
    basic_gauges: Vec<String>,
    label_gauges: HashMap<String, &'static [&'static str]>,
}

impl Metrics {
    /// A builder that creates a Metrics instance from a map of variable names, values, and descriptions.
    /// Gauges are only registered for variables with values that can be parsed as floats, since
    /// Prometheus gauges can only have floats as values.
    pub fn build(ups_vars: &HashMap<String, (String, String)>) -> Metrics {
        let basic_gauges = ups_vars.iter()
            .filter_map(|(name, (value, desc))| {
                value.parse::<f64>().ok().map(|_| {
                    let name = convert_name(name);
                    describe_gauge!(name.clone(), desc.clone());
                    debug!("Gauge {name} has been registered");
                    name
                })
            })
            .collect();

        // Registers label gauges in Prometheus for UPS variables that represent a set of potential status.
        // This currently only includes overall UPS status and beeper status.
        describe_gauge!("ups_status", "UPS Status Code");
        describe_gauge!("ups_beeper_status", "Beeper Status");
        let mut label_gauges = HashMap::new();
        label_gauges.insert(String::from("ups_status"), STATUSES);
        label_gauges.insert(String::from("ups_beeper_status"), BEEPER_STATUSES);

        Metrics {
            basic_gauges,
            label_gauges,
        }
    }

    /// Returns the number of all gauges registered.
    #[must_use]
    pub fn count(&self) -> usize {
        self.basic_gauges.len() + self.label_gauges.len()
    }

    /// Takes a list of variable names and values to update all associated gauges. For label
    /// gauges, each label of the gauge is updated to reflect all current states present in the
    /// value from the UPS.
    pub fn update(&self, var_list: &Vec<rups::Variable>) {
        for (gauge_name, value) in var_list.iter().map(|x| (convert_name(x.name()), x.value())) {
            if self.basic_gauges.contains(&gauge_name) {
                // Update basic gauges
                if let Ok(value) = value.parse::<f64>() {
                    gauge!(gauge_name).set(value);
                } else {
                    warn!("Failed to update gauge {gauge_name} because the value was not a float");
                }
            } else if let Some(states) = self.label_gauges.get(&gauge_name) {
                for state in *states {
                    if value.contains(state) {
                        gauge!(gauge_name.to_string(), "status" => state.to_string()).set(1.0);
                    } else {
                        gauge!(gauge_name.to_string(), "status" => state.to_string()).set(0.0);
                    }
                }
            } else {
                debug!("Variable {gauge_name} does not have an associated gauge to update");
            }
        }
    }

    /// Resets all metrics to zero.
    ///
    /// # Errors
    ///
    /// An error will be returned if any of the metrics to be reset cannot be accessed.
    pub fn reset(&self) {
        for gauge_name in &self.basic_gauges {
            gauge!(gauge_name.to_string()).set(0.0);
        }
        for (gauge_name, states) in &self.label_gauges {
            for state in *states {
                gauge!(gauge_name.to_string(), "status" => state.to_string()).set(0.0);
            }
        }
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
                metrics.reset();
                debug!("Reset gauges to zero because the UPS was unreachable");
                is_failing = true;
            }
        }
        thread::sleep(Duration::from_secs(args.poll_rate));
    }
}

fn convert_name(var_name: &str) -> String {
    let mut gauge_name = var_name.replace('.', "_");
    if !gauge_name.starts_with("ups") {
        gauge_name.insert_str(0, "ups_");
    }
    gauge_name
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
