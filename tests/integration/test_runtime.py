import asyncio

from app import InferraRuntime
from config.model import InferraConfig, StorageConfig
from core.enums import IncidentState
from explanation.template import TemplateExplanationEngine
from web._shared import hypothesis_to_dict as _hypothesis_to_dict
from web._shared import incident_to_dict as _incident_to_dict


def test_runtime_ingest_creates_simple_incident(tmp_path):
    async def run():
        config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
        runtime = InferraRuntime(config)
        await runtime.start()
        try:
            await runtime.ingest_payload('{"service":"api","level":"error","message":"timeout calling postgres"}')
            await runtime.ingest_payload('{"service":"api","level":"error","message":"connection refused from postgres"}')
            incidents = runtime.incident_store.list_incidents(
                state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED]
            )
            assert len(incidents) == 1
            assert incidents[0].primary_service == "api"
        finally:
            await runtime.stop()

    asyncio.run(run())


def test_runtime_ingest_produces_template_explanation_payload(tmp_path):
    async def run():
        config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
        runtime = InferraRuntime(config)
        await runtime.start()
        try:
            await runtime.ingest_payload('{"service":"api","level":"error","message":"timeout calling postgres"}')
            await runtime.ingest_payload('{"service":"api","level":"error","message":"connection refused from postgres"}')
            incident = runtime.incident_store.list_incidents(
                state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
            )[0]
            hypotheses = runtime.incident_store.get_hypotheses(incident.incident_id)
            events = [
                runtime.event_store.get_event(event_id)
                for event_id in incident.events
                if runtime.event_store.get_event(event_id) is not None
            ]
            engine = TemplateExplanationEngine()
            explanation = engine.generate(
                _incident_to_dict(incident),
                [_hypothesis_to_dict(item) for item in hypotheses],
                events,
            )
            assert explanation.generation_model == "template_fallback"
            assert explanation.summary
        finally:
            await runtime.stop()

    asyncio.run(run())
