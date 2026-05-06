# Workspace Intelligence

Workspace intelligence is core to the product. Inferra should not only observe runtime. It should understand what local projects may be responsible for that runtime.

## Current State

The repo has a workspace scanner that detects project markers such as:

- `package.json`
- `pyproject.toml`
- `requirements.txt`
- `Dockerfile`
- `go.mod`
- `Cargo.toml`
- `*.csproj`
- `pom.xml`

This is no longer only discovery. The first control-plane pass also includes:

- workspace mapping through `src/runtime/workspace_map.py`
- workspace scanning through `src/runtime/workspace_scan.py`
- explicit service mappings in config
- CLI commands for map, services, and inspect
- API endpoints for map, services, inspect, and user-provided mappings
- a Workspace tab in the React control plane

The remaining work is depth: richer runtime-to-code signals, better project drilldowns, and tighter incident/service ownership views.

## Target Capability

Inferra should build a local workspace model:

```text
project
  path
  kind
  marker
  services
  compose files
  packages
  scripts
  ports
  env hints
  runtime process links
  container links
  incidents
```

## Runtime-to-Workspace Mapping

Possible signals:

- process current working directory
- command line path
- Docker Compose project name
- container labels
- image name
- exposed ports
- service names in logs
- package scripts
- config files
- topology config
- user-provided mappings

Mapping should have confidence:

```json
{
  "service_id": "api",
  "project_path": "D:/Projects/app",
  "confidence": 0.82,
  "signals": ["process_cwd", "compose_service", "log_service_name"]
}
```

## Workspace Views

CLI:

```powershell
inferra workspace
inferra workspace map
inferra workspace services
inferra workspace inspect D:\Projects\app
inferra investigate workspace
inferra services list
inferra services show api
```

Future CLI should add richer explanation commands such as `inferra workspace explain`, plus guided mapping repair when Inferra has low confidence.

Web:

- project list
- project detail
- service mappings
- related incidents
- config hints
- detected commands
- runtime links

## AI Use

AI should use workspace context to:

- explain likely project ownership
- suggest read-only checks
- connect logs to codebase layout
- identify missing mappings
- produce developer-focused incident summaries

AI should not read secrets into prompts. Environment files should be summarized with redaction.

## Config

Target config:

```toml
[workspace]
enabled = true
roots = []
max_depth = 4
max_results = 100
map_runtime = true
redact_env_files = true

[[workspace.service_mappings]]
service_id = "api"
project_path = "D:/Projects/app"
confidence = 1.0
source = "user"
```

## Acceptance

Workspace intelligence is useful when:

- dashboard shows detected projects
- incidents link to likely project context
- services can show project ownership
- AI can include workspace-aware next steps
- developer mode shows mapping confidence and signals
