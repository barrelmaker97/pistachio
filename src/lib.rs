#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use clap::Parser;
use log::{debug, warn};
use metrics::{describe_gauge, gauge};
use rups::tokio::Connection;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

/// Default configuration options
const DEFAULT_UPS_NAME: &str = "ups";
const DEFAULT_UPS_HOST: &str = "127.0.0.1";
const DEFAULT_UPS_PORT: u16 = 3493;
const DEFAULT_BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
const DEFAULT_BIND_PORT: u16 = 9120;
const DEFAULT_POLL_RATE: u64 = 10;

/// An array of possible UPS system states
const UPS_STATES: &[&str] = &["OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD"];

/// An array of possible UPS beeper states
const BEEPER_STATES: &[&str] = &["enabled", "disabled", "muted"];

/// An array of possible UPS beeper states
const STATUS_VARS: &[(&str, &str, &[&str])] = &[("ups.status", "UPS Status Code", UPS_STATES), ("ups.beeper.status", "Beeper Status", BEEPER_STATES)];

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

/// A collection of all registered metrics, both labelled and unlabelled.
#[derive(Debug)]
pub struct Metrics {
    basic_gauges: HashMap<String, String>,
    label_gauges: HashMap<String, (String, &'static [&'static str])>,
}

impl Metrics {
    /// A builder that creates a Metrics instance from a map of variable names, values, and descriptions.
    /// Gauges are only registered for variables with values that can be parsed as floats, since
    /// gauges can only have floats as values.
    #[must_use]
    pub fn build(ups_vars: &HashMap<String, (String, String)>) -> Self {
        let basic_gauges = ups_vars.iter()
            .filter_map(|(name, (value, desc))| {
                value.parse::<f64>().ok().map(|_| {
                    let gauge_name = convert_var_name(name);
                    describe_gauge!(gauge_name.clone(), desc.clone());
                    debug!("Gauge {gauge_name} has been registered for var {name}");
                    (name.clone(), gauge_name)
                })
            })
            .collect();

        // Registers label gauges for UPS variables that represent a set of potential status.
        // This currently only includes overall UPS status and beeper status.
        let label_gauges = STATUS_VARS.iter()
            .map(|(name, desc, states)| {
                let gauge_name = convert_var_name(name);
                describe_gauge!(gauge_name.clone(), *desc);
                debug!("Gauge {gauge_name} has been registered for var {name}");
                ((*name).to_owned(), (gauge_name, *states))
            })
            .collect();

        Self {
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
        for var in var_list {
            if let Some(gauge_name) = self.basic_gauges.get(var.name()) {
                // Update basic gauges
                if let Ok(value) = var.value().parse::<f64>() {
                    gauge!(gauge_name.clone()).set(value);
                } else {
                    warn!("Failed to update gauge {gauge_name} because the value was not a float");
                }
            } else if let Some((gauge_name, states)) = self.label_gauges.get(var.name()) {
                // Update label gauges
                for (state, is_active) in states.iter().map(|x| ((*x).to_owned(), var.value().contains(x))) {
                    gauge!(gauge_name.clone(), "status" => state).set(u8::from(is_active));
                }
            } else {
                debug!("Variable {} does not have an associated gauge to update", var.name());
            }
        }
    }

    /// Resets all metrics to zero.
    pub fn reset(&self) {
        for gauge_name in self.basic_gauges.values() {
            gauge!(gauge_name.clone()).set(0.0);
        }
        for (gauge_name, states) in self.label_gauges.values() {
            for state in states.iter().map(|x| (*x).to_owned()) {
                gauge!(gauge_name.clone(), "status" => state).set(0.0);
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
pub async fn create_connection(args: &Args) -> Result<Connection, rups::ClientError> {
    // Create connection to UPS
    let rups_host = rups::Host::try_from((args.ups_host.clone(), args.ups_port))?;
    let rups_config = rups::ConfigBuilder::new().with_host(rups_host).build();
    Connection::new(&rups_config).await
}

/// Connects to the NUT server to produce a map of all available UPS variables, along with their
/// values and descriptions.
///
/// # Errors
///
/// An error will be returned if the list of variables or their descriptions cannot be retrieved
/// from the NUT server, such as if connection to the server is lost.
pub async fn get_ups_vars(args: &Args, conn: &mut Connection) -> Result<HashMap<String, (String, String)>, rups::ClientError> {
    // Get available vars
    let ups_name = args.ups_name.as_str();
    let available_vars = conn.list_vars(ups_name).await?;
    let mut ups_vars = HashMap::new();
    for var in &available_vars {
        let description = conn.get_var_description(ups_name, var.name()).await?;
        ups_vars.insert(var.name().to_owned(), (var.value(), description));
    }
    Ok(ups_vars)
}

fn convert_var_name(name: &str) -> String {
    let mut gauge_name = name.replace('.', "_");
    if !gauge_name.starts_with("ups") {
        gauge_name.insert_str(0, "ups_");
    }
    gauge_name
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_override_args() {
        let args = Args::parse_from(["pistachio", "--ups-name", "test_ups", "--ups-host", "192.168.1.1", "--ups-port", "1234", "--bind-ip", "10.0.0.1", "--bind-port", "5678", "--poll-rate", "5"]);
        assert_eq!(args.ups_name, "test_ups");
        assert_eq!(args.ups_host, "192.168.1.1");
        assert_eq!(args.ups_port, 1234);
        assert_eq!(args.bind_ip, IpAddr::V4("10.0.0.1".parse().unwrap()));
        assert_eq!(args.bind_port, 5678);
        assert_eq!(args.poll_rate, 5);
    }

    #[test]
    fn build_metrics_basic() {
        let mut ups_vars = HashMap::new();
        ups_vars.insert("input.voltage".to_string(), (String::from("122.0"), String::from("Nominal input voltage")));
        ups_vars.insert("ups.load".to_string(), (String::from("25.5"), String::from("UPS load in percent")));
        ups_vars.insert("battery.charge".to_string(), (String::from("100.0"), String::from("Battery charge in percent")));
        let expected_metric_name1 = convert_var_name("input.voltage");
        let expected_metric_name2 = convert_var_name("ups.load");
        let expected_metric_name3 = convert_var_name("battery.charge");

        let metrics = Metrics::build(&ups_vars);

        assert_eq!(metrics.basic_gauges.len(), 3);
        assert_eq!(*metrics.basic_gauges.get("input.voltage").unwrap(), expected_metric_name1);
        assert_eq!(*metrics.basic_gauges.get("ups.load").unwrap(), expected_metric_name2);
        assert_eq!(*metrics.basic_gauges.get("battery.charge").unwrap(), expected_metric_name3);
    }

    #[test]
    fn build_metrics_not_float() {
        let mut ups_vars = HashMap::new();
        let var_name = "ups.mfr";
        ups_vars.insert(var_name.to_string(), (String::from("CPS"), String::from("UPS Manufacturer")));

        let metrics = Metrics::build(&ups_vars);

        assert_eq!(metrics.basic_gauges.len(), 0);
    }

    #[test]
    fn build_metrics_label_gauges() {
        let ups_vars = HashMap::new();

        let metrics = Metrics::build(&ups_vars);

        assert_eq!(metrics.count(), 2);
        assert_eq!(metrics.label_gauges.len(), 2);
    }

    #[test]
    fn convert_var_does_not_have_ups_prefix() {
        let var_name = "input.voltage";
        let expected_metric_name = "ups_input_voltage";

        let metric_name = convert_var_name(var_name);

        assert_eq!(metric_name, expected_metric_name);
    }

    #[test]
    fn convert_var_already_has_ups_prefix() {
        let var_name = "ups.load";
        let expected_metric_name = "ups_load";

        let metric_name = convert_var_name(var_name);

        assert_eq!(metric_name, expected_metric_name);
    }

    #[test]
    fn convert_var_multiple_dots() {
        let var_name = "input.voltage.nominal";
        let expected_metric_name = "ups_input_voltage_nominal";

        let metric_name = convert_var_name(var_name);

        assert_eq!(metric_name, expected_metric_name);
    }

    #[test]
    fn convert_var_empty() {
        let var_name = "";
        let expected_metric_name = "ups_";

        let metric_name = convert_var_name(var_name);

        assert_eq!(metric_name, expected_metric_name);
    }
}
