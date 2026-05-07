"""Verified Gemma registry entries from https://ollama.com/library.

Verification date: 2026-05-05. Gemma 4 is published upstream; this module records live Ollama tags
from https://ollama.com/library/gemma3 and https://ollama.com/library/gemma4. Ollama exposes
relative release labels in the public library UI; `release_date` stores the page label observed on the
verification date.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class GemmaModel:
    name: str
    size: str
    context_window: str
    input_modes: tuple[str, ...]
    family: str
    variant: str
    quantization: str
    digest: str
    release_date: str
    local_weight: bool = True
    recommended_for: str = ""
    resolves_to: str | None = None
    forward_alias: bool = False


Gemma4Model = GemmaModel


GEMMA3_MODELS: tuple[GemmaModel, ...] = (
    GemmaModel(
        "gemma3:latest",
        "3.3GB",
        "128K",
        ("text", "image"),
        "4b",
        "alias",
        "q4_K_M",
        "a2af6cc3eb7f",
        "1 year ago",
        resolves_to="gemma3:4b-it-q4_K_M",
    ),
    GemmaModel("gemma3:270m", "292MB", "32K", ("text",), "270m", "alias", "q8_0", "e7d36fb2c3b3", "8 months ago", resolves_to="gemma3:270m-it-q8_0"),
    GemmaModel("gemma3:1b", "815MB", "32K", ("text",), "1b", "alias", "q4_K_M", "8648f39daa8f", "1 year ago", resolves_to="gemma3:1b-it-q4_K_M"),
    GemmaModel("gemma3:4b", "3.3GB", "128K", ("text", "image"), "4b", "alias", "q4_K_M", "a2af6cc3eb7f", "1 year ago", resolves_to="gemma3:4b-it-q4_K_M"),
    GemmaModel("gemma3:12b", "8.1GB", "128K", ("text", "image"), "12b", "alias", "q4_K_M", "f4031aab637d", "1 year ago", resolves_to="gemma3:12b-it-q4_K_M"),
    GemmaModel("gemma3:27b", "17GB", "128K", ("text", "image"), "27b", "alias", "q4_K_M", "a418f5838eaf", "1 year ago", resolves_to="gemma3:27b-it-q4_K_M"),
    GemmaModel("gemma3:270m-it-qat", "241MB", "32K", ("text",), "270m", "it", "qat", "b16d6d39dfbd", "8 months ago"),
    GemmaModel("gemma3:270m-it-q8_0", "292MB", "32K", ("text",), "270m", "it", "q8_0", "e7d36fb2c3b3", "8 months ago"),
    GemmaModel("gemma3:270m-it-fp16", "543MB", "32K", ("text",), "270m", "it", "fp16", "a44af03dd6b3", "8 months ago"),
    GemmaModel("gemma3:270m-it-bf16", "543MB", "32K", ("text",), "270m", "it", "bf16", "dc598b095ea6", "8 months ago"),
    GemmaModel("gemma3:1b-it-qat", "1.0GB", "32K", ("text",), "1b", "it", "qat", "b491bd3989c6", "1 year ago"),
    GemmaModel("gemma3:1b-it-q4_K_M", "815MB", "32K", ("text",), "1b", "it", "q4_K_M", "8648f39daa8f", "1 year ago"),
    GemmaModel("gemma3:1b-it-q8_0", "1.1GB", "32K", ("text",), "1b", "it", "q8_0", "0fdb9c7fefee", "1 year ago"),
    GemmaModel("gemma3:1b-it-fp16", "2.0GB", "32K", ("text",), "1b", "it", "fp16", "16d00907691b", "1 year ago"),
    GemmaModel("gemma3:4b-cloud", "cloud", "32K", ("text", "image"), "4b", "cloud", "cloud", "89c58fea5420", "4 months ago", local_weight=False),
    GemmaModel("gemma3:4b-it-qat", "4.0GB", "128K", ("text", "image"), "4b", "it", "qat", "d01ad0579247", "1 year ago"),
    GemmaModel("gemma3:4b-it-q4_K_M", "3.3GB", "128K", ("text", "image"), "4b", "it", "q4_K_M", "a2af6cc3eb7f", "1 year ago"),
    GemmaModel("gemma3:4b-it-q8_0", "5.0GB", "128K", ("text", "image"), "4b", "it", "q8_0", "2376388dec16", "1 year ago"),
    GemmaModel("gemma3:4b-it-fp16", "8.6GB", "128K", ("text", "image"), "4b", "it", "fp16", "c4da438ae756", "1 year ago"),
    GemmaModel("gemma3:12b-cloud", "cloud", "32K", ("text", "image"), "12b", "cloud", "cloud", "485e7119a53a", "4 months ago", local_weight=False),
    GemmaModel("gemma3:12b-it-qat", "8.9GB", "128K", ("text", "image"), "12b", "it", "qat", "5d4fa005e7bb", "1 year ago"),
    GemmaModel("gemma3:12b-it-q4_K_M", "8.1GB", "128K", ("text", "image"), "12b", "it", "q4_K_M", "f4031aab637d", "1 year ago"),
    GemmaModel("gemma3:12b-it-q8_0", "13GB", "128K", ("text", "image"), "12b", "it", "q8_0", "997a7c2c0975", "1 year ago"),
    GemmaModel("gemma3:12b-it-fp16", "24GB", "128K", ("text", "image"), "12b", "it", "fp16", "6b1ba564b78d", "1 year ago"),
    GemmaModel("gemma3:27b-cloud", "cloud", "128K", ("text", "image"), "27b", "cloud", "cloud", "9e1580299085", "4 months ago", local_weight=False),
    GemmaModel("gemma3:27b-it-qat", "18GB", "128K", ("text", "image"), "27b", "it", "qat", "29eb0b9aeda3", "1 year ago"),
    GemmaModel("gemma3:27b-it-q4_K_M", "17GB", "128K", ("text", "image"), "27b", "it", "q4_K_M", "a418f5838eaf", "1 year ago"),
    GemmaModel("gemma3:27b-it-q8_0", "30GB", "128K", ("text", "image"), "27b", "it", "q8_0", "273cbcd67032", "1 year ago"),
    GemmaModel("gemma3:27b-it-fp16", "55GB", "128K", ("text", "image"), "27b", "it", "fp16", "b7d58e2e179e", "1 year ago"),
)

GEMMA4_MODELS: tuple[GemmaModel, ...] = (
    GemmaModel(
        "gemma4:latest",
        "9.6GB",
        "128K",
        ("text", "image"),
        "e4b",
        "alias",
        "q4_K_M",
        "c6eb396dbd59",
        "1 month ago",
        recommended_for="default alias",
        resolves_to="gemma4:e4b-it-q4_K_M",
    ),
    GemmaModel(
        "gemma4:e2b",
        "7.2GB",
        "128K",
        ("text", "image"),
        "e2b",
        "alias",
        "q4_K_M",
        "7fbdbf8f5e45",
        "1 month ago",
        recommended_for="small local systems",
        resolves_to="gemma4:e2b-it-q4_K_M",
    ),
    GemmaModel(
        "gemma4:e4b",
        "9.6GB",
        "128K",
        ("text", "image"),
        "e4b",
        "alias",
        "q4_K_M",
        "c6eb396dbd59",
        "1 month ago",
        recommended_for="balanced local default",
        resolves_to="gemma4:e4b-it-q4_K_M",
    ),
    GemmaModel(
        "gemma4:26b",
        "18GB",
        "256K",
        ("text", "image"),
        "26b-a4b",
        "alias",
        "q4_K_M",
        "5571076f3d70",
        "1 month ago",
        recommended_for="workstations",
        resolves_to="gemma4:26b-a4b-it-q4_K_M",
    ),
    GemmaModel(
        "gemma4:31b",
        "20GB",
        "256K",
        ("text", "image"),
        "31b",
        "alias",
        "q4_K_M",
        "6316f0629137",
        "1 month ago",
        recommended_for="high-memory workstations",
        resolves_to="gemma4:31b-it-q4_K_M",
    ),
    GemmaModel("gemma4:e2b-it-q4_K_M", "7.2GB", "128K", ("text", "image"), "e2b", "it", "q4_K_M", "7fbdbf8f5e45", "1 month ago"),
    GemmaModel("gemma4:e2b-it-q8_0", "8.1GB", "128K", ("text", "image"), "e2b", "it", "q8_0", "95e5aad2e60a", "1 month ago"),
    GemmaModel("gemma4:e2b-it-bf16", "10GB", "128K", ("text", "image"), "e2b", "it", "bf16", "850bc7fea32f", "1 month ago"),
    GemmaModel("gemma4:e2b-mlx-bf16", "10GB", "128K", ("text",), "e2b", "mlx", "bf16", "3d4d7700cd44", "2 weeks ago"),
    GemmaModel("gemma4:e2b-mxfp8", "7.9GB", "128K", ("text",), "e2b", "mxfp8", "mxfp8", "4bb69c53aae4", "2 weeks ago"),
    GemmaModel("gemma4:e2b-nvfp4", "7.1GB", "128K", ("text",), "e2b", "nvfp4", "nvfp4", "c4e49a77005e", "2 weeks ago"),
    GemmaModel("gemma4:e4b-it-q4_K_M", "9.6GB", "128K", ("text", "image"), "e4b", "it", "q4_K_M", "c6eb396dbd59", "1 month ago"),
    GemmaModel("gemma4:e4b-it-q8_0", "12GB", "128K", ("text", "image"), "e4b", "it", "q8_0", "9dcc35808b42", "1 month ago"),
    GemmaModel("gemma4:e4b-it-bf16", "16GB", "128K", ("text", "image"), "e4b", "it", "bf16", "d0d10a1b1ddb", "1 month ago"),
    GemmaModel("gemma4:e4b-mlx-bf16", "16GB", "128K", ("text",), "e4b", "mlx", "bf16", "9bf9fe5d5c83", "2 weeks ago"),
    GemmaModel("gemma4:e4b-mxfp8", "11GB", "128K", ("text",), "e4b", "mxfp8", "mxfp8", "5fc736d1217f", "2 weeks ago"),
    GemmaModel("gemma4:e4b-nvfp4", "9.6GB", "128K", ("text",), "e4b", "nvfp4", "nvfp4", "64af8205368d", "2 weeks ago"),
    GemmaModel("gemma4:26b-a4b-it-q4_K_M", "18GB", "256K", ("text", "image"), "26b-a4b", "it", "q4_K_M", "5571076f3d70", "1 month ago"),
    GemmaModel("gemma4:26b-a4b-it-q8_0", "28GB", "256K", ("text", "image"), "26b-a4b", "it", "q8_0", "6bfaf9a8cb37", "1 month ago"),
    GemmaModel("gemma4:26b-mlx-bf16", "52GB", "256K", ("text",), "26b-a4b", "mlx", "bf16", "ab7de04c2599", "2 weeks ago"),
    GemmaModel("gemma4:26b-mxfp8", "27GB", "256K", ("text",), "26b-a4b", "mxfp8", "mxfp8", "3950c545841f", "2 weeks ago"),
    GemmaModel("gemma4:26b-nvfp4", "17GB", "256K", ("text",), "26b-a4b", "nvfp4", "nvfp4", "21c59a2eae30", "2 weeks ago"),
    GemmaModel("gemma4:31b-cloud", "cloud", "256K", ("text", "image"), "31b", "cloud", "cloud", "c382fbfbc73b", "4 weeks ago", local_weight=False),
    GemmaModel("gemma4:31b-it-q4_K_M", "20GB", "256K", ("text", "image"), "31b", "it", "q4_K_M", "6316f0629137", "1 month ago"),
    GemmaModel("gemma4:31b-it-q8_0", "34GB", "256K", ("text", "image"), "31b", "it", "q8_0", "53dd8459790f", "1 month ago"),
    GemmaModel("gemma4:31b-it-bf16", "63GB", "256K", ("text", "image"), "31b", "it", "bf16", "236d76ae0874", "1 month ago"),
    GemmaModel("gemma4:31b-mlx-bf16", "63GB", "256K", ("text",), "31b", "mlx", "bf16", "cd34f05c33e9", "2 weeks ago"),
    GemmaModel("gemma4:31b-mxfp8", "32GB", "256K", ("text",), "31b", "mxfp8", "mxfp8", "746b932fc925", "2 weeks ago"),
    GemmaModel("gemma4:31b-nvfp4", "20GB", "256K", ("text",), "31b", "nvfp4", "nvfp4", "700de81aa191", "2 weeks ago"),
)

GEMMA4_DEFAULT_ALIASES: dict[str, str] = {
    "gemma4": "gemma4:e4b",
    "gemma4:latest": "gemma4:e4b-it-q4_K_M",
    "gemma4:e2b": "gemma4:e2b-it-q4_K_M",
    "gemma4:e4b": "gemma4:e4b-it-q4_K_M",
    "gemma4:26b": "gemma4:26b-a4b-it-q4_K_M",
    "gemma4:31b": "gemma4:31b-it-q4_K_M",
}

GEMMA3_DEFAULT_ALIASES: dict[str, str] = {
    "gemma3": "gemma3:4b",
    "gemma3:latest": "gemma3:4b-it-q4_K_M",
    "gemma3:270m": "gemma3:270m-it-q8_0",
    "gemma3:1b": "gemma3:1b-it-q4_K_M",
    "gemma3:4b": "gemma3:4b-it-q4_K_M",
    "gemma3:12b": "gemma3:12b-it-q4_K_M",
    "gemma3:27b": "gemma3:27b-it-q4_K_M",
}


def list_gemma3_models() -> list[GemmaModel]:
    return list(GEMMA3_MODELS)


def list_gemma4_models() -> list[GemmaModel]:
    return list(GEMMA4_MODELS)


def list_gemma_models() -> list[GemmaModel]:
    return [*GEMMA3_MODELS, *GEMMA4_MODELS]


def gemma3_model(name: str) -> GemmaModel | None:
    normalized = name.strip()
    return next((model for model in GEMMA3_MODELS if model.name == normalized), None)


def gemma4_model(name: str) -> GemmaModel | None:
    normalized = name.strip()
    return next((model for model in GEMMA4_MODELS if model.name == normalized), None)


def resolve_gemma3_alias(name: str) -> str:
    normalized = name.strip()
    return GEMMA3_DEFAULT_ALIASES.get(normalized, normalized)


def resolve_gemma4_alias(name: str) -> str:
    normalized = name.strip()
    return GEMMA4_DEFAULT_ALIASES.get(normalized, normalized)


def resolve_gemma_model_alias(name: str) -> str:
    normalized = name.strip()
    if normalized.startswith("gemma4"):
        return resolve_gemma4_alias(normalized)
    if normalized.startswith("gemma3"):
        return resolve_gemma3_alias(normalized)
    return normalized


def choose_available_gemma_model(name: str, available_models: list[str] | tuple[str, ...]) -> str:
    normalized = name.strip()
    available = {item.strip() for item in available_models if item}
    if normalized in available:
        return normalized

    alias = resolve_gemma_model_alias(normalized)
    if alias in available:
        return alias

    model = gemma4_model(normalized) or gemma3_model(normalized)
    if model is not None and model.resolves_to and model.resolves_to in available:
        return model.resolves_to
    return normalized


def recommended_gemma4_model() -> GemmaModel:
    model = gemma4_model("gemma4:e4b")
    if model is None:
        raise RuntimeError("Gemma 4 registry is missing the default model")
    return model
