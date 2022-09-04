from __future__ import annotations

import json
import subprocess

from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import List, Dict


class PortBroker:
    def __init__(self) -> None:
        self.current = 45100
        self.ports: Dict[str, int] = {}
        self.rev_ports: Dict[str, int] = {}

    def port_for(self, interface_id: str) -> int:
        try:
            return self.ports[interface_id]
        except KeyError:
            self.ports[interface_id] = self.current
            self.rev_ports[self.current] = interface_id
            self.current += 1
            return self.ports[interface_id]

    def interface_for(self, port: int) -> str:
        return self.rev_ports[port]


PORT_BROKER = PortBroker()


@dataclass
class Node:
    node_id: str

    def port(self) -> int:
        return PORT_BROKER.port_for(self.node_id)

    def report_config(self) -> Dict:
        return {
            "http-port": self.port(),
        }

    def command(self, hubs: List[Hub]) -> List[str]:
        return [
            "cargo",
            "run",
            "--release",
            "--bin=samizdat-node",
            "--",
            f"--port={self.port()}",
            f"--data=data/{self.node_id}",
            "--hubs",
            *[hub.address() for hub in hubs],
        ]


@dataclass
class Hub:
    hub_id: str

    def direct_port(self) -> int:
        return PORT_BROKER.port_for(f"{self.hub_id}-direct")

    def reverse_port(self) -> int:
        return PORT_BROKER.port_for(f"{self.hub_id}-reverse")

    def http_port(self) -> int:
        return PORT_BROKER.port_for(f"{self.hub_id}-http")

    def address(self) -> str:
        return f"[::1]:{self.direct_port()}/{self.reverse_port()}"

    def report_config(self) -> Dict:
        return {
            "address": self.address(),
            "http-port": self.http_port(),
        }

    def command(self, hubs: List[Hub]) -> List[str]:
        return [
            "cargo",
            "run",
            "--release",
            "--bin=samizdat-hub",
            "--",
            f"--addresses={self.address()}",
            f"--data=data/{self.hub_id}",
            f"--http-port={self.http_port()}",
            "--partners",
            *[hub.address() for hub in hubs],
        ]


@dataclass
class Graph:
    nodes: List[str]
    hubs: List[str]
    connections: List[Tuple[str, str]]

    def run(self) -> None:
        assert len(set(self.nodes + self.hubs)) == len(self.nodes + self.hubs), \
            "Node and hub names have to be unique"
        
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

        node_commands = [
            node.command(connections.get(node_id, [])) for node_id, node in nodes.items()
        ]
        hub_commands = [
            hub.command(connections.get(hub_id, [])) for hub_id, hub in hubs.items()
        ]

        report_config = {
            **{
                node_id: node.report_config() for node_id, node in nodes.items()
            },
            **{
                hub_id: hub.report_config() for hub_id, hub in hubs.items()
            },
        }

        print("Configuration:", json.dumps(report_config, indent=4, ensure_ascii=False))
        input("Press any key to continue...")

        subprocesses = [
            subprocess.Popen(command) for command in node_commands + hub_commands
        ]

        try:
            for process in subprocesses:
                process.wait()
        finally:
            print("Sending SIGTERM to all child processes...")
            
            for process in subprocesses:
                process.terminate()
                process.wait()
            
            print("... all processes terminated")


if __name__ == "__main__":
    with open("network.json") as network_desc:
        graph = Graph(**json.load(network_desc))

    graph.run()
