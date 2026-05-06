FROM python:3.12-slim AS builder

WORKDIR /build
RUN pip install --no-cache-dir build
COPY . .
RUN python -m build --wheel --outdir /dist

FROM python:3.12-slim

RUN useradd --system --uid 1000 --create-home inferra
RUN mkdir -p /data && chown inferra:inferra /data

WORKDIR /app
COPY --from=builder /dist /dist
RUN WHEEL="$(ls /dist/*.whl | head -n1)" && pip install --no-cache-dir "${WHEEL}[kubernetes]" && rm -rf /dist

USER inferra
WORKDIR /home/inferra

EXPOSE 7433

ENV INFERRA_CONFIG=/etc/inferra/inferra.toml

CMD ["inferra", "--config", "/etc/inferra/inferra.toml", "serve", "--data-dir", "/data", "--host", "0.0.0.0", "--port", "7433"]
