FROM rust:bullseye

# Update APT:
RUN apt-get update &&\
    apt-get install -y\
    llvm\
    libclang-dev\
    build-essential\
    git\
    libbz2-dev\
    libgflags-dev\
    libjemalloc-dev\
    libsnappy-dev\
    libtbb-dev\
    zlib1g-dev\
    lld &&\
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

ENTRYPOINT [ "/bin/bash" ]
