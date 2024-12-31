FROM rust:latest

# Update APT:
RUN apt-get update &&\
    apt-get install -y\
    build-essential\
    git &&\
    apt-get clean autoclean && \
    apt-get autoremove --yes && \
    rm -rf /var/lib/{apt,dpkg,cache,log}/


# Install zig (for better cross-compilation!)
RUN cd /root && \
    wget https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz && \
    tar xf zig-linux-x86_64-0.13.0.tar.xz && \
    rm zig-linux-x86_64-0.13.0.tar.xz && \
    ./zig-linux-x86_64-0.13.0/zig --help
ENV PATH "/root/zig-linux-x86_64-0.13.0:$PATH"

# Switch to nightly:
RUN rustup default nightly

# Install cargo plugins:
RUN cargo install cargo-zigbuild

# Install targets:
RUN rustup target add aarch64-apple-darwin && \
    rustup target add x86_64-unknown-linux-gnu && \
    rustup target add x86_64-pc-windows-gnu

# Copy build code:
WORKDIR /build
COPY . .

# Inflate macOS sdk:
# you can always count on https://github.com/alexey-lysiuk/macos-sdk for future updates!
RUN mkdir -p /darwin && cd /darwin && tar xf /build/install/src/rust/MacOSX14.5.tar.xz
ENV SDKROOT /darwin/MacOSX14.5.sdk

ENTRYPOINT [ "/bin/bash" ]
