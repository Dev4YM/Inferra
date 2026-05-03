# Collector Commands

Windows:

```powershell
inferra collect-host
inferra collect-processes
inferra collect-services --include-stopped
inferra collect-eventlog --channel Application
```

Linux:

```bash
inferra collect-host
inferra collect-processes
inferra collect-syslog --path /var/log/syslog
inferra collect-journald --unit nginx.service --since "-1 hour"
```

Kubernetes:

```bash
python -m pip install ".[kubernetes]"
inferra collect-kubernetes --namespace default
```

All `collect-*` commands are safe one-shot ingests.

For supervised collection:

```bash
inferra config preset linux-node
inferra run-collectors
```

Available presets are `web-only`, `windows-server`, `linux-node`, and `kubernetes`.
