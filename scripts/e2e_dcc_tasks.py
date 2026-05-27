#!/usr/bin/env python3
"""Run Renderacre end-to-end jobs against generic commands and DCC hosts."""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


def main():
    args = parse_args()
    repo = Path(__file__).resolve().parents[1]
    port = args.port or int(os.environ.get("RENDERACRE_E2E_PORT", "17878"))
    jobs = resolve_jobs(args.jobs, args.blender_exe, args.maya_python, args.shells)

    if args.build:
        run(["cargo", "build", "-p", "renderacre-controller", "-p", "renderacre-worker"], repo)

    controller_bin = resolve_binary(repo, "renderacre-controller", args.controller_bin)
    worker_bin = resolve_binary(repo, "renderacre-worker", args.worker_bin)
    run_dir = repo / "target" / "e2e-dcc"
    run_dir.mkdir(parents=True, exist_ok=True)

    controller = None
    worker = None
    try:
        controller = start_process(
            [str(controller_bin), "--bind", "127.0.0.1:{0}".format(port)],
            repo,
            run_dir / "controller.out.log",
            run_dir / "controller.err.log",
        )
        wait_controller(port)

        worker = start_process(
            [
                str(worker_bin),
                "--controller",
                "http://127.0.0.1:{0}".format(port),
                "--name",
                "dcc-e2e-worker",
                "--label",
                "e2e=dcc",
            ],
            repo,
            run_dir / "worker.out.log",
            run_dir / "worker.err.log",
        )
        time.sleep(1.0)

        results = []
        for job in jobs:
            if job == "python":
                results.append(run_python_job(repo, port, run_dir, args.python_exe))
            elif job == "command":
                results.append(run_command_job(port, run_dir, args.shells))
            elif job == "blender":
                results.append(run_blender_job(repo, port, run_dir, args.blender_exe))
            elif job == "maya":
                results.append(run_maya_job(repo, port, run_dir, args.maya_python))
            else:
                raise ValueError("unknown job: {0}".format(job))

        print("DCC E2E passed:")
        for result in results:
            print("  {name}: {job_id} ({tasks} tasks)".format(**result))
    except Exception:
        dump_log("controller stdout", run_dir / "controller.out.log")
        dump_log("controller stderr", run_dir / "controller.err.log")
        dump_log("worker stdout", run_dir / "worker.out.log")
        dump_log("worker stderr", run_dir / "worker.err.log")
        raise
    finally:
        stop_process(worker)
        stop_process(controller)


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--jobs",
        default="auto",
        help="Comma-separated jobs: auto, python, command, blender, maya, or all.",
    )
    parser.add_argument("--port", type=int)
    parser.add_argument("--build", dest="build", action="store_true", default=True)
    parser.add_argument("--skip-build", dest="build", action="store_false")
    parser.add_argument("--controller-bin")
    parser.add_argument("--worker-bin")
    parser.add_argument("--python-exe", default=sys.executable)
    parser.add_argument(
        "--shells",
        default=os.environ.get("RENDERACRE_E2E_SHELLS", "auto"),
        help="Comma-separated shells for command jobs: auto, cmd, powershell, pwsh, bash, sh.",
    )
    parser.add_argument("--blender-exe", default=os.environ.get("BLENDER_EXE", "blender"))
    parser.add_argument("--maya-python", default=os.environ.get("MAYA_PYTHON", "mayapy"))
    return parser.parse_args()


def resolve_jobs(value, blender_exe, maya_python, shells):
    requested = [item.strip().lower() for item in value.split(",") if item.strip()]
    requested = ["command" if job == "shell" else job for job in requested]
    if requested == ["all"]:
        requested = ["python", "command", "blender", "maya"]
    if requested == ["auto"]:
        jobs = ["python"]
        if resolve_shells(shells):
            jobs.append("command")
        if find_executable(blender_exe):
            jobs.append("blender")
        if find_executable(maya_python):
            jobs.append("maya")
        return jobs

    allowed = {"python", "command", "blender", "maya"}
    unknown = [job for job in requested if job not in allowed]
    if unknown:
        raise ValueError("unknown jobs: {0}".format(", ".join(unknown)))
    return requested


