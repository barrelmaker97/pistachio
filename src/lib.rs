#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use clap::Parser;
use log::{debug, error, info, warn};
use metrics::{describe_gauge, gauge};
use rups::tokio::Connection;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU64;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time;

/// Default configuration options
const DEFAULT_UPS_NAME: &str = "ups";
const DEFAULT_UPS_HOST: &str = "127.0.0.1";
const DEFAULT_UPS_PORT: u16 = 3493;
const DEFAULT_BIND_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
const DEFAULT_BIND_PORT: u16 = 9120;
const DEFAULT_POLL_RATE: NonZeroU64 = NonZeroU64::MIN.saturating_add(9);

/// An array of possible UPS system states
const UPS_STATES: &[&str] = &[
    "OL", "OB", "LB", "RB", "CHRG", "DISCHRG", "ALARM", "OVER", "TRIM", "BOOST", "BYPASS", "OFF", "CAL", "TEST", "FSD",
];

/// An array of possible UPS beeper states
const BEEPER_STATES: &[&str] = &["enabled", "disabled", "muted"];

/// UPS status variables and their associated label state sets
const STATUS_VARS: &[(&str, &str, &[&str])] =
    &[("ups.status", "UPS Status Code", UPS_STATES), ("ups.beeper.status", "Beeper Status", BEEPER_STATES)];

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
    #[arg(long, env, default_value_t = DEFAULT_POLL_RATE)]
    pub poll_rate: NonZeroU64,
}

impl Args {
    /// Returns the socket address on which the Prometheus exporter will listen.
    #[must_use]
    pub const fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.bind_port)
    }

    /// Returns the polling interval as a [`Duration`].
    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_rate.get())
    }

    /// Creates a connection for communicating with the NUT server.
    ///
    /// # Errors
    ///
    /// An error will be returned if the UPS host and port cannot be used to create a valid
    /// [`rups::Host`] or if the connection fails.
    pub async fn create_connection(&self) -> Result<Connection, rups::ClientError> {
        let rups_host = rups::Host::try_from((self.ups_host.clone(), self.ups_port))?;
        let rups_config = rups::ConfigBuilder::new().with_host(rups_host).build();
        Connection::new(&rups_config).await
    }

    /// Queries the NUT server for all numeric UPS variables and their descriptions.
    ///
    /// Non-numeric variables are filtered out before requesting descriptions, since basic
    /// Prometheus gauges only represent floating-point values.
    ///
    /// # Errors
    ///
    /// An error will be returned if the list of variables or their descriptions cannot be
    /// retrieved from the NUT server, such as if connection to the server is lost.
    pub async fn get_ups_vars(&self, conn: &mut Connection) -> Result<HashMap<String, String>, rups::ClientError> {
        let ups_name = self.ups_name.as_str();
        let available_vars = conn.list_vars(ups_name).await?;
        let mut ups_vars = HashMap::new();
        for var in available_vars.iter().filter(|var| var.value().parse::<f64>().is_ok()) {
            let description = conn.get_var_description(ups_name, var.name()).await?;
            ups_vars.insert(var.name().to_owned(), description);
        }
        Ok(ups_vars)
    }
}

/// A Prometheus gauge that tracks a set of status labels.
#[derive(Debug)]
struct LabelGauge {
    name: String,
    states: &'static [&'static str],
}

impl LabelGauge {
    fn register(var_name: &str, description: &'static str, states: &'static [&'static str]) -> Self {
        let name = convert_var_name(var_name);
        describe_gauge!(name.clone(), description);
        debug!("Gauge {name} has been registered for var {var_name}");
        Self { name, states }
    }

    fn update(&self, value: &str) {
        for &state in self.states {
            let is_active = value.split_whitespace().any(|token| token == state);
            gauge!(self.name.clone(), "status" => state).set(u8::from(is_active));
        }
    }

    fn reset(&self) {
        for &state in self.states {
            gauge!(self.name.clone(), "status" => state).set(0.0);
        }
    }
}

/// A collection of all registered metrics, both labelled and unlabelled.
#[derive(Debug)]
pub struct Metrics {
    basic_gauges: HashMap<String, String>,
    label_gauges: HashMap<String, LabelGauge>,
}

impl Metrics {
    /// Creates a [`Metrics`] instance from a map of numeric variable names and descriptions.
    #[must_use]
    pub fn build(ups_vars: &HashMap<String, String>) -> Self {
        let basic_gauges = ups_vars
            .iter()
            .map(|(name, desc)| {
                let gauge_name = convert_var_name(name);
                describe_gauge!(gauge_name.clone(), desc.clone());
                debug!("Gauge {gauge_name} has been registered for var {name}");
                (name.clone(), gauge_name)
            })
            .collect();

        // Registers label gauges for UPS variables that represent a set of potential status.
        // This currently only includes overall UPS status and beeper status.
        let label_gauges = STATUS_VARS
            .iter()
            .map(|&(name, desc, states)| (name.to_owned(), LabelGauge::register(name, desc, states)))
            .collect();

        Self { basic_gauges, label_gauges }
    }

    /// Returns the number of all gauges registered.
    #[must_use]
    pub fn count(&self) -> usize {
        self.basic_gauges.len() + self.label_gauges.len()
    }

    /// Takes a slice of variables and updates all associated gauges. For label gauges, each
    /// label of the gauge is updated to reflect all current states present in the value from
    /// the UPS.
    pub fn update(&self, var_list: &[rups::Variable]) {
        for var in var_list {
            if let Some(gauge_name) = self.basic_gauges.get(var.name()) {
                // Update basic gauges
                if let Ok(value) = var.value().parse::<f64>() {
                    gauge!(gauge_name.clone()).set(value);
                } else {
                    warn!("Failed to update gauge {gauge_name} because the value was not a float");
                }
            } else if let Some(label_gauge) = self.label_gauges.get(var.name()) {
                // Update label gauges
                label_gauge.update(&var.value());
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
        for label_gauge in self.label_gauges.values() {
            label_gauge.reset();
        }
    }
}

/// Polls the NUT server at the configured interval and updates Prometheus metrics, resetting
/// gauges on error and recreating the connection on I/O failures until a shutdown signal is
/// received.
pub async fn monitor_ups(mut conn: Connection, args: Args, metrics: Metrics, shutdown_rx: oneshot::Receiver<()>) {
    let mut is_failing = false;
    let mut interval = time::interval(args.poll_interval());

    tokio::select! {
        () = async {
            loop {
                interval.tick().await;
                debug!("Polling UPS...");
                match conn.list_vars(args.ups_name.as_str()).await {
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

                        // IO errors can cause the connection to continue failing,
                        // even once the UPS is back online. Recreating the connection
                        // resolves the issue
                        if let rups::ClientError::Io(_) = err {
                            debug!("Attempting to recreate connection due to IO error...");
                            // Creating new connection
                            match args.create_connection().await {
                                Ok(new_conn) => {
                                    conn = new_conn;
                                    debug!("Connection recreated successfully");
                                }
                                Err(err) => {
                                    error!("Failed to recreate connection: {err}");
                                }
                            }
                        }
                    }
                }
            }
        } => {},
        _ = shutdown_rx => {
            info!("Attempting graceful shutdown");
            if let Err(e) = conn.close().await {
                error!("Failed to close connection during shutdown: {e}");
            }
        }
    }
}

fn convert_var_name(name: &str) -> String {
    let mut gauge_name = name.replace('.', "_");
    if !gauge_name.starts_with("ups") {
        gauge_name.insert_str(0, "ups_");
    }
    gauge_name
}

#[cfg(test)]
mod tests;
