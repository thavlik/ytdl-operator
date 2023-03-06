#!/bin/bash
set -euo pipefail

# Returns the unmasked public IP address.
get_ip() {
    curl -s ${IP_SERVICE:-"https://api.ipify.org"}
}

# Get the current IP address before the VPN is connected.
CURRENT_IP=$(get_ip)
echo "Unmasked public IP is $CURRENT_IP"

# Save the unmasked IP address to a file so that it can be accessed by other containers.
# The executor will know the VPN is connected when its probed IP is different from this value.
echo $CURRENT_IP > /shared/ip

# Run the Gluetun entrypoint script as normal.
/gluetun-entrypoint $@