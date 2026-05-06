from __future__ import annotations

import json
import shutil
import subprocess
import threading
from pathlib import Path
from typing import Any

import networkx as nx

from config.loader import load_config


class ServiceGraphCache:
    def __init__(
        self,
        persist_path: str | Path = "./data/service_graph.json",
        *,
        config: Any | None = None,
        discover_docker: bool = True,
        log_pattern_inference: bool = False,
        auto_persist: bool = True,
        max_nodes: int = 500,
        max_edges: int = 5000,
    ) -> None:
        self.graph = nx.DiGraph()
        self._dirty = False
        self._persist_path = Path(persist_path)
        self._lock = threading.RLock()
        self._config = config or load_config()
        self._discover_docker = discover_docker
        self._log_pattern_inference = log_pattern_inference
        self._auto_persist = auto_persist
        self._max_nodes = max_nodes
        self._max_edges = max_edges
        self._edge_sequence = 0
        self.load()

    def add_relation(
        self,
        source: str,
        target: str,
        relation_type: str,
        origin: str = "config",
        confidence: str = "high",
    ) -> None:
        source = self._normalize_service(source)
        target = self._normalize_service(target)
        relation_type = relation_type.strip().lower() or "depends_on"
        if not source or not target or source == target:
            return
        with self._lock:
            if not self._can_add_nodes({source, target}):
                return
            self.graph.add_node(source)
            self.graph.add_node(target)
            self._edge_sequence += 1
            self.graph.add_edge(
                source,
                target,
                relation_type=relation_type,
                origin=origin,
                confidence=confidence,
                created_seq=self._edge_sequence,
            )
            self._trim_edges_if_needed()
            self._dirty = True
        if self._auto_persist:
            self.persist()

    def get_dependencies(self, service_id: str) -> list[str]:
        service_id = self._normalize_service(service_id)
        with self._lock:
            if not self.graph.has_node(service_id):
                return []
            return sorted(self._dependency_graph().successors(service_id))

    def get_dependents(self, service_id: str) -> list[str]:
        service_id = self._normalize_service(service_id)
        with self._lock:
            if not self.graph.has_node(service_id):
                return []
            return sorted(self._dependency_graph().predecessors(service_id))

    def get_colocated(self, service_id: str) -> list[str]:
        service_id = self._normalize_service(service_id)
        matches: set[str] = set()
        with self._lock:
            if not self.graph.has_node(service_id):
                return []
            for neighbor in self.graph.successors(service_id):
                if self.graph[service_id][neighbor].get("relation_type") in {"colocated_with", "shares_host"}:
                    matches.add(neighbor)
            for neighbor in self.graph.predecessors(service_id):
                if self.graph[neighbor][service_id].get("relation_type") in {"colocated_with", "shares_host"}:
                    matches.add(neighbor)
        return sorted(matches)

    def shortest_path(self, source: str, target: str) -> list[str] | None:
        source = self._normalize_service(source)
        target = self._normalize_service(target)
        with self._lock:
            dependency_graph = self._dependency_graph()
            if not (dependency_graph.has_node(source) and dependency_graph.has_node(target)):
                return None
            try:
                return nx.shortest_path(dependency_graph.to_undirected(), source, target)
            except nx.NetworkXNoPath:
                return None

    def shortest_path_length(self, source: str, target: str) -> int | None:
        path = self.shortest_path(source, target)
        if path is None:
            return None
        return max(0, len(path) - 1)

    def subgraph_around(self, service_id: str, depth: int = 2) -> nx.DiGraph:
        service_id = self._normalize_service(service_id)
        with self._lock:
            if not self.graph.has_node(service_id):
                return nx.DiGraph()
            nodes = nx.single_source_shortest_path_length(
                self.graph.to_undirected(),
                service_id,
                cutoff=max(0, depth),
            ).keys()
            return self.graph.subgraph(nodes).copy()

    def persist(self) -> None:
        with self._lock:
            if not self._dirty:
                return
            self._persist_path.parent.mkdir(parents=True, exist_ok=True)
            data = nx.node_link_data(self.graph)
            temp_path = self._persist_path.with_suffix(self._persist_path.suffix + ".tmp")
            temp_path.write_text(json.dumps(data, indent=2, sort_keys=True), encoding="utf-8")
            temp_path.replace(self._persist_path)
            self._dirty = False

    def load(self) -> None:
        with self._lock:
            self.graph = nx.DiGraph()
            if self._persist_path.exists():
                data = json.loads(self._persist_path.read_text(encoding="utf-8"))
                self.graph = nx.node_link_graph(data)
                self._edge_sequence = max(
                    [int(attrs.get("created_seq", 0)) for _, _, attrs in self.graph.edges(data=True)] or [0]
                )
            self._merge_config_edges()
            if self._discover_docker:
                self._merge_docker_edges()
            self._dirty = False

    def edges(self) -> list[dict[str, str]]:
        with self._lock:
            return [
                {
                    "source": source,
                    "target": target,
                    "relation_type": attrs.get("relation_type", "depends_on"),
                    "origin": attrs.get("origin", "config"),
                    "confidence": attrs.get("confidence", "high"),
                }
                for source, target, attrs in self.graph.edges(data=True)
            ]

    def _merge_config_edges(self) -> None:
        for edge in getattr(getattr(self._config, "topology", None), "edges", []):
            self._add_startup_edge(edge.source, edge.target, edge.type, origin="config", confidence="high")

    def _merge_docker_edges(self) -> None:
        for edge in self._discover_docker_edges():
            self._add_startup_edge(
                edge["source"],
                edge["target"],
                edge["relation_type"],
                origin="docker_compose",
                confidence="medium",
            )

    def _add_startup_edge(
        self,
        source: str,
        target: str,
        relation_type: str,
        *,
        origin: str,
        confidence: str,
    ) -> None:
        source = self._normalize_service(source)
        target = self._normalize_service(target)
        if not source or not target or source == target or not self._can_add_nodes({source, target}):
            return
        self.graph.add_node(source)
        self.graph.add_node(target)
        self._edge_sequence += 1
        self.graph.add_edge(
            source,
            target,
            relation_type=relation_type,
            origin=origin,
            confidence=confidence,
            created_seq=self._edge_sequence,
        )
        self._trim_edges_if_needed()

    def _discover_docker_edges(self) -> list[dict[str, str]]:
        if shutil.which("docker") is None:
            return []
        try:
            ps = subprocess.run(
                ["docker", "ps", "-q"],
                check=True,
                capture_output=True,
                text=True,
                timeout=2,
            )
        except (OSError, subprocess.SubprocessError):
            return []
        container_ids = [line.strip() for line in ps.stdout.splitlines() if line.strip()]
        if not container_ids:
            return []
        try:
            inspect = subprocess.run(
                ["docker", "inspect", *container_ids],
                check=True,
                capture_output=True,
                text=True,
                timeout=3,
            )
            payload = json.loads(inspect.stdout)
        except (OSError, ValueError, subprocess.SubprocessError):
            return []

        edges: list[dict[str, str]] = []
        by_project: dict[str, set[str]] = {}
        for item in payload:
            config = item.get("Config") or {}
            labels = config.get("Labels") or {}
            service = self._normalize_service(labels.get("com.docker.compose.service") or item.get("Name", "").lstrip("/"))
            project = labels.get("com.docker.compose.project")
            if not service:
                continue
            if project:
                by_project.setdefault(project, set()).add(service)
            depends_on = labels.get("com.docker.compose.depends_on")
            if depends_on:
                try:
                    decoded = json.loads(depends_on)
                    for dependency in decoded:
                        dep_service = self._normalize_service(dependency)
                        if dep_service:
                            edges.append(
                                {"source": service, "target": dep_service, "relation_type": "depends_on"}
                            )
                except ValueError:
                    continue

        for services in by_project.values():
            ordered = sorted(services)
            for index, source in enumerate(ordered):
                for target in ordered[index + 1 :]:
                    edges.append({"source": source, "target": target, "relation_type": "colocated_with"})
                    edges.append({"source": target, "target": source, "relation_type": "colocated_with"})
        return edges

    def _trim_edges_if_needed(self) -> None:
        while self.graph.number_of_edges() > self._max_edges:
            oldest = min(
                self.graph.edges(data=True),
                key=lambda item: int(item[2].get("created_seq", 0)),
            )
            self.graph.remove_edge(oldest[0], oldest[1])

    def _can_add_nodes(self, candidates: set[str]) -> bool:
        current_nodes = self.graph.number_of_nodes()
        missing = sum(1 for item in candidates if item not in self.graph)
        return current_nodes + missing <= self._max_nodes

    def _dependency_graph(self) -> nx.DiGraph:
        dependency_graph = nx.DiGraph()
        dependency_graph.add_nodes_from(self.graph.nodes(data=True))
        for source, target, attrs in self.graph.edges(data=True):
            if attrs.get("relation_type") in {"colocated_with", "shares_host"}:
                continue
            dependency_graph.add_edge(source, target, **attrs)
        return dependency_graph

    def _normalize_service(self, service_id: str) -> str:
        return service_id.strip().lower()
