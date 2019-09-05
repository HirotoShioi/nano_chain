# select build image
FROM rust:1.37.0 as build

RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install tmux -y

# create a new empty shell project
RUN USER=root cargo new --bin nano_chain
WORKDIR /nano_chain

# copy over your manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./config/ ./config/
COPY ./scripts/ scripts/

RUN cargo clean
# this build step will cache your dependencies
RUN cargo build --release
RUN rm src/*.rs

# copy your source tree
COPY ./src ./src

# build for release
RUN rm ./target/release/deps/nano_chain*
RUN cargo build --release

CMD ["bash"]
