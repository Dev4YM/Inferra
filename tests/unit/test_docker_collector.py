import asyncio

from collectors.docker import DockerCollector, DockerContainer


class FakeDockerApiClient:
    def __init__(self) -> None:
        self._containers = [
            DockerContainer(
                container_id="abc123",
                name="api",
                image="api:latest",
                labels={"app": "api", "tier": "backend"},
            )
        ]

    async def close(self) -> None:
        return None

    async def list_containers(self):
        return self._containers

    async def stream_events(self, since=None):
        yield {"id": "abc123", "status": "start", "Type": "container", "Actor": {"Attributes": {"name": "api"}}}

    async def stream_logs(self, container_id, since=None):
        yield "2026-05-04T10:00:00Z service started"


def test_docker_collector_streams_events_and_logs():
    async def run():
        collector = DockerCollector(
            include_all=True,
            include_labels=("app=api",),
            api_client=FakeDockerApiClient(),
        )
        queue = asyncio.Queue()
        task = asyncio.create_task(collector.run(queue))
        for _ in range(20):
            if queue.qsize() >= 1:
                break
            await asyncio.sleep(0.01)
        await collector.stop()
        await asyncio.gather(task, return_exceptions=True)
        return collector, queue.get_nowait()

    collector, first = asyncio.run(run())

    assert first.metadata["kind"] in {"event", "log"}
    assert collector._matches_container(FakeDockerApiClient()._containers[0]) is True
