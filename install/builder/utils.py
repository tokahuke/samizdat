import os


def shell() -> str:
    return os.environ.get("SHELL", "/bin/sh")
