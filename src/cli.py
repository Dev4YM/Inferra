from __future__ import annotations

import argparse
import asyncio
from dataclasses import replace
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

from ai import AIService, OllamaError, OllamaProvider, gemma4_model, list_gemma4_models, recommended_gemma4_model
from app import InferraRuntime
from config import PRESET_NAMES, InferraConfig, apply_preset, dump_config, load_config, set_config_value, write_config
from storage import initialize_storage


def _version() -> str:
    try:
        return version("inferra")
    except PackageNotFoundError:
        return "0.1.0"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="inferra")
    parser.add_argument("--config", default="inferra.toml", help="Path to inferra.toml")
    parser.add_argument("--version", action="store_true", help="Print version and exit")
    sub = parser.add_subparsers(dest="command")

    sub.add_parser("check-config")
    sub.add_parser("init-db")

    setup = sub.add_parser("setup")
    setup.add_argument("--local", action="store_true", help="Use local Ollama at http://127.0.0.1:11434")
    setup.add_argument("--remote-url", help="Use a remote Ollama-compatible server URL")
    setup.add_argument("--token-env", default="", help="Environment variable containing a bearer token")
    setup.add_argument("--model", default=None, help="Gemma 4/Ollama model tag to configure")
    setup.add_argument("--data-dir", default=None, help="SQLite data directory")
    setup.add_argument("--yes", action="store_true", help="Accept non-destructive defaults")
    setup.add_argument("--pull", action="store_true", help="Pull the configured model after confirmation or --yes")
    setup.add_argument("--skip-connection-test", action="store_true", help="Write config without contacting Ollama")

    ai = sub.add_parser("ai")
    ai_sub = ai.add_subparsers(dest="ai_command")
    ai_sub.add_parser("status")
    ai_sub.add_parser("models")
    ai_pull = ai_sub.add_parser("pull")
    ai_pull.add_argument("model", nargs="?", help="Model tag to pull; defaults to configured model")
    ai_pull.add_argument("--yes", action="store_true", help="Pull without an interactive confirmation")
    ai_test = ai_sub.add_parser("test")
    ai_test.add_argument("--model", help="Temporarily test a specific model tag")

    config_cmd = sub.add_parser("config")
    config_sub = config_cmd.add_subparsers(dest="config_command")
    config_sub.add_parser("show")
    config_set = config_sub.add_parser("set")
    config_set.add_argument("key")
    config_set.add_argument("value")
    config_preset = config_sub.add_parser("preset")
    config_preset.add_argument("name", choices=PRESET_NAMES)

    ingest = sub.add_parser("ingest-file")
    ingest.add_argument("path")
    ingest.add_argument("--service-id")

    processes = sub.add_parser("collect-processes")
    processes.add_argument("--top", type=int, default=None, help="Maximum process events to ingest")
    processes.add_argument("--min-cpu", type=float, default=None, help="CPU percent threshold")
    processes.add_argument("--min-memory-mb", type=float, default=None, help="Resident memory threshold")

    host_metrics = sub.add_parser("collect-host")
    host_metrics.add_argument("--warn-cpu", type=float, default=None, help="CPU warning threshold")
    host_metrics.add_argument("--warn-memory", type=float, default=None, help="Memory warning threshold")
    host_metrics.add_argument("--warn-disk", type=float, default=None, help="Disk warning threshold")

    services = sub.add_parser("collect-services")
    services.add_argument("--include-stopped", action="store_true", help="Include stopped Windows services")
    services.add_argument("--name", action="append", default=[], help="Specific Windows service name to include")

    eventlog = sub.add_parser("collect-eventlog")
    eventlog.add_argument("--channel", action="append", default=[], help="Windows Event Log channel to read")

    syslog = sub.add_parser("collect-syslog")
    syslog.add_argument("--path", action="append", default=[], help="Syslog file path to read")

    journald = sub.add_parser("collect-journald")
    journald.add_argument("--unit", action="append", default=[], help="systemd unit to include")
    journald.add_argument("--since", default=None, help="journalctl --since value for first read")
    journald.add_argument("--limit", type=int, default=None, help="Maximum journal rows for first read")

    kubernetes = sub.add_parser("collect-kubernetes")
    kubernetes.add_argument("--namespace", action="append", default=[], help="Kubernetes namespace to include")
    kubernetes.add_argument("--current-namespace", action="store_true", help="Use configured namespaces instead of all namespaces")
    kubernetes.add_argument("--limit", type=int, default=None, help="Maximum Kubernetes events/pods to read")
    kubernetes.add_argument("--skip-pods", action="store_true", help="Do not collect pod state snapshots")
    kubernetes.add_argument("--skip-events", action="store_true", help="Do not collect Kubernetes Events")

    run_collectors = sub.add_parser("run-collectors")
    run_collectors.add_argument("--duration", type=float, default=None, help="Run collectors for N seconds, then exit")

    collectors_cmd = sub.add_parser("collectors")
    collectors_sub = collectors_cmd.add_subparsers(dest="collectors_command")
    collectors_sub.add_parser("status")

    topology = sub.add_parser("topology-add")
    topology.add_argument("source")
    topology.add_argument("target")
    topology.add_argument("--type", default="depends_on", dest="relation_type")

    serve = sub.add_parser("serve")
    serve.add_argument("--host")
    serve.add_argument("--port", type=int)

    args = parser.parse_args(argv)
    if args.version:
        print(_version())
        return 0

    config = load_config(args.config)

    if args.command == "check-config":
        print("Inferra config OK")
        print(f"data_dir={config.storage.data_dir}")
        print(f"server={config.server.host}:{config.server.port}")
        return 0

    if args.command == "init-db":
        initialize_storage(config.storage.data_dir)
        print(f"Initialized SQLite storage under {config.storage.data_dir}")
        return 0

    if args.command == "setup":
        return _setup(args)

    if args.command == "ai":
        return _ai(config, args)

    if args.command == "config":
        return _config(args)

    if args.command == "ingest-file":
        return asyncio.run(_ingest_file(config, args.path, args.service_id))

    if args.command == "collect-processes":
        return asyncio.run(_collect_processes(config, args))

    if args.command == "collect-host":
        return asyncio.run(_collect_host_metrics(config, args))

    if args.command == "collect-services":
        return asyncio.run(_collect_services(config, args))

    if args.command == "collect-eventlog":
        return asyncio.run(_collect_eventlog(config, args))

    if args.command == "collect-syslog":
        return asyncio.run(_collect_syslog(config, args))

    if args.command == "collect-journald":
        return asyncio.run(_collect_journald(config, args))

    if args.command == "collect-kubernetes":
        return asyncio.run(_collect_kubernetes(config, args))

    if args.command == "run-collectors":
        return asyncio.run(_run_collectors(config, args.duration))

    if args.command == "collectors":
        return _collectors(config, args)

    if args.command == "topology-add":
        runtime = InferraRuntime(config)
        runtime.add_topology_relation(args.source, args.target, args.relation_type)
        runtime.event_store.close()
        runtime.incident_store.close()
        print(f"Added topology edge: {args.source} -> {args.target} ({args.relation_type})")
        return 0

    if args.command == "serve":
        return _serve(config, host=args.host, port=args.port, config_path=args.config)

    parser.print_help()
    return 0


