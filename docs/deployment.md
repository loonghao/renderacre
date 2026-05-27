# Renderacre lightweight deployment

This guide packages the small-studio profile: one controller, SQLite durability,
the built dashboard served by the controller, shared storage for scene/output
paths, and one worker process on each render node. It does not require
Kubernetes, an external database, or a multi-service control plane.

## Single-node controller

Build the dashboard once on the controller host:

```powershell
npm --prefix dashboard ci
npm --prefix dashboard run build
```

Then start the durable controller and dashboard with one command:

```powershell
renderacre-controller --config deploy/lightweight/controller.yaml
```

The sample profile stores scheduler state in `./var/renderacre/renderacre.sqlite3`
and serves `./dashboard/dist` from the same HTTP listener as the REST API. In a
source checkout, create the data directory before the first run:

```powershell
New-Item -ItemType Directory -Force var/renderacre
```

Equivalent environment configuration is available when a config file is not
convenient:

```powershell
$env:RFARM_BIND = "0.0.0.0:7878"
$env:RFARM_STORAGE = "sqlite"
$env:RFARM_SQLITE_PATH = "C:\ProgramData\Renderacre\renderacre.sqlite3"
$env:RFARM_DASHBOARD_DIR = "C:\ProgramData\Renderacre\dashboard"
renderacre-controller
```

CLI arguments and environment variables override config-file values. Omitted
values fall back to the built-in defaults.

## Worker install and registration

Install the release binaries on every worker host:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/install.ps1
```

```bash
curl -fsSL https://raw.githubusercontent.com/loonghao/renderacre/main/scripts/install.sh | sh
```

Windows workers can use either the installed binary directory or an explicit
path to `renderacre-worker.exe`. macOS and Linux workers can place
`renderacre-worker` in `/usr/local/bin` or another service account path.

Register a worker from a config file:

```powershell
renderacre-worker --config deploy/lightweight/worker.yaml
```

Or use environment variables:

```powershell
$env:RFARM_CONTROLLER = "http://controller-host:7878"
$env:RFARM_WORKER_NAME = $env:COMPUTERNAME
$env:RFARM_WORKER_LABELS = "pool=lighting,app=blender"
$env:RFARM_WORKER_SLOTS = "4"
renderacre-worker
```

Labels describe scheduling capabilities. Use `pool`, `app`, operating-system,
GPU, or DCC-specific labels that match submitted task requirements.

## Service examples

Systemd examples live in `deploy/lightweight/systemd/`.
Create a `renderacre` service user first, then install the config and service
files:

```bash
sudo install -d -o renderacre -g renderacre /etc/renderacre /var/lib/renderacre /opt/renderacre
sudo install -m 0644 deploy/lightweight/controller.yaml /etc/renderacre/controller.yaml
sudo install -m 0644 deploy/lightweight/worker.yaml /etc/renderacre/worker.yaml
sudo install -m 0644 deploy/lightweight/systemd/renderacre-controller.service /etc/systemd/system/
sudo install -m 0644 deploy/lightweight/systemd/renderacre-worker.service /etc/systemd/system/
```

Edit `/etc/renderacre/controller.yaml` so `sqlite_path` points at
`/var/lib/renderacre/renderacre.sqlite3` and `dashboard_dir` points at the
deployed dashboard assets, for example `/opt/renderacre/dashboard/dist`. Then
enable the services:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now renderacre-controller
sudo systemctl enable --now renderacre-worker
```

Windows service examples live in `deploy/lightweight/windows/`. Run PowerShell as
Administrator after copying the binaries, config files, and dashboard assets to
their final paths:

```powershell
powershell -ExecutionPolicy Bypass -File deploy/lightweight/windows/install-controller-service.ps1
powershell -ExecutionPolicy Bypass -File deploy/lightweight/windows/install-worker-service.ps1
Start-Service RenderacreController
Start-Service RenderacreWorker
```

For service-managed Windows installs, prefer absolute paths in
`C:\ProgramData\Renderacre\controller.yaml` because the Windows service working
directory is manager-dependent.

## Backup and upgrades

For the default SQLite profile, back up the controller database before upgrades.
The safest path is to stop the controller, copy the SQLite file, upgrade the
binaries, then start the controller again:

```powershell
Stop-Service RenderacreController
Copy-Item C:\ProgramData\Renderacre\renderacre.sqlite3 C:\Backups\renderacre.sqlite3
Start-Service RenderacreController
```

On Linux:

```bash
sudo systemctl stop renderacre-controller
sudo cp /var/lib/renderacre/renderacre.sqlite3 /var/backups/renderacre.sqlite3
sudo systemctl start renderacre-controller
```

Upgrade controller binaries before worker binaries when a release changes API
behavior. Keep the previous controller and worker binaries until the dashboard
loads, `/readyz` returns `status: "ok"`, and workers register successfully.

## Growing the farm

Keep the lightweight profile until SQLite backup windows, controller CPU, or
network placement becomes the bottleneck. The next steps are additive:

- Put the controller behind a reverse proxy that owns TLS, authentication, and
  network policy.
- Move dashboard assets to the proxy or static web tier if controller-local
  serving is no longer desirable.
- Graduate the storage boundary from SQLite to a future Postgres or managed
  backend while preserving submitter and worker contracts.
- Move large logs and artifacts to object storage behind the controller artifact
  APIs.

The stable and experimental extension boundaries are described in
[extension-contracts.md](extension-contracts.md).
