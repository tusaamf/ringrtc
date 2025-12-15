# --- Stage 1: Patch GN & Ninja cho ARM64 ---
FROM ubuntu:20.04 AS build-tools

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /tmp

RUN apt-get update && \
    apt-get install -y curl unzip git python3

# Tải GN bản ARM64
RUN ARCH=$(uname -m) && \
    if [ "$ARCH" = "aarch64" ]; then \
        curl -L -o gn.zip https://chrome-infra-packages.appspot.com/dl/gn/gn/linux-arm64/+/latest; \
    else \
        curl -L -o gn.zip https://chrome-infra-packages.appspot.com/dl/gn/gn/linux-amd64/+/latest; \
    fi && \
    unzip gn.zip && chmod +x gn && mv gn /usr/local/bin/gn

# Setup Depot Tools giả lập
WORKDIR /deps
RUN git clone https://chromium.googlesource.com/chromium/tools/depot_tools.git
RUN rm -f /deps/depot_tools/gn && ln -s /usr/local/bin/gn /deps/depot_tools/gn
RUN touch /deps/depot_tools/.cipd_bin_tools_initialized
RUN echo "python3" > /deps/depot_tools/python3_bin_reldir.txt

# --- Stage 2: Android Env ---
FROM ubuntu:20.04 AS android-env
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y openjdk-17-jdk wget unzip
ENV ANDROID_HOME=/root/.android
ENV ANDROID_NDK_VERSION=25.2.9519653
ENV ANDROID_NDK_HOME=${ANDROID_HOME}/ndk/${ANDROID_NDK_VERSION}
ENV PATH=${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${ANDROID_NDK_HOME}:${PATH}
RUN mkdir -p ${ANDROID_HOME}/cmdline-tools && cd ${ANDROID_HOME}/cmdline-tools && \
    wget "https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip" -O cmd.zip && \
    unzip cmd.zip && mv cmdline-tools latest && rm cmd.zip
RUN yes | sdkmanager --licenses && \
    sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;${ANDROID_NDK_VERSION}"

# --- Stage 3: Rust (FIXED) ---
FROM ubuntu:20.04 AS rustup
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y curl
# Cài đặt Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.76.0
# [QUAN TRỌNG] Cài đặt Targets ngay tại stage này để đảm bảo toolchain được tải về
ENV PATH=/root/.cargo/bin:$PATH
RUN rustup target add \
    armv7-linux-androideabi \
    aarch64-linux-android \
    i686-linux-android \
    x86_64-linux-android

# --- Stage 4: Final Image ---
FROM ubuntu:20.04 AS final
ENV DEBIAN_FRONTEND=noninteractive

# 1. Cấu hình apt sources và kiến trúc (Fix lỗi 404 & thứ tự lệnh)
RUN echo "deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports focal main restricted universe multiverse" > /etc/apt/sources.list && \
    echo "deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports focal-updates main restricted universe multiverse" >> /etc/apt/sources.list && \
    echo "deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports focal-security main restricted universe multiverse" >> /etc/apt/sources.list && \
    echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu focal main restricted universe multiverse" >> /etc/apt/sources.list && \
    echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu focal-updates main restricted universe multiverse" >> /etc/apt/sources.list && \
    echo "deb [arch=amd64] http://security.ubuntu.com/ubuntu focal-security main restricted universe multiverse" >> /etc/apt/sources.list && \
    dpkg --add-architecture amd64 && \
    apt-get update && \
    apt-get install -y software-properties-common

# 2. Cài Python 3.9 (Fix lỗi functools.cache) và thư viện x86 (Fix lỗi Clang)
RUN add-apt-repository ppa:deadsnakes/ppa && \
    apt-get update && \
    apt-get install -y \
    python3.9 \
    python3.9-dev \
    python3.9-venv \
    python3.9-distutils \
    libc6:amd64 \
    libstdc++6:amd64 \
    zlib1g:amd64 \
    lib32gcc1 \
    lib32stdc++6 \
    lib32z1

# 3. Ép dùng Python 3.9
RUN update-alternatives --install /usr/bin/python3 python3 /usr/bin/python3.9 1 && \
    ln -sf /usr/bin/python3.9 /usr/bin/python && \
    ln -sf /usr/bin/python3.9 /usr/bin/python3

# 4. Cài tool native
RUN apt-get install -y \
    protobuf-compiler make libglib2.0-dev openjdk-17-jdk build-essential \
    git curl unzip \
    ninja-build pkg-config

# Cài CMake Native
RUN ARCH=$(uname -m) && \
    if [ "$ARCH" = "aarch64" ]; then \
      CMAKE_URL="https://github.com/Kitware/CMake/releases/download/v3.22.1/cmake-3.22.1-linux-aarch64.tar.gz"; \
    else \
      CMAKE_URL="https://github.com/Kitware/CMake/releases/download/v3.22.1/cmake-3.22.1-linux-x86_64.tar.gz"; \
    fi && \
    curl -L ${CMAKE_URL} | tar --strip-components=1 -xz -C /usr/local

# Copy Depot Tools
ENV PATH=/deps/depot_tools:${PATH}
COPY --from=build-tools /deps/depot_tools /deps/depot_tools
COPY --from=build-tools /usr/local/bin/gn /usr/local/bin/gn
ENV DEPOT_TOOLS_UPDATE=0
ENV VPYTHON_BYPASS="manually managed python not supported by chrome operations"
RUN ln -sf /usr/bin/ninja /deps/depot_tools/ninja

# Copy Android SDK
ENV ANDROID_HOME=/deps/.android
ENV ANDROID_NDK_VERSION=25.2.9519653
ENV ANDROID_NDK_HOME=${ANDROID_HOME}/ndk/${ANDROID_NDK_VERSION}
ENV PATH=${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${ANDROID_NDK_HOME}:${PATH}
COPY --from=android-env /root/.android ${ANDROID_HOME}

# [FIXED] Copy Rust & Targets đúng cách
ENV RUSTUP_HOME=/deps/.rustup
ENV CARGO_HOME=/deps/.cargo
ENV PATH=$CARGO_HOME/bin:$PATH

# Copy CẢ HAI thư mục từ stage rustup
COPY --from=rustup /root/.rustup $RUSTUP_HOME
COPY --from=rustup /root/.cargo $CARGO_HOME

VOLUME [ "/build/ringrtc" ]
WORKDIR /build/ringrtc

CMD [ "tail", "-f", "/dev/null" ]