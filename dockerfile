FROM rust:1.37

RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install tmux -y

WORKDIR /usr/src/nano_chain
COPY . .
RUN cargo build --release

CMD ["bash"]