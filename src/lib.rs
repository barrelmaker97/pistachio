#![deny(missing_docs)]

//! # pistachio
//!
//! Pistachio is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

use clap::Parser;
use log::{debug, error, info, warn};
use metrics::{Gauge, describe_gauge, gauge};
use rups::tokio::Connection;
use std::collections::HashMap;
use std::fmt;
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
}

/// A client that manages the connection and polling lifecycle for a single UPS on a NUT server.
pub struct NutClient {
    ups_name: String,
    config: rups::Config,
    conn: Connection,
    is_failing: bool,
}

impl fmt::Debug for NutClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NutClient")
            .field("ups_name", &self.ups_name)
            .field("config", &self.config)
            .field("is_failing", &self.is_failing)
            .finish_non_exhaustive()
    }
}

impl NutClient {
    /// Connects to the NUT server specified in [`Args`].
    ///
    /// # Errors
    ///
    /// Returns an error if the UPS host and port cannot form a valid [`rups::Host`] or if the
    /// initial TCP connection fails.
    pub async fn connect(args: &Args) -> Result<Self, rups::ClientError> {
        let rups_host = rups::Host::try_from((args.ups_host.clone(), args.ups_port))?;
        let config = rups::ConfigBuilder::new().with_host(rups_host).build();
        let conn = Connection::new(&config).await?;
        Ok(Self { ups_name: args.ups_name.clone(), config, conn, is_failing: false })
    }

    /// Queries the NUT server for all numeric UPS variables and their descriptions.
    ///
    /// Non-numeric variables are filtered out before requesting descriptions, since basic
    /// Prometheus gauges only represent floating-point values.
    ///
    /// # Errors
    ///
    /// Returns an error if the variable list or descriptions cannot be retrieved from the server.
    pub async fn discover_numeric_vars(&mut self) -> Result<HashMap<String, String>, rups::ClientError> {
        let available_vars = self.conn.list_vars(&self.ups_name).await?;
        let mut ups_vars = HashMap::new();
        for var in available_vars.iter().filter(|var| var.value().parse::<f64>().is_ok()) {
            let description = self.conn.get_var_description(&self.ups_name, var.name()).await?;
            ups_vars.insert(var.name().to_owned(), description);
        }
        Ok(ups_vars)
    }

    /// Polls all current variables for the UPS, automatically attempting to recreate the
    /// connection when an I/O error occurs.
    ///
    /// # Errors
    ///
    /// Returns an error if polling the NUT server fails.
    pub async fn poll_vars(&mut self) -> Result<Vec<rups::Variable>, rups::ClientError> {
        debug!("Polling UPS...");
        match self.conn.list_vars(&self.ups_name).await {
            Ok(var_list) => {
                if self.is_failing {
                    info!("Connection with the UPS has been reestablished");
                    self.is_failing = false;
                }
                Ok(var_list)
            }
            Err(err) => {
                warn!("Failed to connect to the UPS: {err}");
                self.is_failing = true;

                // IO errors can cause the connection to continue failing,
                // even once the UPS is back online. Recreating the connection
                // resolves the issue.
                if let rups::ClientError::Io(_) = err {
                    debug!("Attempting to recreate connection due to IO error...");
                    match Connection::new(&self.config).await {
                        Ok(new_conn) => {
                            self.conn = new_conn;
                            debug!("Connection recreated successfully");
                        }
                        Err(reconnect_err) => {
                            error!("Failed to recreate connection: {reconnect_err}");
                        }
                    }
                }
                Err(err)
            }
        }
    }

    /// Gracefully closes the connection to the NUT server.
    pub async fn close(self) {
        info!("Attempting graceful shutdown");
        if let Err(e) = self.conn.close().await {
            error!("Failed to close connection during shutdown: {e}");
        }
    }
}

/// A Prometheus gauge that tracks a set of status labels.
#[derive(Debug)]
struct LabelGauge {
    states: Vec<(&'static str, Gauge)>,
}

impl LabelGauge {
    fn register(var_name: &str, description: &'static str, states: &'static [&'static str]) -> Self {
        let gauge_name = convert_var_name(var_name);
        describe_gauge!(gauge_name.clone(), description);
        debug!("Gauge {gauge_name} has been registered for var {var_name}");
        let states = states.iter().map(|&state| (state, gauge!(gauge_name.clone(), "status" => state))).collect();
        Self { states }
    }

    fn update(&self, value: &str) {
        for &(state, ref gauge) in &self.states {
            let is_active = value.split_whitespace().any(|token| token == state);
            gauge.set(u8::from(is_active));
        }
    }

    fn reset(&self) {
        for (_, gauge) in &self.states {
            gauge.set(0.0);
        }
    }
}

/// A collection of all registered metrics, both labelled and unlabelled.
#[derive(Debug)]
pub struct Metrics {
    basic_gauges: HashMap<String, Gauge>,
    label_gauges: HashMap<&'static str, LabelGauge>,
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
                (name.clone(), gauge!(gauge_name))
            })
            .collect();

        // Registers label gauges for UPS variables that represent a set of potential status.
        // This currently only includes overall UPS status and beeper status.
        let label_gauges =
            STATUS_VARS.iter().map(|&(name, desc, states)| (name, LabelGauge::register(name, desc, states))).collect();

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
            if let Some(gauge) = self.basic_gauges.get(var.name()) {
                // Update basic gauges
                if let Ok(value) = var.value().parse::<f64>() {
                    gauge.set(value);
                } else {
                    warn!("Failed to update gauge for {} because the value was not a float", var.name());
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
        for gauge in self.basic_gauges.values() {
            gauge.set(0.0);
        }
        for label_gauge in self.label_gauges.values() {
            label_gauge.reset();
        }
    }
}

/// Polls the NUT server at the configured interval and updates Prometheus metrics, resetting
/// gauges on error until a shutdown signal is received.
pub async fn monitor_ups(
    mut client: NutClient,
    metrics: Metrics,
    poll_interval: Duration,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let mut interval = time::interval(poll_interval);

    tokio::select! {
        () = async {
            loop {
                interval.tick().await;
                if let Ok(var_list) = client.poll_vars().await {
                    metrics.update(&var_list);
                    debug!("Metrics updated");
                } else {
                    metrics.reset();
                    debug!("Reset gauges to zero because the UPS was unreachable");
                }
            }
        } => {},
        _ = shutdown_rx => {
            client.close().await;
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
