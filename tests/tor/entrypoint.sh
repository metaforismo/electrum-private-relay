#!/bin/sh
set -eu

mode="${1:-}"
install -d -o debian-tor -g debian-tor -m 0700 /var/lib/tor/data

case "$mode" in
    client)
        cat >/tmp/torrc <<'EOF'
User debian-tor
DataDirectory /var/lib/tor/data
ClientOnly 1
SocksPort 0.0.0.0:9050
SocksPolicy accept *
Log notice stdout
EOF
        ;;
    service)
        relay_port="${2:?service mode requires the host relay port}"
        relay_host="${EPR_TOR_RELAY_HOST:-host.docker.internal}"
        install -d -o debian-tor -g debian-tor -m 0700 /var/lib/tor/onion
        cat >/tmp/torrc <<EOF
User debian-tor
DataDirectory /var/lib/tor/data
SocksPort 0
HiddenServiceDir /var/lib/tor/onion
HiddenServiceVersion 3
HiddenServicePort 50001 ${relay_host}:${relay_port}
Log notice stdout
EOF
        ;;
    *)
        echo "usage: epr-tor-smoke client | service HOST_RELAY_PORT" >&2
        exit 64
        ;;
esac

exec tor -f /tmp/torrc
