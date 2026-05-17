import docker
import docker.errors
import json
import os

from typing import Any
from concurrent.futures import ThreadPoolExecutor
from threading import local


def dbg[T](x: T) -> T:
    print(x)
    return x


__thread_local = local()


def client() -> docker.DockerClient:
    initialized = getattr(__thread_local, "client", None)
    if initialized is None:
        setattr(__thread_local, "client", docker.from_env(timeout=2**16))
        return client()
    else:
        return initialized


def _ensure_image(name: str, spec: dict[str, Any], build: bool) -> None:
    def get() -> bool:
        try:
            client().images.get(name)
            print(f"image {name} found")
            return True
        except docker.errors.ImageNotFound:
            return False

    def create():
        print(f"building image {name}...")
        logs = client().api.build(**{**spec, "tag": name, "pull": True})

        for lines in logs:
            for line in lines.split(b"\r\n"):
                if line.strip() == b"":
                    continue
                log = json.loads(line)
                if "stream" in log:
                    print(log["stream"], end="")
                elif "status" in log:
                    print("Status:", log["status"])
        try:
            client().images.get(name)
        except docker.errors.ImageNotFound:
            raise Exception(f"build of image {name} failed. See logs.")

    def delete():
        print(f"removing image {name}...")
        client().images.get(name).remove(force=True)

    if get():
        if build:
            delete()
            create()
    else:
        create()


def ensure_images(
    project: str,
    images: dict[str, dict[str, Any]] | None,
    build: bool = False,
) -> None:
    if images is None:
        return

    with ThreadPoolExecutor() as exec:
        for _ in exec.map(
            lambda item: _ensure_image(f"{project}_{item[0]}", item[1], build),
            images.items(),
        ):
            pass


def _expand_volumes(spec: dict[str, Any]) -> dict[str, Any]:
    """Expand `~` and `$VAR` in volume source paths and ensure each
    host path exists on disk. docker-py errors confusingly if it does
    not (and the image's own copy would shadow the empty mount)."""
    raw = spec.get("volumes")
    if not raw:
        return spec
    expanded: dict[str, Any] = {}
    for src, mount in raw.items():
        host = os.path.expandvars(os.path.expanduser(src))
        os.makedirs(host, exist_ok=True)
        expanded[host] = mount
    return {**spec, "volumes": expanded}


def _run_builder(
    name: str,
    spec: dict[str, Any],
    env: dict[str, str],
    build: bool,
) -> None:
    spec = _expand_volumes(spec)

    def get() -> bool:
        try:
            container = client().containers.get(name)
            result = container.wait()
            status_code = result["StatusCode"]
            if status_code == 0:
                print(f"container {name} found")
                return True
            else:
                print(f"container {name} found, but exited with error")
                delete()
                return False
        except docker.errors.NotFound:
            return False

    def create():
        print(f"running container {name}...")

        container = client().containers.run(
            **{**spec, "name": name, "environment": env},
            detach=True,
        )

        for line in container.logs(stream=True):
            line: bytes
            print(line.decode(), end="")

        result = container.wait()
        status_code = result["StatusCode"]
        if status_code != 0:
            raise Exception(f"container {name} exited with status {status_code}")

    def delete():
        print(f"removing container {name}...")
        client().containers.get(name).remove(force=True)

    if get():
        if build:
            delete()
            create()
    else:
        create()


def run_builders(
    project: str,
    builders: dict[str, dict[str, Any]] | None,
    env: dict[str, str] | None = None,
    build: bool = False,
) -> None:
    if builders is None:
        return

    container_env = env or {}

    with ThreadPoolExecutor() as exec:
        for _ in exec.map(
            lambda item: _run_builder(
                f"{project}_{item[0]}",
                {**item[1], "image": f'{project}_{item[1]["image"]}'},
                container_env,
                build,
            ),
            builders.items(),
        ):
            pass
