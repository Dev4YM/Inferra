from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Gemma4Model:
    name: str
    size: str
    context_window: str
    input_modes: tuple[str, ...]
    family: str
    variant: str
    local_weight: bool = True
    recommended_for: str = ""


# Source checked against https://registry.ollama.com/library/gemma4/tags on 2026-05-02.
GEMMA4_MODELS: tuple[Gemma4Model, ...] = (
    Gemma4Model("gemma4:latest", "9.6GB", "128K", ("text", "image"), "e4b", "alias", recommended_for="default alias"),
    Gemma4Model("gemma4:e2b", "7.2GB", "128K", ("text", "image"), "e2b", "alias", recommended_for="small local systems"),
    Gemma4Model("gemma4:e4b", "9.6GB", "128K", ("text", "image"), "e4b", "alias", recommended_for="balanced local default"),
    Gemma4Model("gemma4:26b", "18GB", "256K", ("text", "image"), "26b-a4b", "alias", recommended_for="workstations"),
    Gemma4Model("gemma4:31b", "20GB", "256K", ("text", "image"), "31b", "alias", recommended_for="high-memory workstations"),
    Gemma4Model("gemma4:e2b-it-q4_K_M", "7.2GB", "128K", ("text", "image"), "e2b", "q4_K_M"),
    Gemma4Model("gemma4:e2b-it-q8_0", "8.1GB", "128K", ("text", "image"), "e2b", "q8_0"),
    Gemma4Model("gemma4:e2b-it-bf16", "10GB", "128K", ("text", "image"), "e2b", "bf16"),
    Gemma4Model("gemma4:e2b-mlx-bf16", "10GB", "128K", ("text",), "e2b", "mlx-bf16"),
    Gemma4Model("gemma4:e2b-mxfp8", "7.9GB", "128K", ("text",), "e2b", "mxfp8"),
    Gemma4Model("gemma4:e2b-nvfp4", "7.1GB", "128K", ("text",), "e2b", "nvfp4"),
    Gemma4Model("gemma4:e4b-it-q4_K_M", "9.6GB", "128K", ("text", "image"), "e4b", "q4_K_M"),
    Gemma4Model("gemma4:e4b-it-q8_0", "12GB", "128K", ("text", "image"), "e4b", "q8_0"),
    Gemma4Model("gemma4:e4b-it-bf16", "16GB", "128K", ("text", "image"), "e4b", "bf16"),
    Gemma4Model("gemma4:e4b-mlx-bf16", "16GB", "128K", ("text",), "e4b", "mlx-bf16"),
    Gemma4Model("gemma4:e4b-mxfp8", "11GB", "128K", ("text",), "e4b", "mxfp8"),
    Gemma4Model("gemma4:e4b-nvfp4", "9.6GB", "128K", ("text",), "e4b", "nvfp4"),
    Gemma4Model("gemma4:26b-a4b-it-q4_K_M", "18GB", "256K", ("text", "image"), "26b-a4b", "q4_K_M"),
    Gemma4Model("gemma4:26b-a4b-it-q8_0", "28GB", "256K", ("text", "image"), "26b-a4b", "q8_0"),
    Gemma4Model("gemma4:26b-mlx-bf16", "52GB", "256K", ("text",), "26b-a4b", "mlx-bf16"),
    Gemma4Model("gemma4:26b-mxfp8", "27GB", "256K", ("text",), "26b-a4b", "mxfp8"),
    Gemma4Model("gemma4:26b-nvfp4", "17GB", "256K", ("text",), "26b-a4b", "nvfp4"),
    Gemma4Model("gemma4:31b-cloud", "cloud", "256K", ("text", "image"), "31b", "cloud", local_weight=False),
    Gemma4Model("gemma4:31b-it-q4_K_M", "20GB", "256K", ("text", "image"), "31b", "q4_K_M"),
    Gemma4Model("gemma4:31b-it-q8_0", "34GB", "256K", ("text", "image"), "31b", "q8_0"),
    Gemma4Model("gemma4:31b-it-bf16", "63GB", "256K", ("text", "image"), "31b", "bf16"),
    Gemma4Model("gemma4:31b-mlx-bf16", "63GB", "256K", ("text",), "31b", "mlx-bf16"),
    Gemma4Model("gemma4:31b-mxfp8", "32GB", "256K", ("text",), "31b", "mxfp8"),
    Gemma4Model("gemma4:31b-nvfp4", "20GB", "256K", ("text",), "31b", "nvfp4"),
)


def list_gemma4_models() -> list[Gemma4Model]:
    return list(GEMMA4_MODELS)


def gemma4_model(name: str) -> Gemma4Model | None:
    normalized = name.strip()
    return next((model for model in GEMMA4_MODELS if model.name == normalized), None)


def recommended_gemma4_model() -> Gemma4Model:
    model = gemma4_model("gemma4:e4b")
    if model is None:
        raise RuntimeError("Gemma 4 registry is missing the default model")
    return model
