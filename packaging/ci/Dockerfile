ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}
COPY packaging/ci/bootstrap.sh /tmp/epoxy-bootstrap.sh
RUN bash /tmp/epoxy-bootstrap.sh && rm /tmp/epoxy-bootstrap.sh
ENV CARGO_HOME=/opt/cargo RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh && \
    sh /tmp/rustup.sh -y --profile minimal --default-toolchain 1.95.0 && \
    chmod -R a+rX /opt/cargo /opt/rustup && rm /tmp/rustup.sh
WORKDIR /build
