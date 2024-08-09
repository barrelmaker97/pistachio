use std::{env, thread, time};

use rups::blocking::Connection;
use rups::{ConfigBuilder};
use std::convert::TryInto;

fn main() -> rups::Result<()> {
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

    let poll_rate = 5;

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
