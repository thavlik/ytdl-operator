use std::fs;
use std::time::{Duration, SystemTime};

use crate::Error;

const READY_FILE_PATH: &str = "/shared/ready";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Wait a short while for the ready file to appear.
/// Returns an error if the file does not appear within the timeout.
pub async fn wait_for_vpn() -> Result<(), Error> {
    let start = SystemTime::now();
    loop {
        match fs::metadata(READY_FILE_PATH) {
            Ok(_) => return Ok(()),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    if start.elapsed()? > TIMEOUT {
                        // Timed out waiting for VPN to connect.
                        return Err(Error::ReadyFileNotFound);
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                return Err(e.into());
            }
        }
    }
}
