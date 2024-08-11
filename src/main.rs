use std::{env, thread, time};
use std::convert::TryInto;
use std::net::SocketAddr;

use rups::blocking::Connection;
use rups::{ConfigBuilder};

use prometheus_exporter::prometheus::register_gauge;

use env_logger::{Builder, Env};
use log::info;

fn main() -> rups::Result<()> {
    // Setup exporter address
    let addr_raw = "0.0.0.0:9184";
    let addr: SocketAddr = addr_raw.parse().expect("Cannot parse listen address");

    // Create metric and start exporter
    let metric = register_gauge!("test_gauge", "test description").expect("Cannot create gauge");
    metric.set(42.0);
    prometheus_exporter::start(addr).expect("Cannot start exporter");

    Builder::from_env(Env::default().default_filter_or("info")).init();
    info!("Exporter started!");

    let poll_rate = 10;
    let host = env::var("UPS_HOST").unwrap_or_else(|_| "localhost".into());
    let port = env::var("UPS_PORT")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(3493);

    let config = ConfigBuilder::new()
        .with_host((host, port).try_into().unwrap_or_default())
        .with_debug(false) // Turn this on for debugging network chatter
        .build();

    let mut conn = Connection::new(&config)?;

    // Print a list of all UPS devices
    let mut counter = 0;
    loop {
        if counter % poll_rate == 0 {
            println!("Connected UPS devices:");
            for (name, description) in conn.list_ups()? {
                println!("\t- Name: {}", name);
                println!("\t  Description: {}", description);

                // List UPS variables (key = val)
                println!("\t  Variables:");
                for var in conn.list_vars(&name)? {
                    println!("\t\t- {}", var);
                }
            }
        }
        counter += 1;
        thread::sleep(time::Duration::from_secs(1));
    }
}
