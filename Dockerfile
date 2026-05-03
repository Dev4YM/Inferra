FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    INFERRA_CONFIG=/etc/inferra/inferra.toml

WORKDIR /app
COPY pyproject.toml README.md ./
COPY src ./src

RUN pip install --no-cache-dir .

RUN useradd --create-home --uid 10001 inferra \
    && mkdir -p /data /etc/inferra \
    && chown -R inferra:inferra /data /etc/inferra

USER inferra
EXPOSE 7433
VOLUME ["/data", "/etc/inferra"]

CMD ["inferra", "--config", "/etc/inferra/inferra.toml", "serve", "--host", "0.0.0.0", "--port", "7433"]
