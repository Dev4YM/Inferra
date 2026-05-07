#!/bin/sh
set -eu

CONFIG_PATH="${INFERRA_CONFIG:-/etc/inferra/inferra.toml}"

/app/inferra --config "${CONFIG_PATH}" init-db
exec /app/inferra --config "${CONFIG_PATH}" serve
