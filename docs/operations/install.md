# Install Inferra

## Local Python

```powershell
python -m pip install -e ".[dev]"
inferra --config inferra.toml setup --yes --skip-connection-test
inferra --config inferra.toml serve
```

Open `http://127.0.0.1:7433`.

## Windows Service

Run PowerShell as Administrator:

```powershell
python -m pip install -e ".[windows]"
.\deploy\windows\install-service.ps1 -Python python
```

Remove it with:

```powershell
.\deploy\windows\uninstall-service.ps1
```

## Linux systemd

Install the project into `/opt/inferra`, create `/etc/inferra/inferra.toml`, then copy:

```bash
sudo cp deploy/systemd/inferra.service /etc/systemd/system/inferra.service
sudo systemctl daemon-reload
sudo systemctl enable --now inferra
```

## Docker

```bash
docker compose up --build
```

## Kubernetes

```bash
helm install inferra deploy/helm/inferra
```

## macOS Launch Agent

Copy `deploy/macos/com.inferra.agent.plist` to `~/Library/LaunchAgents/`, adjust paths if needed, then run:

```bash
launchctl load ~/Library/LaunchAgents/com.inferra.agent.plist
```
