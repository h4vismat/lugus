"""Shared pinned dependency and typed failures for independent capabilities."""
YFINANCE_VERSION = '1.7.0'


class ProviderError(Exception):
    def __init__(self, kind, message, code=-32000, retry_after_seconds=None):
        super().__init__(message)
        self.kind = kind
        self.code = code
        self.retry_after_seconds = retry_after_seconds