async def _ingest_file(config, path: str, service_id: str | None) -> int:
    runtime = InferraRuntime(config)
    await runtime.start()
    try:
        inserted = await runtime.ingest_file_once(Path(path), service_id=service_id)
    finally:
        await runtime.stop()
    print(f"Ingested {inserted} events from {path}")
    return 0


async def _collect_processes(config: InferraConfig, args) -> int:
    from collectors import ProcessSnapshotCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.process
    collector = ProcessSnapshotCollector(
        poll_interval_seconds=collector_config.poll_interval_seconds,
        top_n=args.top or collector_config.top_n,
        min_cpu_percent=args.min_cpu if args.min_cpu is not None else collector_config.min_cpu_percent,
        min_memory_mb=args.min_memory_mb if args.min_memory_mb is not None else collector_config.min_memory_mb,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} process snapshots; stored {inserted} events.")
    return 0


async def _collect_host_metrics(config: InferraConfig, args) -> int:
    from collectors import HostMetricsCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.host_metrics
    collector = HostMetricsCollector(
        poll_interval_seconds=collector_config.poll_interval_seconds,
        warn_cpu_percent=args.warn_cpu if args.warn_cpu is not None else collector_config.warn_cpu_percent,
        warn_memory_percent=args.warn_memory if args.warn_memory is not None else collector_config.warn_memory_percent,
        warn_disk_percent=args.warn_disk if args.warn_disk is not None else collector_config.warn_disk_percent,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} host metric snapshots; stored {inserted} events.")
    return 0


async def _collect_services(config: InferraConfig, args) -> int:
    from collectors import WindowsServiceCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.windows_service
    names = tuple(args.name) if args.name else collector_config.names
    collector = WindowsServiceCollector(
        poll_interval_seconds=collector_config.poll_interval_seconds,
        include_stopped=args.include_stopped or collector_config.include_stopped,
        names=names,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} Windows service snapshots; stored {inserted} events.")
    return 0


