import asyncio

from collectors.file import FileCollector
from normalization.pipeline import NormalizationPipeline


def test_file_collector_merges_multiline_and_derives_service_from_filename(tmp_path):
    path = tmp_path / "api.log"
    path.write_text(
        "2026-05-04 First line\n"
        "trace line one\n"
        "trace line two\n"
        "2026-05-04 Second line\n",
        encoding="utf-8",
    )
    collector = FileCollector(
        path,
        service_id_from_filename=True,
        multiline_pattern=r"^\d{4}-\d{2}-\d{2}",
        poll_interval_seconds=0.01,
    )
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_existing(queue))
    first = queue.get_nowait()
    second = queue.get_nowait()
    normalized = NormalizationPipeline().normalize(first)

    assert emitted == 2
    assert "trace line one" in first.raw_payload
    assert second.raw_payload == "2026-05-04 Second line"
    assert normalized.service_id == "api"
