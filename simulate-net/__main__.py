from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import traceback
from pathlib import Path

import ryan  # pip install ryan-lang

from dataclasses import dataclass
from time import sleep

from json import JSONEncoder

# Run paths are anchored to the repo root (parent of `simulate-net/`) so the
# script works regardless of where the user runs it from.
WORKSPACE_ROOT = Path(__file__).resolve().parent.parent


def log[T](x: T) -> T:
    print(x)
    return x


def wrapped_default(self, obj):
    return getattr(obj.__class__, "__json__", wrapped_default.default)(obj)


wrapped_default.default = JSONEncoder().default
JSONEncoder.default = wrapped_default  # type:ignore


IS_RELEASE = False


def folder():
    if IS_RELEASE:
        return "release"
    else:
        return "debug"


def opt_flag():
    if IS_RELEASE:
        return ["--release"]
    else:
        return []


def wait_all(processes: list[subprocess.Popen]) -> None:
    for process in processes:
        process.wait()


def _grab_free_port() -> int:
    """Asks the OS for a currently-free port on the loopback interface.
    There is a small race window between this call and the actual bind by
    the spawned process, but it is good enough for local simulation."""
    with socket.socket(socket.AF_INET6, socket.SOCK_STREAM) as s:
        s.bind(("::1", 0))
        return s.getsockname()[1]


class PortBroker:
    def __init__(self) -> None:
        self.ports: dict[str, int] = {}
        self.rev_ports: dict[int, str] = {}

    def port_for(self, interface_id: str) -> int:
        try:
            return self.ports[interface_id]
        except KeyError:
            port = _grab_free_port()
            self.ports[interface_id] = port
            self.rev_ports[port] = interface_id
            return port

    def interface_for(self, port: int) -> str:
        return self.rev_ports[port]


PORT_BROKER = PortBroker()


@dataclass
class Node:
    node_id: str

    def __json__(self) -> str:
        return self.node_id

    def port(self) -> int:
        return PORT_BROKER.port_for(self.node_id)

    def report_config(self) -> dict:
        return {
            "http-port": self.port(),
        }

    def command(self) -> list[str]:
        return [
            str(WORKSPACE_ROOT / f"target/{folder()}/samizdat-node"),
            f"--port={self.port()}",
            f"--data={WORKSPACE_ROOT}/data/{self.node_id}",
        ]

    def connect_to_hub(self, hub: Hub) -> list[str]:
        return log(
            [
                str(WORKSPACE_ROOT / f"target/{folder()}/samizdat"),
                f"--data={WORKSPACE_ROOT}/data/{self.node_id}",
                "hub",
                "new",
                hub.address(),
                "EnsureIpv6",
            ]
        )


@dataclass
class Hub:
    hub_id: str

    def __json__(self) -> str:
        return self.hub_id

    def direct_port(self) -> int:
        return PORT_BROKER.port_for(f"{self.hub_id}")

    def http_port(self) -> int:
        return PORT_BROKER.port_for(f"{self.hub_id}-http")

    def address(self) -> str:
        return f"[::1]:{self.direct_port()}"

    def report_config(self) -> dict:
        return {
            "address": self.address(),
            "http-port": self.http_port(),
        }

    def command(self, hubs: list[Hub]) -> list[str]:
        return [
            str(WORKSPACE_ROOT / f"target/{folder()}/samizdat-hub"),
            f"--addresses={self.address()}",
            f"--data={WORKSPACE_ROOT}/data/{self.hub_id}",
            f"--http-port={self.http_port()}",
            "--partners",
            *[hub.address() for hub in hubs],
        ]


@dataclass
class Graph:
    nodes: list[str]
    hubs: list[str]
    connections: list[tuple[str, str]]

    def run(self) -> None:
        assert len(set(self.nodes + self.hubs)) == len(
            self.nodes + self.hubs
        ), "Node and hub names have to be unique"

        nodes = {node_id: Node(node_id) for node_id in self.nodes}
        hubs = {hub_id: Hub(hub_id) for hub_id in self.hubs}
        connections = {}

        for origin, dest in self.connections:
            assert origin in nodes or origin in hubs, f"No such node or hub {origin!r}"

            try:
                dest_hub = hubs[dest]
            except KeyError:
                raise ValueError(f"No such hub {dest!r}")

            connections.setdefault(origin, []).append(dest_hub)

        node_commands = [node.command() for node in nodes.values()]
        hub_commands = [
            hub.command(connections.get(hub_id, [])) for hub_id, hub in hubs.items()
        ]

        report_config = {
            **{node_id: node.report_config() for node_id, node in nodes.items()},
            **{hub_id: hub.report_config() for hub_id, hub in hubs.items()},
            "connections": connections,
        }

        print("Configuration:", json.dumps(report_config, indent=4, ensure_ascii=False))
        if sys.stdin.isatty() and not os.environ.get("SAMIZDAT_SIM_NONINTERACTIVE"):
            input("Press any key to continue...")

        # Compile each binary, failing fast if any build errors out.
        build_processes = [
            subprocess.Popen(
                ["cargo", "build", *opt_flag(), "--bin", "samizdat-node"],
                cwd=WORKSPACE_ROOT,
            ),
            subprocess.Popen(
                ["cargo", "build", *opt_flag(), "--bin", "samizdat-hub"],
                cwd=WORKSPACE_ROOT,
            ),
            subprocess.Popen(
                ["cargo", "build", *opt_flag(), "--bin", "samizdat"],
                cwd=WORKSPACE_ROOT,
            ),
        ]
        for proc in build_processes:
            if proc.wait() != 0:
                raise RuntimeError(
                    f"cargo build failed (exit {proc.returncode}); aborting simulation"
                )

        # Launch processes:
        subprocesses = [
            subprocess.Popen(command) for command in node_commands + hub_commands
        ]
        sleep(1.0)

        # Set connections up
        wait_all(
            [
                subprocess.Popen(nodes[origin].connect_to_hub(hubs[dest]))
                for origin, dest in self.connections
                if origin in nodes
            ]
        )

        try:
            wait_all(subprocesses)
        except KeyboardInterrupt:
            pass
        except Exception as e:
            traceback.print_exc()
        finally:
            print("Sending SIGTERM to all child processes...")

            for process in subprocesses:
                process.terminate()
                process.wait()

            print("... all processes terminated")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Run a local Samizdat network simulation."
    )
    parser.add_argument(
        "--config",
        default=str(WORKSPACE_ROOT / "network.ryan"),
        help="Path to the .ryan network description (default: network.ryan in repo root).",
    )
    parser.add_argument(
        "--release",
        action="store_true",
        help="Compile and run with --release (slower build, faster runtime).",
    )
    args = parser.parse_args()

    if args.release:
        IS_RELEASE = True

    with open(args.config) as network_desc:
        config = ryan.from_str(network_desc.read())  # type: ignore
        graph = Graph(**config["graph"])

    graph.run()
