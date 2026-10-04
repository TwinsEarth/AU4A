# AU4A 运行时镜像：只装 au4a-node 二进制，默认起一个只读观察面板。
# 人类唯一入口是 HTTP GET；Agent 侧全部在进程内自主运行（见 docs/OBSERVER.md）。
# syntax=docker/dockerfile:1

FROM rust:1.88-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock VERSION ./
COPY crates ./crates
RUN cargo build --release --locked -p au4a-node

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 au4a
COPY --from=build /src/target/release/au4a-node /usr/local/bin/au4a-node
COPY --from=build /src/VERSION /VERSION
USER au4a
EXPOSE 8787
ENTRYPOINT ["au4a-node"]
# 默认：8 个 Agent 自主跑，并对外提供只读观察面板（GET /、/api/progress、/api/results、/api/revenue）
CMD ["run", "--agents", "8", "--observe", "0.0.0.0:8787"]
