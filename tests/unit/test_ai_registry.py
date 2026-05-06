from ai.registry import gemma3_model, gemma4_model, list_gemma3_models, list_gemma4_models, recommended_gemma4_model


def test_gemma3_registry_contains_official_tag_set():
    names = {model.name for model in list_gemma3_models()}

    assert len(names) == 29
    assert "gemma3:latest" in names
    assert "gemma3:270m-it-qat" in names
    assert "gemma3:4b-cloud" in names
    assert "gemma3:12b-it-q8_0" in names
    assert "gemma3:27b-it-fp16" in names
    assert gemma3_model("gemma3:latest").resolves_to == "gemma3:4b-it-q4_K_M"


def test_gemma4_registry_contains_official_tag_set():
    names = {model.name for model in list_gemma4_models()}

    assert len(names) == 29
    assert "gemma4:e2b" in names
    assert "gemma4:e4b" in names
    assert "gemma4:26b-a4b-it-q8_0" in names
    assert "gemma4:31b-cloud" in names
    assert "gemma4:31b-nvfp4" in names


def test_recommended_gemma4_model_is_balanced_local_default():
    model = recommended_gemma4_model()

    assert model.name == "gemma4:e4b"
    assert model.local_weight is True
    assert model.quantization == "q4_K_M"
    assert model.resolves_to == "gemma4:e4b-it-q4_K_M"
    assert gemma4_model(model.name) == model
