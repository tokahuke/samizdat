import os

from subprocess import Popen, PIPE
from typing import Any

from utils import shell


def _set_env(
    env: str,
    spec: str | dict[str, Any],
):
    match spec:
        case str():
            os.environ[env] = spec.strip()
        case {"run": script}:
            proc = Popen(
                [shell(), script.split("/")[-1]],
                stdout=PIPE,
                stderr=PIPE,
                cwd="./" + "/".join(script.split("/")[:-1]),
            )
            stdout, stderr = proc.communicate()
            if proc.returncode:
                raise Exception(
                    f"Script {script} raised status {proc.returncode}: {stderr.decode()}"
                )
            os.environ[env] = stdout.decode().strip()
        case _:
            raise Exception(f"bad format for env {env}: {spec}")


def set_env(spec: dict[str, Any]) -> None:
    for env, subspec in spec.items():
        _set_env(env, subspec)
