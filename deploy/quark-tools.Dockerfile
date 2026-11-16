# Official supported runtime; no model credentials are provided to this service.
FROM ghcr.io/anomalyco/opencode@sha256:f4cfe16d91efbbfca01a51c44094fba4cf82e15c7fa17de5f11f9059699b1e6e
USER root
RUN apk add --no-cache nodejs ca-certificates && \
    addgroup -g 1000 quark && adduser -D -u 1000 -G quark quark && \
    mkdir -p /workspace /home/quark/.local/share /opt/quark-tools && \
    chown -R 1000:1000 /workspace /home/quark
COPY plugins/quark-drive/worker/job.cjs plugins/quark-drive/worker/server.cjs plugins/quark-drive/worker/health.cjs /opt/quark-tools/
ENV HOME=/home/quark
USER 1000:1000
WORKDIR /workspace
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["node","/opt/quark-tools/health.cjs"]
ENTRYPOINT ["node","/opt/quark-tools/server.cjs"]
