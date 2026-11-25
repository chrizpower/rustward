ARG RUST_IMAGE=default-rust
ARG DEBIAN_IMAGE=default-debian
FROM rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07 AS default-rust
FROM debian:13-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS default-debian
FROM docker.io/library/node:24.16.0-trixie-slim@sha256:45fbb3ca3b6c7e6646cd2889d0ac7bf314bb180036da792221fc2f48fe4d43fb AS node
FROM ${RUST_IMAGE} AS source
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm
WORKDIR /source
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY examples ./examples
COPY web ./web

FROM source AS project-checks
COPY .gitattributes Justfile ./
COPY acceptance ./acceptance
COPY scripts ./scripts
COPY tests ./tests
RUN useradd --create-home --uid 1000 checks && chown -R checks:checks /source
USER checks
ENV HOME=/home/checks
RUN rustup component add rustfmt clippy llvm-tools-preview && \
    cargo install --locked --version 1.59.0 just && \
    cargo install --locked --version 0.9.1 cargo-llvm-cov
RUN namei -l /source /usr/bin/env && just check

FROM source AS release-build
ARG RUSTWARD_SOURCE_REVISION
RUN cargo build --locked --release --bin rustward --bin rustward-exec

FROM ${DEBIAN_IMAGE} AS release-smoke
COPY --from=release-build /source/target/release/rustward /release/rustward
COPY --from=release-build /source/target/release/rustward-exec /release/rustward-exec
COPY examples/rustward.toml /example/rustward.toml
COPY examples/tasks /example/tasks
COPY examples/schedules /example/schedules
USER 65534:65534
RUN /release/rustward validate /example/rustward.toml && \
    version="$(/release/rustward --version)" && \
    launcher_version="$(/release/rustward-exec --version)" && \
    test "${version#rustward }" = "${launcher_version#rustward-exec }" && \
    printf '%s\n' "${version#rustward }" > /tmp/rustward-version && \
    test "$(/release/rustward-exec /tmp /tmp /usr/bin/printf '%s' 'launcher works')" = 'launcher works'

FROM scratch AS release-files
COPY --from=release-smoke /release/ /
COPY --from=release-smoke /tmp/rustward-version /rustward-version

FROM source AS preview-build
RUN cargo build --locked --release --features web-preview --bin rustward

FROM ${DEBIAN_IMAGE} AS web-preview
COPY --from=preview-build /source/target/release/rustward /usr/local/bin/rustward
USER 65534:65534
ENTRYPOINT ["rustward", "web-preview", "--container"]

FROM source AS recovery-test
RUN cargo test --locked --release --lib service::maintenance::tests

FROM recovery-test AS telemetry-test
RUN cargo test --locked --release --lib telemetry::tests && \
    cargo test --locked --release --lib service::web::tests && \
    cargo test --locked --release --lib service::tests

FROM telemetry-test AS build
RUN cargo build --locked --release --bin rustward --bin rustward-exec --example acceptance
COPY tests ./tests
RUN cargo test --locked --release --no-run --test systemd_host --test runtime_host --message-format=json > /test-artifacts.json
RUN mkdir /bundle && cp target/release/rustward target/release/rustward-exec target/release/examples/acceptance /bundle/ && \
    python3 -c 'import json,shutil; records=[json.loads(line) for line in open("/test-artifacts.json")]; [shutil.copyfile(record["executable"],"/bundle/"+record["target"]["name"]) for record in records if record.get("executable")];' && \
    chmod 755 /bundle/*

FROM ${DEBIAN_IMAGE} AS ssh-target
RUN apt-get update && apt-get install --yes --no-install-recommends openssh-server python3 && \
    rm -rf /var/lib/apt/lists/* && rm -f /etc/ssh/ssh_host_* && \
    useradd --uid 1000 --create-home --shell /bin/sh --password '' rustward-target
COPY acceptance/ssh_target.py /opt/ssh_target.py
COPY acceptance/fixtures/hold.py /opt/rustward_hold.py
COPY acceptance/fixtures/flood.py /opt/rustward_flood.py
CMD ["/usr/bin/python3", "-I", "-B", "/opt/ssh_target.py", "serve"]

FROM ${DEBIAN_IMAGE} AS pilot-tools
RUN apt-get update && apt-get install --yes --no-install-recommends \
      prometheus prometheus-alertmanager python3-aiosmtpd && \
    rm -rf /var/lib/apt/lists/*

FROM pilot-tools AS pilot-rehearsal
COPY --from=preview-build /source/target/release/rustward /usr/local/bin/rustward-preview
COPY acceptance /opt/rustward-acceptance-source
ENTRYPOINT ["python3", "-B", "/opt/rustward-acceptance-source/pilot_alerts.py", "/usr/local/bin/rustward-preview", "/results"]

FROM pilot-tools
ENV container=docker
RUN apt-get update && apt-get install --yes --no-install-recommends \
      systemd systemd-sysv dbus dbus-user-session libpam-systemd \
      ansible-core openssh-client python3 python3-prometheus-client coreutils libc-bin util-linux mount ca-certificates nginx openssl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --uid 1000 --create-home --shell /bin/bash rustward && \
    mkdir -p /var/lib/systemd/linger && touch /var/lib/systemd/linger/rustward && \
    systemctl --global enable dbus.socket && \
    systemctl --root=/ mask prometheus.service prometheus-alertmanager.service && \
    mkdir -p /etc/systemd/system/user@1000.service.d && \
    printf '[Service]\nNoNewPrivileges=yes\nCapabilityBoundingSet=\nAmbientCapabilities=\n' > /etc/systemd/system/user@1000.service.d/rustward-acceptance.conf && \
    truncate --size=0 /etc/machine-id
COPY --from=build /bundle /opt/rustward-acceptance-tools
COPY --from=preview-build /source/target/release/rustward /opt/rustward-acceptance-tools/rustward-preview
COPY acceptance /opt/rustward-acceptance-source
ARG SOURCE_REVISION=unspecified
RUN printf '%s\n' "$SOURCE_REVISION" > /opt/rustward-acceptance-source/source-revision
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
