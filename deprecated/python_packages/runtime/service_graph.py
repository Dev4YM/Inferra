from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import json


@dataclass
class ServiceGraph:
    dependencies: dict[str, set[str]] = field(default_factory=dict)
    dependents: dict[str, set[str]] = field(default_factory=dict)
    colocated: dict[str, set[str]] = field(default_factory=dict)

    def add_relation(self, source: str, target: str, relation_type: str = "depends_on") -> None:
        source = source.strip().lower()
        target = target.strip().lower()
        if not source or not target or source == target:
            return
        self.dependencies.setdefault(source, set()).add(target)
        self.dependents.setdefault(target, set()).add(source)

    def add_colocation(self, left: str, right: str) -> None:
        left = left.strip().lower()
        right = right.strip().lower()
        if not left or not right or left == right:
            return
        self.colocated.setdefault(left, set()).add(right)
        self.colocated.setdefault(right, set()).add(left)

    def get_colocated(self, service_id: str) -> set[str]:
        return set(self.colocated.get(service_id, set()))

    def get_dependencies(self, service_id: str) -> set[str]:
        return set(self.dependencies.get(service_id, set()))

    def get_dependents(self, service_id: str) -> set[str]:
        return set(self.dependents.get(service_id, set()))

    def related_services(self, service_id: str) -> set[str]:
        related = {service_id} | self.get_dependencies(service_id) | self.get_dependents(service_id)
        related |= self.get_colocated(service_id)
        for dep in list(self.get_dependencies(service_id)):
            related |= self.get_dependencies(dep)
        return related

    def edges(self) -> list[dict[str, str]]:
        rows: list[dict[str, str]] = []
        for source, targets in sorted(self.dependencies.items()):
            for target in sorted(targets):
                rows.append({"source": source, "target": target, "relation_type": "depends_on"})
        return rows

    def shortest_path_length(self, source: str, target: str, max_depth: int = 3) -> int | None:
        if source == target:
            return 0
        seen = {source}
        frontier = [(source, 0)]
        while frontier:
            current, depth = frontier.pop(0)
            if depth >= max_depth:
                continue
            neighbors = self.get_dependencies(current) | self.get_dependents(current)
            for neighbor in neighbors:
                if neighbor == target:
                    return depth + 1
                if neighbor not in seen:
                    seen.add(neighbor)
                    frontier.append((neighbor, depth + 1))
        return None

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        data = {
            "dependencies": {key: sorted(value) for key, value in self.dependencies.items()},
            "colocated": {key: sorted(value) for key, value in self.colocated.items()},
        }
        path.write_text(json.dumps(data, indent=2, sort_keys=True), encoding="utf-8")

    @classmethod
    def load(cls, path: Path) -> "ServiceGraph":
        graph = cls()
        if not path.exists():
            return graph
        data = json.loads(path.read_text(encoding="utf-8"))
        for source, targets in data.get("dependencies", {}).items():
            for target in targets:
                graph.add_relation(source, target)
        for left, peers in data.get("colocated", {}).items():
            for right in peers:
                graph.add_colocation(left, right)
        return graph
