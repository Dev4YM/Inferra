from __future__ import annotations

import hashlib
from datetime import timedelta

from analysis.correlation import CorrelationEngine
from core.enums import IncidentState, Severity
from core.time import utc_now
from events.models import EventFilter, NormalizedEvent
from reasoning import SimpleHypothesisEngine
from runtime.service_graph import ServiceGraph
from storage.sqlite import SQLiteEventStore, SQLiteIncidentStore


class SimpleIncidentAnalyzer:
    """First-pass incident creation.

    This now uses the correlation layer, then persists incidents, clusters, and
    deterministic v0 hypotheses. Later reasoning modules can replace the simple
    hypothesis engine without changing the store/API shape.
    """

    def __init__(
        self,
        event_store: SQLiteEventStore,
        incident_store: SQLiteIncidentStore,
        service_graph: ServiceGraph | None = None,
    ) -> None:
        self.event_store = event_store
        self.incident_store = incident_store
        self.correlation = CorrelationEngine(service_graph)
        self.hypotheses = SimpleHypothesisEngine(service_graph)

    def analyze_recent(self, window_seconds: int = 60) -> int:
        end = utc_now()
        start = end - timedelta(seconds=window_seconds)
        filters = EventFilter(severities={Severity.WARN, Severity.ERROR, Severity.CRITICAL})
        events = list(self.event_store.query_time_range(start, end, filters=filters, limit=500))
        clusters = self.correlation.build_clusters(events)
        if not clusters:
            return 0
        events_by_id = {event.event_id: event for event in events}
        updated = 0
        for cluster in clusters:
            cluster_events = [events_by_id[event_id] for event_id in cluster.events if event_id in events_by_id]
            if not cluster_events:
                continue
            incident_id = self._incident_id(cluster)
            self.incident_store.upsert_incident(
                incident_id=incident_id,
                state=IncidentState.INVESTIGATING,
                severity=cluster.primary_severity,
                affected_services=cluster.affected_services,
                primary_service=sorted(cluster.affected_services)[0] if cluster.affected_services else None,
                time_range_start=cluster.time_range[0],
                time_range_end=cluster.time_range[1],
                event_ids=cluster.events,
            )
            self.incident_store.upsert_cluster(incident_id, cluster)
            self.incident_store.replace_hypotheses(incident_id, self.hypotheses.generate(incident_id, cluster_events))
            updated += 1
        return updated

    def _incident_id(self, cluster) -> str:
        bucket = int(cluster.time_range[0].timestamp() // 300)
        services = ",".join(sorted(cluster.affected_services))
        digest = hashlib.sha256(f"{services}|{bucket}".encode("utf-8")).hexdigest()[:16]
        return f"inc-{digest}"
