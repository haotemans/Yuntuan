# syntax=docker/dockerfile:1
# yuantuan 一体化镜像：内嵌前端，Node.js 支持管理员安装的夸克官方 CLI

FROM node:22-bookworm-slim AS node-runtime
FROM node-runtime AS frontend
WORKDIR /app
COPY webui-frontend/package*.json webui-frontend/
RUN cd webui-frontend && npm ci
COPY webui-frontend/ webui-frontend/
RUN cd webui-frontend && npm run build

# ── 阶段 1: 构建 ─────────────────────────────────────────────────
FROM rust:1.92-bookworm AS builder

WORKDIR /app

# cargo-chef 风格缓存:先只复制 manifest,让依赖层可缓存
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/
COPY crates/plugin-api/Cargo.toml crates/plugin-api/
COPY crates/mcp-http/Cargo.toml crates/mcp-http/
COPY crates/adapter-qq/Cargo.toml crates/adapter-qq/
COPY crates/webui/Cargo.toml crates/webui/
COPY crates/yuantuan/Cargo.toml crates/yuantuan/
COPY plugins/anysearch/Cargo.toml plugins/anysearch/
COPY plugins/social-media/Cargo.toml plugins/social-media/
COPY plugins/moderator/Cargo.toml plugins/moderator/
COPY plugins/nai-image/Cargo.toml plugins/nai-image/
COPY plugins/qzone/Cargo.toml plugins/qzone/
COPY plugins/tencent-docs/Cargo.toml plugins/tencent-docs/
COPY plugins/quark-drive/Cargo.toml plugins/quark-drive/
COPY plugins/qq/Cargo.toml plugins/qq/

# 占位的 dummy src,让 cargo fetch 只拉依赖
RUN for d in crates/core crates/plugin-api crates/mcp-http crates/adapter-qq crates/webui crates/yuantuan \
        plugins/anysearch plugins/social-media plugins/moderator plugins/nai-image plugins/qzone plugins/tencent-docs plugins/quark-drive plugins/qq; do \
        mkdir -p "$d/src" && echo "fn main(){}" > "$d/src/lib.rs"; \
    done && \
    echo "fn main(){}" > crates/yuantuan/src/main.rs && \
    cargo fetch --locked && \
    rm -rf crates plugins

# 真源码再拷进来,只增量编译业务代码
COPY crates/ crates/
COPY plugins/ plugins/
COPY --from=frontend /app/crates/webui/static/ crates/webui/static/

# release 构建
RUN cargo build --release --locked -p yuantuan

# ── 阶段 2: 运行时 ────────────────────────────────────────────────
FROM debian:bookworm-slim AS connector-runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libstdc++6 \
        libatomic1 \
        curl \
        tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node-runtime /usr/local/bin/node /usr/local/bin/node
RUN node --version

# uid 1000 = 主流发行版首个普通用户(ubuntu user 在多数 VM 里是 1000);
# 跟宿主机挂载的 config/data 目录所有者 uid 对齐,容器内 yuantuan 可读可写
RUN useradd -r -u 1000 -m -s /usr/sbin/nologin yuantuan 2>/dev/null || \
    useradd -m -s /usr/sbin/nologin yuantuan

WORKDIR /app


# 让 config 跟 data 都放 /app 下(volumes 挂进来)
RUN mkdir -p /app/data && chown -R yuantuan:yuantuan /app
USER yuantuan

FROM connector-runtime AS runtime
ENV YUANTUAN_RESTART_MODE=supervised
COPY --from=builder /app/target/release/yuantuan /usr/local/bin/yuantuan

EXPOSE 8085 6199

# 容器内不 fork、不 detach;Rust 进程 PID 1 镜像(用 tini 可换)
ENTRYPOINT ["/usr/local/bin/yuantuan"]

# 可选后端 target：仅增加 Docker CLI；开发工具始终位于独立沙箱镜像。
FROM docker:27-cli AS docker-cli
FROM runtime AS sandbox-runtime
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker

# 普通构建保持原运行时；沙箱部署显式选择 sandbox-runtime。
FROM runtime AS default
