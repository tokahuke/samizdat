import yaml
import os

from typing import Any

import builders
import exporters
import env

def main():
    with open("build.yaml") as f:
        buildspec: dict[str, Any] = yaml.load(f, Loader=yaml.SafeLoader)

    original_pwd = os.getcwd()
    output = f"{original_pwd}/dist"

    if "root" in buildspec:
        os.chdir(buildspec["root"])

    env.set_env(buildspec["env"])

    builders.ensure_images(
        buildspec.get("project", "builder"),
        buildspec["images"],
        build=False,
    )
    builders.run_builders(
        buildspec.get("project", "builder"),
        buildspec["builders"],
        build=False,
    )

    exporters.export(
        buildspec.get("project", "builder"), buildspec["exports"], output=output
    )


if __name__ == "__main__":
    main()
