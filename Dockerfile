# ---------- Giai đoạn 1: biên dịch server Rust ----------
FROM rust:1-slim-bookworm AS build
WORKDIR /src/server

# Build trước phần thư viện với main.rs giả để Docker cache lại,
# lần sau chỉ đổi code game thì không phải tải/biên dịch lại toàn bộ crate.
COPY server/Cargo.toml server/Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY server/src ./src
# touch để cargo nhận ra main.rs thật mới hơn bản giả
RUN touch src/main.rs && cargo build --release

# ---------- Giai đoạn 2: image chạy, gọn nhẹ ----------
FROM debian:bookworm-slim
WORKDIR /app

RUN useradd --system xito
COPY --from=build /src/server/target/release/xito-server /app/xito-server
COPY client /app/client

ENV CLIENT_DIR=/app/client \
    PORT=3000
EXPOSE 3000
USER xito

CMD ["/app/xito-server"]
