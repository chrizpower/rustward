ARG RUST_IMAGE=rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07
ARG DEBIAN_IMAGE=debian:13-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
FROM ${RUST_IMAGE} AS build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY examples/acceptance ./examples/acceptance
RUN cargo build --locked --release --bin rustward-exec --example acceptance
COPY tests ./tests
RUN cargo test --locked --release --no-run --test systemd_host --test runtime_host --message-format=json > /test-artifacts.json
RUN mkdir /bundle && cp target/release/rustward-exec target/release/examples/acceptance /bundle/ && \
    python3 -c 'import json,shutil; records=[json.loads(line) for line in open("/test-artifacts.json")]; [shutil.copyfile(record["executable"],"/bundle/"+record["target"]["name"]) for record in records if record.get("executable")];' && \
    chmod 755 /bundle/*

FROM ${DEBIAN_IMAGE} AS ssh-target
RUN apt-get update && apt-get install --yes --no-install-recommends openssh-server python3 && \
    rm -rf /var/lib/apt/lists/* && rm -f /etc/ssh/ssh_host_* && \
    useradd --uid 1000 --create-home --shell /bin/sh --password '' rustward-target
COPY acceptance/ssh_target.py /opt/ssh_target.py
CMD ["/usr/bin/python3", "-I", "-B", "/opt/ssh_target.py", "serve"]

FROM ${DEBIAN_IMAGE}
ENV container=docker
RUN apt-get update && apt-get install --yes --no-install-recommends \
      systemd systemd-sysv dbus dbus-user-session libpam-systemd \
      ansible-core openssh-client python3 coreutils libc-bin util-linux ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --uid 1000 --create-home --shell /bin/bash rustward && \
    mkdir -p /var/lib/systemd/linger && touch /var/lib/systemd/linger/rustward && \
    systemctl --global enable dbus.socket && \
    mkdir -p /etc/systemd/system/user@1000.service.d && \
    printf '[Service]\nNoNewPrivileges=yes\nCapabilityBoundingSet=\nAmbientCapabilities=\n' > /etc/systemd/system/user@1000.service.d/rustward-acceptance.conf && \
    truncate --size=0 /etc/machine-id
COPY --from=build /bundle /opt/rustward-acceptance-tools
COPY acceptance /opt/rustward-acceptance-source
ARG SOURCE_REVISION=unspecified
RUN printf '%s\n' "$SOURCE_REVISION" > /opt/rustward-acceptance-source/source-revision
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
