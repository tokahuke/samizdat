import yaml
import os

from typing import Any

import builders
import exporters
import env
import postbuild


def main():
    with open("build.yaml") as f:
        buildspec: dict[str, Any] = yaml.load(f, Loader=yaml.SafeLoader)

    original_pwd = os.getcwd()
    output = f"{original_pwd}/dist/latest"

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

    print(f"cleaning output directory at {original_pwd}/dist")
    os.system(f"rm -rf {original_pwd}/dist")

    exporters.export(
        buildspec.get("project", "builder"),
        buildspec["exports"],
        output=output,
    )

    if "VERSION" in os.environ:
        tagged_dir = f"{original_pwd}/dist/{os.environ['VERSION']}"
        print(f"creating tagged version directory at {tagged_dir}")
        os.system(f"cp -r {output} {tagged_dir}")

    postbuild.run(f"{original_pwd}/dist")


if __name__ == "__main__":
    main()
