import docker
import docker.errors
import os
import tarfile

from subprocess import Popen, PIPE
from typing import Any, Iterator, IO
from threading import local
from io import BytesIO

import builders


def dbg[T](x: T) -> T:
    print(x)
    return x


def _export(
    project: str,
    path: list[str],
    spec: dict[str, Any],
) -> Iterator[tuple[list[str], bytes]]:
    match spec:
        case None:
            with open("/".join(path), "rb") as f:
                yield path, f.read()
        case {"run": script}:
            proc = Popen(
                [os.environ["SHELL"], script.split("/")[-1]],
                stdout=PIPE,
                stderr=PIPE,
                cwd="./" + "/".join(script.split("/")[:-1]),
            )
            stdout, stderr = proc.communicate()
            if proc.returncode:
                raise Exception(
                    f"Script {script} raised status {proc.returncode}: {stderr.decode()}"
                )
            yield path, stdout
        case {"from": builder, "import": resource}:
            stream, stat = (
                builders.client()
                .containers.get(f"{project}_{builder}")
                .get_archive(resource)
            )
            with tarfile.open(fileobj=BytesIO(b''.join(stream)), mode='r') as tar:
                buf = tar.extractfile(resource.split("/")[-1])
                assert buf is not None
                yield path, buf.read()
        case _:
            for name, subspec in spec.items():
                yield from _export(project, [*path, name], subspec)


def export(project: str, spec: dict[str, Any], output: str = "./dist"):
    output = output if output.endswith("/") else output + "/"

    for subpath, contents in _export(project, [], spec):
        folder = output + "/".join(subpath[:-1])
        os.makedirs(folder, exist_ok=True)

        raw = output + "/".join(subpath)
        print(f"saving to {raw}")
        with open(raw, "wb") as f:
            f.write(contents)
