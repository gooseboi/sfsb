# syntax = docker/dockerfile:1.6
FROM debian:bookworm-20231009-slim as rust_builder

# Install curl and deps
RUN apt-get update; \
	apt-get install -y --no-install-recommends \
		curl ca-certificates gcc libc6-dev pkg-config libssl-dev;

# Install rustup
# We don't really care what toolchain it installs, as we just use
# rust-toolchain.toml, but as far as I know there is no way to just install
# the toolchain in the file at this point
RUN curl --location --fail \
        "https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init" \
		--output /rustup-init; \
    chmod +x /rustup-init; \
	/rustup-init -y --no-modify-path --profile minimal --no-update-default-toolchain; \
	rm /rustup-init;

ENV PATH=${PATH}:/root/.cargo/bin
RUN rustup --version

RUN cargo install just

# Copy sources and build them
WORKDIR /app
COPY . .

RUN --mount=type=cache,target=/root/.rustup \
    --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
	--mount=type=cache,target=/app/target/build \
	--mount=type=cache,target=/app/target/deps \
	--mount=type=cache,target=/app/target/incremental \
	set -eux; \
	just build_release

FROM debian:bookworm-20231009-slim

WORKDIR /app
COPY --from=rust_builder /app/target/release/sfsb .

ENV SFSB_DATA_DIR="/data"

EXPOSE 3799

ENTRYPOINT ["/app/sfsb"]