async def _collect_eventlog(config: InferraConfig, args) -> int:
    from collectors import WindowsEventLogCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.windows_eventlog
    channels = tuple(args.channel) if args.channel else collector_config.channels
    collector = WindowsEventLogCollector(
        channels=channels,
        poll_interval_seconds=collector_config.poll_interval_seconds,
        state_store=runtime.event_store,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} Windows Event Log records; stored {inserted} events.")
    return 0


async def _collect_syslog(config: InferraConfig, args) -> int:
    from collectors import LinuxSyslogCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.linux_syslog
    paths = tuple(args.path) if args.path else collector_config.paths
    collector = LinuxSyslogCollector(
        paths=paths,
        poll_interval_seconds=collector_config.poll_interval_seconds,
        start_at_end=False,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} syslog records; stored {inserted} events.")
    return 0


async def _collect_journald(config: InferraConfig, args) -> int:
    from collectors import JournaldCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.journald
    units = tuple(args.unit) if args.unit else collector_config.units
    collector = JournaldCollector(
        units=units,
        since=args.since or collector_config.since,
        limit=args.limit or collector_config.limit,
        poll_interval_seconds=collector_config.poll_interval_seconds,
        state_store=runtime.event_store,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} journald records; stored {inserted} events.")
    return 0


async def _collect_kubernetes(config: InferraConfig, args) -> int:
    from collectors import KubernetesCollector

    runtime = InferraRuntime(config)
    collector_config = config.collectors.kubernetes
    namespaces = tuple(args.namespace) if args.namespace else collector_config.namespaces
    collector = KubernetesCollector(
        namespaces=namespaces,
        all_namespaces=collector_config.all_namespaces and not args.current_namespace and not namespaces,
        limit=args.limit or collector_config.limit,
        include_pods=collector_config.include_pods and not args.skip_pods,
        include_events=collector_config.include_events and not args.skip_events,
        poll_interval_seconds=collector_config.poll_interval_seconds,
    )
    emitted, inserted = await _collect_once_into_runtime(runtime, collector)
    print(f"Collected {emitted} Kubernetes records; stored {inserted} events.")
    return 0


async def _run_collectors(config: InferraConfig, duration: float | None) -> int:
    runtime = InferraRuntime(config)
    await runtime.start(start_collectors=True)
    try:
        print(f"Started {len(runtime.collector_health())} collectors.")
        if duration is not None:
            await asyncio.sleep(duration)
        else:
            while True:
                await asyncio.sleep(3600)
    except KeyboardInterrupt:
        print("Stopping collectors...")
    finally:
        await runtime.stop()
    return 0


async def _collect_once_into_runtime(runtime: InferraRuntime, collector) -> tuple[int, int]:
    queue = asyncio.Queue()
    try:
        emitted = await collector.collect_once(queue)
        inserted = 0
        while not queue.empty():
            event = await queue.get()
            stored = await runtime.ingest_raw(event)
            inserted += 1 if stored is not None else 0
            queue.task_done()
        return emitted, inserted
    finally:
        runtime.event_store.close()
        runtime.incident_store.close()


def _serve(config, host: str | None = None, port: int | None = None, config_path: str | Path | None = None) -> int:
    import uvicorn
    from web import create_app

    app = create_app(config, config_path=config_path)
    uvicorn.run(app, host=host or config.server.host, port=port or config.server.port)
    return 0


def _collectors(config: InferraConfig, args) -> int:
    if args.collectors_command == "status":
        runtime = InferraRuntime(config)
        try:
            rows = runtime.collector_health()
            if not rows:
                print("No collectors configured for this platform.")
                return 0
            for row in rows:
                print(
                    f"{row['source_type']:<18} {row['status']:<10} "
                    f"events={row['events_emitted']} errors={row['error_count']} id={row['collector_id']}"
                )
            return 0
        finally:
            runtime.event_store.close()
            runtime.incident_store.close()
    print("Missing collectors subcommand. Use: inferra collectors status")
    return 1


