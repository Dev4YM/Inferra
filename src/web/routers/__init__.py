"""Domain-grouped FastAPI routers for the Inferra web control plane.

Each router owns one product domain. They keep endpoint paths stable; only
the file layout has changed since the original `web.api` monolith.
"""

from web.routers.ai import AiDeps, build_ai_router
from web.routers.collectors import CollectorsDeps, build_collectors_router
from web.routers.events import EventsDeps, build_events_router
from web.routers.incidents import IncidentsDeps, build_incidents_router
from web.routers.investigate import InvestigationDeps, build_investigation_router
from web.routers.services import ServicesDeps, build_services_router
from web.routers.topology import TopologyDeps, build_topology_router
from web.routers.workspace import WorkspaceDeps, build_workspace_router

__all__ = [
    "AiDeps",
    "CollectorsDeps",
    "EventsDeps",
    "IncidentsDeps",
    "InvestigationDeps",
    "ServicesDeps",
    "TopologyDeps",
    "WorkspaceDeps",
    "build_ai_router",
    "build_collectors_router",
    "build_events_router",
    "build_incidents_router",
    "build_investigation_router",
    "build_services_router",
    "build_topology_router",
    "build_workspace_router",
]
