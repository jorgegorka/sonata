FROM node:22-bullseye-slim AS node
FROM ubuntu:22.04
ARG DEBIAN_FRONTEND=noninteractive
ARG RUST_VERSION=1.100.0
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates curl wget file git pkg-config patchelf \
    libwebkit2gtk-4.1-dev libgtk-3-dev libssl-dev libayatana-appindicator3-dev \
    librsvg2-dev libxdo-dev libfuse2 squashfs-tools xdg-utils python3 \
    xvfb xauth dbus-x11 openbox webkit2gtk-driver gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/ /usr/local/
ENV CARGO_HOME=/opt/cargo RUSTUP_HOME=/opt/rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION"
RUN useradd --create-home --uid 1000 sonata && mkdir -p /work && chown -R sonata:sonata /work /opt/cargo /opt/rustup
COPY linux-container.sh /usr/local/bin/sonata-validate
ENV PATH="/opt/cargo/bin:${PATH}"
ENV CARGO_BUILD_JOBS=2
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
ENV SONATA_CI_CONTAINER=1
WORKDIR /work
ENTRYPOINT ["bash", "/usr/local/bin/sonata-validate"]
