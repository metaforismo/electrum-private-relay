# Tor Deployment

The secure default is a loopback application listener published through a v3
onion service with client authorization. Do not bind the proxy directly to a
public interface merely to make it remote.

## Service side

Run the proxy on its default `127.0.0.1:50003` listener. A minimal onion-service
mapping in `torrc` is:

```text
HiddenServiceDir /var/lib/tor/electrum-private-relay/
HiddenServicePort 50001 127.0.0.1:50003
```

Restart Tor and read the generated `hostname` file from the hidden-service
directory. Treat every other file in that directory as secret key material.

An onion address alone is not authentication. Configure v3 client authorization
by placing an authorized client's public key in the onion service's
`authorized_clients/` directory, then restart Tor. Follow the current Tor Project
guide rather than copying credentials or key material into this repository:

- <https://community.torproject.org/onion-services/setup/>
- <https://community.torproject.org/onion-services/advanced/client-auth/>

## Wallet side

The wallet must be able to reach the onion endpoint through Tor and present the
client-authorization credential. Wallet support varies; test the exact desktop
or mobile wallet and version on testnet before relying on it.

## Broadcast side

`socks-electrum` uses a local SOCKS5 proxy such as Tor at `127.0.0.1:9050`.
Hostname resolution is delegated through SOCKS5, so an onion relay name is not
resolved by the local DNS resolver.

Tor protects the network path to the configured destination. It does not make a
normal Electrum broadcaster into a direct-to-miner relay, does not prevent
transaction fingerprinting, and does not neutralize identity embedded in a
provider account or client code.

## Operational checklist

- Keep the application listener on loopback.
- Enable v3 onion client authorization.
- Protect and back up the onion service key directory with restrictive file
  permissions.
- Use a self-hosted query upstream where possible.
- Verify the relay endpoint independently before configuration.
- Confirm that failure returns an error and never reaches the query upstream.
- Keep Tor and the operating system patched.
- Never place onion keys, client credentials, raw transactions, or `.env` files
  in Git.
