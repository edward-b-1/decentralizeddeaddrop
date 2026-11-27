# deaddropd: the Decentralized Dead Drop server, as a small container.
#
#   docker build -t deaddropd .          (from the top of the repository)
#
# Everything the server keeps (its database, its TLS key and certificate) is in /data.
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release --locked -p deaddropd \
 && mkdir -p /out/data \
 && cp target/release/deaddropd /out/deaddropd

# No shell, no package manager, not root.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /out/deaddropd /deaddropd
COPY --from=build --chown=65532:65532 /out/data /data
VOLUME /data
EXPOSE 7443
ENTRYPOINT ["/deaddropd"]
CMD ["serve", "--db", "/data/deaddrop.db", "--listen", "0.0.0.0:7443"]
