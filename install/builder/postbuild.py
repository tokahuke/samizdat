import os

from subprocess import Popen, PIPE
from typing import Any

from utils import shell


def _run(cwd: str):
    proc = Popen([shell(), f"{os.getcwd()}/postbuild.sh"], cwd=cwd)
    proc.communicate()
    if proc.returncode:
        raise Exception(f"Post-build script raised status {proc.returncode}")


def run(cwd: str) -> None:
    if os.path.exists("postbuild.sh"):
        print("running post-build script")
        _run(cwd)
