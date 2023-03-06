#!/bin/bash
set -euo pipefail

get_ip() {
    echo $(curl -s https://api.ipify.org)
}

CURRENT_IP=$(get_ip)
echo "Unmasked public IP is $CURRENT_IP"
echo "Logging into NordVPN as '$NORDVPN_USERNAME'"

# Login using credentials from environment variables.
nordvpn login \
    --username $NORDVPN_USERNAME \
    --password $NORDVPN_PASSWORD

# Connect to a random NordVPN server.
nordvpn connect $@

# Enable the killswitch, which will block all traffic if the VPN connection is lost.
nordvpn set killswitch on

NEW_IP=$(get_ip)
if [ "$NEW_IP" == "$CURRENT_IP" ]; then
    echo "Public IP address is unchanged after connecting to NordVPN. This is probably a configuration error."
    exit 1
fi
echo "Masked public IP is $NEW_IP"

# Signify that the VPN connection is ready.
touch /shared/ready
echo "Signaled ready."

# Do nothing forever.
tail -f /dev/null
