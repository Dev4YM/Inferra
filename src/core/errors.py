class InferraError(Exception):
    """Base exception for Inferra."""


class ConfigError(InferraError):
    """Raised when configuration cannot be loaded or validated."""


class StorageError(InferraError):
    """Raised for storage initialization and persistence errors."""
