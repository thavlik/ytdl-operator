use std::{
    io,
    time::{Duration, SystemTime},
};
use tokio::{fs, time};
use ytdl_common::pod::{IP_FILE_PATH, IP_SERVICE};

use crate::Error;

/// Initialization timeout. The VPN must connect and
/// the public IP must change in this time frame
/// or the executor will bail.
const TIMEOUT: Duration = Duration::from_secs(12);

/// Waits for the VPN container to write the initial
/// public IP to a file then probes an external service
/// until the IP changes, signifying that the VPN is
/// connected and the pod's public IP is properly masked.
pub async fn wait_for_vpn() -> Result<(), Error> {
    // Get the unmasked IP address from the shared dir.
    let ip = wait_for_initial_ip().await?;
    println!("Unmasked public IP: {}", &ip);
    // Probe the public IP until it changes.
    println!("Waiting for public IP to change...");
    let ip = wait_for_ip_change(&ip).await?;
    println!("VPN connected. Masked public IP: {}", &ip);
    Ok(())
}

/// Wait a short while for the ready file to appear.
/// Returns an error if the file does not appear within
/// the timeout. This file is now created by an init
/// container so it should always exist by the time the
/// executor is started. This code is left here in case
/// the init paradigm is changed.
async fn wait_for_initial_ip() -> Result<String, Error> {
    let start = SystemTime::now();
    loop {
        // Try and read the IP file.
        match fs::read_to_string(IP_FILE_PATH).await {
            // File found, return the IP.
            Ok(ip) => return Ok(ip),
            Err(e) => match e.kind() {
                // Allow retries if the file was not found.
                io::ErrorKind::NotFound => {
                    if start.elapsed()? < TIMEOUT {
                        // Wait for a bit and try again.
                        time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    // Timed out waiting for VPN to connect.
                    return Err(Error::VPNError(
                        "timed out waiting for initial ip file".to_owned(),
                    ));
                }
                // Unknown error reading IP file, bail.
                _ => return Err(e.into()),
            },
        }
    }
}

/// Waits for the public IP address to change then returns
/// the new IP address.
async fn wait_for_ip_change(current: &str) -> Result<String, Error> {
    let start = SystemTime::now();
    loop {
        let ip = get_public_ip().await?;
        if ip != current {
            // Public IP address change detected.
            return Ok(ip);
        }
        if start.elapsed()? < TIMEOUT {
            // Wait a bit and probe the IP again.
            time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        return Err(Error::VPNError(
            "Public IP to change before deadline".to_owned(),
        ));
    }
}

/// Returns the current public IP address by querying
/// an external service (e.g. https://api.ipify.org).
/// This should be the same service used by the init
/// container to write the contents of /shared/ip
async fn get_public_ip() -> Result<String, Error> {
    Ok(reqwest::get(IP_SERVICE).await?.text().await?)
}
