# ---- Builder Stage ----
FROM rust:slim AS builder

WORKDIR /root/build

# Copy source code
COPY . .

# Install system dependencies required for building
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Build the application in release mode
RUN cargo build --release

# ---- Runtime Stage ----
FROM debian:trixie-slim AS runtime

# Install only essential runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user for security best practices
RUN useradd --uid 1000 --create-home --shell /bin/bash trader

# Set working directory
WORKDIR /home/trader

# Copy the built binary from the builder stage
COPY --from=builder /root/build/target/release/Trader /home/trader/Trader

# Ensure the config directory exists (may be populated at runtime or via volume)
RUN mkdir -p /home/trader/config

# Change ownership to non-root user
RUN chown -R trader:trader /home/trader

# Switch to non-root user
USER trader

# Set default log level (can be overridden at runtime)
ENV RUST_LOG=info

# Run the binary
CMD ["/home/trader/Trader"]