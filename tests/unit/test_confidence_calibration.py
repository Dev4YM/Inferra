from reasoning.calibration import ConfidenceCalibrator, ConfidenceInput


def test_confidence_calibrator_high_requires_strong_score_and_clean_evidence():
    label = ConfidenceCalibrator().label(
        ConfidenceInput(score=0.86, supporting_count=4, contradiction_count=0, dependency_proximity=1.0)
    )

    assert label == "high"


def test_confidence_calibrator_downgrades_sparse_or_contradicted_cases():
    calibrator = ConfidenceCalibrator()

    sparse = calibrator.label(
        ConfidenceInput(score=0.72, supporting_count=1, contradiction_count=0, dependency_proximity=0.25)
    )
    contradicted = calibrator.label(
        ConfidenceInput(score=0.86, supporting_count=4, contradiction_count=2, dependency_proximity=1.0)
    )

    assert sparse == "low"
    assert contradicted == "medium"
