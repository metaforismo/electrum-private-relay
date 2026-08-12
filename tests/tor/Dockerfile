FROM debian:13.2-slim@sha256:4bcb9db66237237d03b55b969271728dd3d955eaaa254b9db8a3db94550b1885

ARG TOR_PACKAGE_VERSION=0.4.9.11-0+deb13u1

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        "tor=${TOR_PACKAGE_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/epr-tor-smoke

RUN chmod 0555 /usr/local/bin/epr-tor-smoke

ENTRYPOINT ["/usr/local/bin/epr-tor-smoke"]
