FROM rust:1.73-buster

# Switch to nightly:
RUN rustup default nightly

# Install targets:
RUN rustup target add aarch64-apple-darwin && \
    rustup target add x86_64-unknown-linux-gnu && \
    rustup target add x86_64-pc-windows-gnu

# Update APT:
RUN apt-get update
RUN apt-get install -y\
    mingw-w64\
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
    lld

# RUN cargo install xwin
# RUN xwin --accept-license splat --output /opt/xwin

# Copy build code:
WORKDIR /build

ENTRYPOINT [ "/bin/bash", "./install/build.sh" ]