def run(command, cwd):
    print("+ {0}".format(" ".join(command)))
    subprocess.run(command, cwd=str(cwd), check=True)


def resolve_binary(repo, name, override):
    if override:
        path = Path(override).resolve()
        if not path.exists():
            raise FileNotFoundError(str(path))
        return path

    binary = name + (".exe" if os.name == "nt" else "")
    candidates = [
        repo / "target" / "debug" / binary,
        repo / "target" / "release" / binary,
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise FileNotFoundError(
        "could not find {0}; run cargo build or pass --{1}-bin".format(
            binary, name.replace("renderacre-", "")
        )
    )


def find_executable(value):
    path = Path(value)
    if path.exists():
        return str(path.resolve())
    return shutil.which(value)


def require_executable(value, label):
    resolved = find_executable(value)
    if not resolved:
        raise FileNotFoundError("{0} executable not found: {1}".format(label, value))
    return resolved


def resolve_shells(value):
    requested = [item.strip().lower() for item in value.split(",") if item.strip()]
    if not requested or requested == ["auto"]:
        requested = default_shell_order()

    specs = []
    seen = set()
    candidates = shell_candidates()
    for name in requested:
        if name in seen:
            continue
        spec = candidates.get(name)
        if spec is None:
            raise ValueError("unknown shell: {0}".format(name))
        executable = find_executable(spec["executable"])
        if executable:
            resolved = dict(spec)
            resolved["executable"] = executable
            if name in ("bash", "sh"):
                resolved["path_style"] = detect_posix_path_style(executable)
            specs.append(resolved)
            seen.add(name)
    return specs


def default_shell_order():
    if os.name == "nt":
        return ["cmd", "powershell", "pwsh", "bash"]
    return ["bash", "sh", "pwsh"]


def shell_candidates():
    return {
        "cmd": {"name": "cmd", "executable": "cmd.exe", "args": ["/C"]},
        "powershell": {
            "name": "powershell",
            "executable": "powershell.exe",
            "args": ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"],
        },
        "pwsh": {"name": "pwsh", "executable": "pwsh", "args": ["-NoProfile", "-Command"]},
        "bash": {"name": "bash", "executable": "bash", "args": ["-lc"]},
        "sh": {"name": "sh", "executable": "sh", "args": ["-c"]},
    }


def detect_posix_path_style(executable):
    if os.name != "nt":
        return "native"
    probe = "if command -v cygpath >/dev/null 2>&1; then echo msys; elif [ -d /mnt/c ]; then echo wsl; else echo native; fi"
    try:
        completed = subprocess.run(
            [executable, "-lc", probe],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except Exception:
        return "native"
    value = completed.stdout.strip().splitlines()
    return value[-1] if value and value[-1] in ("msys", "wsl", "native") else "native"


def start_process(command, cwd, stdout_path, stderr_path):
    stdout = open(str(stdout_path), "w", encoding="utf-8")
    stderr = open(str(stderr_path), "w", encoding="utf-8")
    process = subprocess.Popen(
        command,
        cwd=str(cwd),
        stdout=stdout,
        stderr=stderr,
    )
    process._renderacre_logs = (stdout, stderr)
    return process


def stop_process(process):
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    for handle in getattr(process, "_renderacre_logs", ()):
        handle.close()


def wait_controller(port):
    deadline = time.time() + 30
    while time.time() < deadline:
        try:
            data = request_json("GET", "http://127.0.0.1:{0}/healthz".format(port))
            if data.get("status") == "ok":
                return
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("controller did not become ready on port {0}".format(port))


def submit_openjd(port, name, template_path, parameters):
    payload = {
        "name": name,
        "openjd": {
            "template_yaml": template_path.read_text(encoding="utf-8"),
            "parameters": parameters,
            "template_dir": str(template_path.parent),
            "current_working_dir": str(template_path.parent),
        },
    }
    return request_json(
        "POST",
        "http://127.0.0.1:{0}/v1/jobs".format(port),
        payload,
    )


def submit_job(port, payload):
    return request_json(
        "POST",
        "http://127.0.0.1:{0}/v1/jobs".format(port),
        payload,
    )


def wait_job(port, job_id, timeout=180):
    deadline = time.time() + timeout
    while time.time() < deadline:
        job = request_json("GET", "http://127.0.0.1:{0}/v1/jobs/{1}".format(port, job_id))
        if job.get("state") in ("succeeded", "failed", "cancelled"):
            return job
        time.sleep(0.5)
    raise RuntimeError("job {0} did not finish within {1}s".format(job_id, timeout))


def request_json(method, url, payload=None):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            body = response.read().decode("utf-8")
            return json.loads(body)
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError("{0} {1} failed: {2}".format(method, url, body))


def request_bytes(method, url):
    request = urllib.request.Request(url, method=method)
    with urllib.request.urlopen(request, timeout=10) as response:
        return response.read()


def run_command_job(port, run_dir, shell_selection):
    specs = resolve_shells(shell_selection)
    if not specs:
        raise FileNotFoundError("no command shells were found for selection: {0}".format(shell_selection))

    total_tasks = 0
    job_ids = []
    for spec in specs:
        output_dir = clean_dir(run_dir / "command-frames" / spec["name"])
        tasks = []
        expected_texts = []
        for frame in range(1, 4):
            command_text, output_file, expected_text = shell_frame_command(spec, output_dir, frame)
            expected_texts.append(expected_text)
            tasks.append(
                {
                    "name": "{0}-frame-{1:04d}".format(spec["name"], frame),
                    "command": {
                        "executable": spec["executable"],
                        "args": spec["args"] + [command_text],
                        "working_dir": str(output_dir),
                    },
                    "artifact_paths": [str(output_dir)],
                }
            )

        job = submit_job(
            port,
            {
                "name": "e2e-command-{0}-frames".format(spec["name"]),
                "tasks": tasks,
            },
        )
        final = require_success(port, job["id"], "command-{0}".format(spec["name"]))
        safe_name = spec["name"].replace("-", "_")
        require_files(output_dir, "{0}_frame_{{0:04d}}.txt".format(safe_name), range(1, 4))
        require_artifacts(port, final, min_count=3)
        require_artifact_texts(port, final, expected_texts)
        require_worker_logs(port, [expected_texts[0]])
        total_tasks += len(final["tasks"])
        job_ids.append(final["id"])

    return {"name": "command", "job_id": ",".join(job_ids), "tasks": total_tasks}


def shell_frame_command(spec, output_dir, frame):
    shell_name = spec["name"]
    safe_name = shell_name.replace("-", "_")
    output_file = output_dir / "{0}_frame_{1:04d}.txt".format(safe_name, frame)
    expected_text = "renderacre {0} frame {1}".format(shell_name, frame)
    artifact_line = "RENDERACRE_ARTIFACT={0}".format(str(output_file))

    if shell_name == "cmd":
        relative_file = output_file.name
        command = (
            "echo {text} > {file_path} & "
            "echo {artifact} & "
            "type {file_path}"
        ).format(
            text=expected_text,
            file_path=relative_file,
            artifact=artifact_line,
        )
        return command, output_file, expected_text

    if shell_name in ("powershell", "pwsh"):
        command = (
            "$path = {path}; "
            "New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null; "
            "Set-Content -LiteralPath $path -Value {text}; "
            "Write-Output {artifact}; "
            "Get-Content -LiteralPath $path"
        ).format(
            path=ps_quote(output_file),
            text=ps_quote(expected_text),
            artifact=ps_quote(artifact_line),
        )
        return command, output_file, expected_text

    if shell_name in ("bash", "sh"):
        shell_output_dir = posix_shell_path(output_dir, spec.get("path_style", "native"))
        shell_output_file = posix_shell_path(output_file, spec.get("path_style", "native"))
        command = (
            "mkdir -p {dir_q}; "
            "printf '%s\\n' {text_q} > {file_q}; "
            "echo {artifact_q}; "
            "cat {file_q}"
        ).format(
            dir_q=sh_quote(shell_output_dir),
            text_q=sh_quote(expected_text),
            file_q=sh_quote(shell_output_file),
            artifact_q=sh_quote(artifact_line),
        )
        return command, output_file, expected_text

    raise ValueError("unsupported shell: {0}".format(shell_name))


def ps_quote(value):
    return "'{0}'".format(str(value).replace("'", "''"))


def sh_quote(value):
    return "'{0}'".format(str(value).replace("'", "'\"'\"'"))


def posix_shell_path(path, style):
    text = str(path)
    if os.name == "nt" and len(text) > 1 and text[1] == ":":
        drive = text[0].lower()
        rest = text[2:].replace("\\", "/").lstrip("/")
        if style == "msys":
            return "/{0}/{1}".format(drive, rest)
        if style == "wsl":
            return "/mnt/{0}/{1}".format(drive, rest)
    if isinstance(path, Path):
        return path.as_posix()
    return text.replace("\\", "/")


def run_python_job(repo, port, run_dir, python_exe):
    executable = require_executable(python_exe, "Python")
    template = repo / "examples" / "openjd_python_frames.yaml"
    job = submit_openjd(
        port,
        "e2e-python-openjd",
        template,
        {
            "PythonExecutable": executable,
            "Message": "renderacre-ci",
        },
    )
    final = require_success(port, job["id"], "python")
    if len(final["tasks"]) != 5:
        raise AssertionError("python job expected 5 tasks, got {0}".format(len(final["tasks"])))
    require_worker_logs(port, ["renderacre-ci frame 1"])

    artifact_dir = clean_dir(run_dir / "python-artifacts")
    artifact_file = artifact_dir / "direct_artifact.txt"
    direct_code = "\n".join(
        [
            "from pathlib import Path",
            "import time",
            "path = Path({0!r})".format(str(artifact_file)),
            "path.parent.mkdir(parents=True, exist_ok=True)",
            "print('python direct artifact task starting', flush=True)",
            "path.write_text('renderacre artifact e2e\\n', encoding='utf-8')",
            "print(f'RENDERACRE_ARTIFACT={path}', flush=True)",
            "time.sleep(0.2)",
            "print('python direct artifact task done', flush=True)",
        ]
    )
    direct = submit_job(
        port,
        {
            "name": "e2e-python-direct-artifact",
            "tasks": [
                {
                    "name": "write-artifact",
                    "command": {
                        "executable": executable,
                        "args": ["-c", direct_code],
                    },
                    "artifact_paths": [str(artifact_dir)],
                }
            ],
        },
    )
    direct_final = require_success(port, direct["id"], "python-direct")
    require_artifacts(port, direct_final, min_count=1)
    require_worker_logs(port, ["python direct artifact task done"])
    dependency_final = run_dependency_job(port, executable, run_dir)
    return {
        "name": "python",
        "job_id": final["id"],
        "tasks": len(final["tasks"]) + len(direct_final["tasks"]) + len(dependency_final["tasks"]),
    }


def run_dependency_job(port, executable, run_dir):
    dependency_dir = clean_dir(run_dir / "python-dependencies")
    order_file = dependency_dir / "dependency_order.txt"

    def task_code(label, required):
        return "\n".join(
            [
                "from pathlib import Path",
                "path = Path({0!r})".format(str(order_file)),
                "path.parent.mkdir(parents=True, exist_ok=True)",
                "existing = path.read_text(encoding='utf-8').splitlines() if path.exists() else []",
                "required = {0!r}".format(required),
                "missing = [item for item in required if item not in existing]",
                "if missing: raise SystemExit('missing upstream tasks before {0}: ' + ', '.join(missing))".format(label),
                "with path.open('a', encoding='utf-8') as handle: handle.write({0!r} + '\\n')".format(label),
                "print('dependency task {0} done', flush=True)".format(label),
            ]
        )

    dependency = submit_job(
        port,
        {
            "name": "e2e-python-dependency-workflow",
            "tasks": [
                {
                    "name": "prepare",
                    "command": {"executable": executable, "args": ["-c", task_code("prepare", [])]},
                },
                {
                    "name": "cache",
                    "dependencies": ["prepare"],
                    "command": {"executable": executable, "args": ["-c", task_code("cache", ["prepare"])]},
                },
                {
                    "name": "render-a",
                    "dependencies": ["cache"],
                    "command": {"executable": executable, "args": ["-c", task_code("render-a", ["prepare", "cache"])]},
                },
                {
                    "name": "render-b",
                    "dependencies": ["cache"],
                    "command": {"executable": executable, "args": ["-c", task_code("render-b", ["prepare", "cache"])]},
                },
                {
                    "name": "publish",
                    "dependencies": ["render-a", "render-b"],
                    "command": {
                        "executable": executable,
                        "args": ["-c", task_code("publish", ["prepare", "cache", "render-a", "render-b"])],
                    },
                    "artifact_paths": [str(dependency_dir)],
                },
            ],
        },
    )
    final = require_success(port, dependency["id"], "python-dependency")
    require_dependency_graph(final)
    require_artifacts(port, final, min_count=1)
    require_worker_logs(port, ["dependency task publish done"])
    order = order_file.read_text(encoding="utf-8").splitlines()
    if order[-1] != "publish":
        raise AssertionError("publish should be the final dependency task, got order: {0}".format(order))
    return final


def run_blender_job(repo, port, run_dir, blender_exe):
    executable = require_executable(blender_exe, "Blender")
    output_dir = clean_dir(run_dir / "blender")
    template = repo / "examples" / "dcc" / "blender_render_openjd.yaml"
    job = submit_openjd(
        port,
        "e2e-blender-openjd",
        template,
        {
            "BlenderExecutable": executable,
            "ScriptPath": str((repo / "examples" / "dcc" / "blender_render_task.py").resolve()),
            "OutputDir": str(output_dir),
        },
    )
    final = require_success(port, job["id"], "blender", timeout=300)
    require_files(output_dir, "blender_frame_{0:04d}.png", range(1, 4))
    require_artifacts(port, final, min_count=3, png=True)
    require_worker_logs(port, ["RENDERACRE_ARTIFACT="])
    return {"name": "blender", "job_id": final["id"], "tasks": len(final["tasks"])}


def run_maya_job(repo, port, run_dir, maya_python):
    executable = require_executable(maya_python, "Maya Python")
    output_dir = clean_dir(run_dir / "maya")
    template = repo / "examples" / "dcc" / "maya_render_openjd.yaml"
    job = submit_openjd(
        port,
        "e2e-maya-openjd",
        template,
        {
            "MayaPython": executable,
            "ScriptPath": str((repo / "examples" / "dcc" / "maya_render_task.py").resolve()),
            "OutputDir": str(output_dir),
        },
    )
    final = require_success(port, job["id"], "maya", timeout=300)
    require_files(output_dir, "maya_frame_{0:04d}.ma", range(1, 4))
    return {"name": "maya", "job_id": final["id"], "tasks": len(final["tasks"])}


def require_success(port, job_id, label, timeout=180):
    final = wait_job(port, job_id, timeout=timeout)
    if final.get("state") != "succeeded":
        raise RuntimeError(
            "{0} job {1} ended as {2}\n{3}".format(
                label,
                job_id,
                final.get("state"),
                json.dumps(final, indent=2),
            )
        )
    return final


def require_worker_logs(port, expected_messages):
    snapshot = request_json("GET", "http://127.0.0.1:{0}/v1/dashboard".format(port))
    logs = snapshot.get("logs", [])
    worker_logs = [log for log in logs if log.get("source") == "worker"]
    if not worker_logs:
        raise AssertionError("expected worker logs in dashboard snapshot")
    messages = "\n".join(log.get("message", "") for log in worker_logs)
    missing = [message for message in expected_messages if message not in messages]
    if missing:
        raise AssertionError("missing expected worker log messages: {0}".format(", ".join(missing)))


def require_artifacts(port, job, min_count, png=False):
    artifacts = []
    for task in job.get("tasks", []):
        for index, artifact in enumerate(task.get("artifacts", [])):
            artifacts.append((task, index, artifact))
    if len(artifacts) < min_count:
        raise AssertionError(
            "expected at least {0} artifacts, got {1}: {2}".format(
                min_count,
                len(artifacts),
                json.dumps(job, indent=2),
            )
        )

    first_task, first_index, first_artifact = artifacts[0]
    data = request_bytes(
        "GET",
        "http://127.0.0.1:{0}/v1/tasks/{1}/artifacts/{2}".format(
            port,
            first_task["id"],
            first_index,
        ),
    )
    if not data:
        raise AssertionError("downloaded artifact was empty: {0}".format(first_artifact))
    if png:
        png_artifact = next(
            ((task, index, artifact) for task, index, artifact in artifacts if artifact.get("kind") == "image"),
            None,
        )
        if not png_artifact:
            raise AssertionError("expected at least one image artifact")
        task, index, artifact = png_artifact
        data = request_bytes(
            "GET",
            "http://127.0.0.1:{0}/v1/tasks/{1}/artifacts/{2}".format(port, task["id"], index),
        )
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            raise AssertionError("artifact is not a PNG: {0}".format(artifact))


def require_artifact_texts(port, job, expected_texts):
    downloaded = []
    for task in job.get("tasks", []):
        for index, _artifact in enumerate(task.get("artifacts", [])):
            data = request_bytes(
                "GET",
                "http://127.0.0.1:{0}/v1/tasks/{1}/artifacts/{2}".format(
                    port,
                    task["id"],
                    index,
                ),
            )
            downloaded.append(data.decode("utf-8", errors="replace"))

    content = "\n".join(downloaded)
    missing = [text for text in expected_texts if text not in content]
    if missing:
        raise AssertionError("missing expected artifact text: {0}".format(", ".join(missing)))


def require_dependency_graph(job):
    tasks = {task["name"]: task for task in job.get("tasks", [])}
    required = {"prepare", "cache", "render-a", "render-b", "publish"}
    if set(tasks) != required:
        raise AssertionError("dependency job tasks did not match: {0}".format(sorted(tasks)))
    expected = {
        "prepare": set(),
        "cache": {tasks["prepare"]["id"]},
        "render-a": {tasks["cache"]["id"]},
        "render-b": {tasks["cache"]["id"]},
        "publish": {tasks["render-a"]["id"], tasks["render-b"]["id"]},
    }
    for name, dependency_ids in expected.items():
        actual = set(tasks[name].get("dependencies", []))
        if actual != dependency_ids:
            raise AssertionError(
                "task {0} dependencies mismatch: expected {1}, got {2}".format(
                    name,
                    sorted(dependency_ids),
                    sorted(actual),
                )
            )


def clean_dir(path):
    root = Path(__file__).resolve().parents[1] / "target"
    path = path.resolve()
    if root.resolve() not in path.parents:
        raise RuntimeError("refusing to clean path outside target: {0}".format(path))
    if path.exists():
        shutil.rmtree(str(path))
    path.mkdir(parents=True)
    return path


def require_files(output_dir, pattern, frames):
    missing = []
    for frame in frames:
        path = output_dir / pattern.format(frame)
        if not path.exists() or path.stat().st_size <= 0:
            missing.append(str(path))
    if missing:
        raise AssertionError("missing expected output files: {0}".format(", ".join(missing)))


def dump_log(title, path):
    if not path.exists():
        return
    print("\n--- {0}: {1} ---".format(title, path))
    content = path.read_text(encoding="utf-8", errors="replace")
    print(content[-8000:])


if __name__ == "__main__":
    main()
