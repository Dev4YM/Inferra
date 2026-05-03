import asyncio

from app import InferraRuntime
from config.model import InferraConfig, StorageConfig


def test_runtime_ingest_creates_simple_incident(tmp_path):
    async def run():
        config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
        runtime = InferraRuntime(config)
        await runtime.start()
        try:
            await runtime.ingest_payload('{"service":"api","level":"error","message":"timeout calling postgres"}')
            await runtime.ingest_payload('{"service":"api","level":"error","message":"connection refused from postgres"}')
            incidents = runtime.incident_store.list_active()
            assert len(incidents) == 1
            assert incidents[0].primary_service == "api"
        finally:
            await runtime.stop()

    asyncio.run(run())
