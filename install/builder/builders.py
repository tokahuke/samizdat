import docker
import docker.errors

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
        _, logs = client().images.build(**{**spec, "tag": name, "pull": True})
        for line in logs:
            print(line)

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
    images: dict[str, dict[str, Any]],
    build: bool = False,
) -> None:
    with ThreadPoolExecutor() as exec:
        for _ in exec.map(
            lambda item: _ensure_image(f"{project}_{item[0]}", item[1], build),
            images.items(),
        ):
            pass


def _run_builder(name: str, spec: dict[str, Any], build: bool) -> None:
    def get() -> bool:
        try:
            client().containers.get(name)
            print(f"container {name} found")
            return True
        except docker.errors.NotFound:
            return False

    def create():
        print(f"running container {name}...")
        container = client().containers.run(**{**spec, "name": name}, detach=True)
        for line in container.logs(stream=True):
            line: bytes
            print(line.decode(), end="")

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
    builders: dict[str, dict[str, Any]],
    build: bool = False,
) -> None:
    with ThreadPoolExecutor() as exec:
        for _ in exec.map(
            lambda item: _run_builder(
                f"{project}_{item[0]}",
                {**item[1], "image": f'{project}_{item[1]["image"]}'},
                build,
            ),
            builders.items(),
        ):
            pass
