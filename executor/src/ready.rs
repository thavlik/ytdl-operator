use std::time::{Duration, SystemTime};
use tokio::{time, fs};

use crate::Error;

const IP_FILE_PATH: &str = "/shared/ip";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Waits for the VPN container to write the initial
/// public IP to a file then probes an external service
/// until the IP changes, signifying that the VPN is
/// connected and the pod's public IP is properly masked.
pub async fn wait_for_vpn() -> Result<(), Error> {
    // Get the unmasked IP address from the shared dir.
    let ip = wait_for_initial_ip().await?;
    println!("Unmasked IP: {}", &ip);
    // Probe the public IP until it changes.
    println!("Waiting for public IP to change...");
    let ip = wait_for_ip_change(&ip).await?;
    println!("Masked IP: {}", &ip);
    Ok(())
}

/// Wait a short while for the ready file to appear.
/// Returns an error if the file does not appear within the timeout.
async fn wait_for_initial_ip() -> Result<String, Error> {
    let start = SystemTime::now();
    loop {
        match fs::metadata(IP_FILE_PATH).await {
            Ok(_) => return Ok(fs::read_to_string(IP_FILE_PATH).await?),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    if start.elapsed()? > TIMEOUT {
                        // Timed out waiting for VPN to connect.
                        return Err(Error::VPNSidecarFailure(
                            "timed out waiting for initial ip file".to_owned()));
                    }
                    time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                return Err(e.into());
            }
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
            return Ok(ip);
        }
        if start.elapsed()? > TIMEOUT {
            return Err(Error::VPNSidecarFailure(
                "timed out waiting for public ip to change".to_owned()));
        }
        time::sleep(Duration::from_secs(1)).await;
    }
}

/// Returns the current public IP address by querying
/// an external service (https://api.ipify.org). This
/// should be the same service used by the VPN container
/// to write the contents of initial /shared/ip file.
async fn get_public_ip() -> Result<String, Error> {
    Ok(reqwest::get("https://api.ipify.org").await?.text().await?)
}