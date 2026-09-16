# 多阶段构建：docker build -t zbxpatrol .
# 运行示例：
#   docker run --rm -e ZBX_URL=https://zabbix.example.com -e ZBX_USER=admin \
#     -e ZBX_PASSWORD=xxx -v $PWD/reports:/app/reports zbxpatrol report --period day
#   docker run --rm -p 8787:8787 -e ... zbxpatrol serve --listen 0.0.0.0:8787

FROM rust:1.83-slim AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates
RUN cargo build --release

FROM gcr.io/distroless/cc
COPY --from=builder /src/target/release/zbxpatrol /usr/local/bin/zbxpatrol
WORKDIR /app
ENTRYPOINT ["zbxpatrol"]
