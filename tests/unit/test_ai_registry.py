from ai.registry import gemma4_model, list_gemma4_models, recommended_gemma4_model


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
    assert gemma4_model(model.name) == model
