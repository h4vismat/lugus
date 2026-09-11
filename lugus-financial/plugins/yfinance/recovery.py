"""Retry decisions are pure; the synchronous transport edge owns time and cooldown.

Only fresh idempotent source reads enter this boundary, never cursor advancement.
The host's operation deadline remains the hard bound on a blocked dependency.
"""
import math
import random
import time

from common import ProviderError


def retry_delay(error, attempt, remaining, jitter):
    if error.kind not in ('timeout', 'unavailable') or attempt >= 2:
        return None
    delay = error.retry_after_seconds
    if delay is None:
        delay = 2 ** attempt + jitter
    # Leave a full source request timeout before the retry budget expires.
    return delay if 0 <= delay <= 10 and delay + 10 < remaining else None


class Recovery:
    def __init__(self, sleep=time.sleep, monotonic=time.monotonic,
                 jitter=lambda: random.uniform(0, 0.5)):
        self.sleep, self.monotonic, self.jitter = sleep, monotonic, jitter
        self.blocked_until = 0
        self.blocked_kind = 'unavailable'

    def run(self, operation):
        remaining = self.blocked_until - self.monotonic()
        if remaining > 0:
            raise ProviderError(self.blocked_kind, 'Source is cooling down; retry after the indicated delay',
                                retry_after_seconds=math.ceil(remaining))
        deadline = self.monotonic() + 35
        for attempt in range(3):
            try:
                return operation()
            except ProviderError as error:
                delay = retry_delay(error, attempt, deadline - self.monotonic(), self.jitter())
                if delay is not None:
                    self.sleep(delay)
                    if self.monotonic() + 10 < deadline:
                        continue
                if error.kind in ('rate_limited', 'timeout', 'unavailable'):
                    cooldown = max(error.retry_after_seconds or 0,
                                   60 if error.kind == 'rate_limited' else 5)
                    self.blocked_until = self.monotonic() + cooldown
                    self.blocked_kind = error.kind
                    raise ProviderError(error.kind, str(error), error.code, cooldown) from error
                raise