def _setup(args) -> int:
    config_path = Path(args.config)
    existing = load_config(config_path)
    model = args.model or existing.ai.model or recommended_gemma4_model().name
    if gemma4_model(model) is None and model.startswith("gemma4:"):
        print(f"Warning: {model} is not in the bundled Gemma 4 registry.")

    base_url = _setup_base_url(args, existing)
    data_dir = Path(args.data_dir) if args.data_dir else existing.storage.data_dir
    allow_remote = not _is_local_ollama(base_url)
    ai_config = replace(
        existing.ai,
        enabled=True,
        provider="ollama",
        base_url=base_url,
        model=model,
        token_env=args.token_env or existing.ai.token_env,
        allow_remote=allow_remote,
    )
    updated = replace(existing, storage=replace(existing.storage, data_dir=data_dir), ai=ai_config)
    initialize_storage(updated.storage.data_dir)

    if not args.skip_connection_test:
        provider = OllamaProvider(updated.ai)
        status = provider.status()
        if status.available:
            print(f"Ollama reachable at {updated.ai.base_url}")
            print(f"Configured model installed: {status.installed}")
            if not status.installed:
                _maybe_pull(provider, updated.ai.model, args.pull, args.yes)
        else:
            print(f"Ollama is not reachable yet: {status.error}")
    write_config(updated, config_path)
    print(f"Wrote config to {config_path}")
    print(f"Initialized SQLite storage under {updated.storage.data_dir}")
    print(f"AI provider: ollama ({updated.ai.base_url})")
    print(f"AI model: {updated.ai.model}")
    return 0


def _ai(config: InferraConfig, args) -> int:
    service = AIService(config)
    command = args.ai_command
    if command == "status":
        status = service.status()
        for key in ("enabled", "provider", "base_url", "model", "available", "installed", "reason", "error"):
            if key in status and status[key] is not None:
                print(f"{key}={status[key]}")
        return 0
    if command == "models":
        installed: set[str] = set()
        if config.ai.enabled:
            try:
                installed = set(service.installed_models())
            except OllamaError as exc:
                print(f"Could not list installed Ollama models: {exc}")
        for model in service.registry():
            marker = "*" if model["name"] in installed else " "
            local = "local" if model["local_weight"] else "cloud"
            print(f"{marker} {model['name']:<32} {model['size']:<7} {model['context_window']:<5} {local} {model['variant']}")
        return 0
    if command == "pull":
        model = args.model or config.ai.model
        provider = OllamaProvider(config.ai)
        if not args.yes and not _confirm(f"Pull {model}? This can download a large model weight."):
            print("Pull cancelled.")
            return 1
        result = provider.pull_model(model)
        print(result.get("status", "Pull requested."))
        return 0
    if command == "test":
        ai_config = config.ai
        if args.model:
            ai_config = replace(ai_config, model=args.model)
        try:
            print(OllamaProvider(ai_config).test())
            return 0
        except OllamaError as exc:
            print(f"AI test failed: {exc}")
            return 1
    print("Missing ai subcommand. Use: inferra ai status|models|pull|test")
    return 1


def _config(args) -> int:
    if args.config_command == "show":
        config = load_config(args.config)
        print(dump_config(config))
        return 0
    if args.config_command == "set":
        updated = set_config_value(args.config, args.key, args.value)
        print(f"Updated {args.key}")
        print(f"server={updated.server.host}:{updated.server.port}")
        print(f"ai={updated.ai.provider}:{updated.ai.model}")
        return 0
    if args.config_command == "preset":
        updated = apply_preset(load_config(args.config), args.name)
        write_config(updated, args.config)
        print(f"Applied preset {args.name}")
        print(f"collectors.auto_start={updated.collectors.auto_start}")
        return 0
    print("Missing config subcommand. Use: inferra config show|set|preset")
    return 1


def _setup_base_url(args, existing: InferraConfig) -> str:
    if args.remote_url:
        return args.remote_url.rstrip("/")
    if args.local or args.yes:
        return "http://127.0.0.1:11434"
    current = existing.ai.base_url or "http://127.0.0.1:11434"
    answer = input(f"Ollama base URL [{current}]: ").strip()
    return (answer or current).rstrip("/")


def _maybe_pull(provider: OllamaProvider, model: str, pull_requested: bool, yes: bool) -> None:
    if not pull_requested:
        print("Model is not installed. Run `inferra ai pull --yes` when you are ready to download it.")
        return
    if yes or _confirm(f"Pull {model}? This can download a large model weight."):
        result = provider.pull_model(model)
        print(result.get("status", "Pull requested."))


def _confirm(prompt: str) -> bool:
    return input(f"{prompt} [y/N] ").strip().lower() in {"y", "yes"}


def _is_local_ollama(base_url: str) -> bool:
    normalized = base_url.lower()
    return "127.0.0.1" in normalized or "localhost" in normalized


if __name__ == "__main__":
    raise SystemExit(main())
