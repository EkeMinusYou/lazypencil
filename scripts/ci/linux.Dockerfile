FROM rust:1.94.0-bookworm@sha256:365468470075493dc4583f47387001854321c5a8583ea9604b297e67f01c5a4f

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
       clang libclang-dev cmake ninja-build pkg-config python3 \
       libfontconfig1-dev libfreetype6-dev libwayland-dev libx11-dev \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt --toolchain 1.94.0

ENV CARGO_HOME=/work/cargo \
    CARGO_TARGET_DIR=/work/target \
    CARGO_INCREMENTAL=0 \
    CARGO_BUILD_JOBS=1
WORKDIR /work
