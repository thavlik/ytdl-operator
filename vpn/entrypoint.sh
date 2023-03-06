#!/bin/bash
set -euo pipefail

get_ip() {
    echo $(curl -s https://api.ipify.org)
}

CURRENT_IP=$(get_ip)
echo "Unmasked public IP is $CURRENT_IP"

# Save the unmasked IP address to a file so that it can be accessed by other containers.
# The executor will know the VPN is connected when its probed IP is different.
echo $CURRENT_IP > /shared/ip

/gluetun-entrypoint $@