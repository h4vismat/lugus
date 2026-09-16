"""EdgarTools HTTP edge; Lugus owns decoding, retries, bounds and provenance.

Use the library's reusable SEC client, not its decoded JSON/DataFrames: converting
source numbers to floats or re-encoding documents would lose evidence fidelity.
The small urllib response bridge keeps the existing transport policy independent
of the third-party client. No EdgarTools retry decorators are used.
"""
import atexit
from contextlib import contextmanager, redirect_stderr, redirect_stdout
from email.message import Message
from functools import lru_cache
from importlib.metadata import version
import io
import os
import ssl
from tempfile import TemporaryDirectory
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urljoin

from provider import ProviderError, validate_url


@lru_cache(maxsize=1)
def client_manager():
    """One uncached, connection-pooled EdgarTools client per plugin process."""
    try:
        if version('edgartools') != '5.57.0':
            raise ValueError('Unsupported EdgarTools version')
        # Importing EdgarTools creates its default cache directory. Isolate that
        # side effect; source evidence and freshness belong to the Lugus host.
        temporary = TemporaryDirectory(prefix='lugus-edgar-')
        previous = os.environ.get('EDGAR_LOCAL_DATA_DIR')
        os.environ['EDGAR_LOCAL_DATA_DIR'] = temporary.name
        try:
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                from edgar.httpclient import get_http_mgr
                manager = get_http_mgr(cache_enabled=False, request_per_sec_limit=2)
        finally:
            if previous is None:
                os.environ.pop('EDGAR_LOCAL_DATA_DIR', None)
            else:
                os.environ['EDGAR_LOCAL_DATA_DIR'] = previous
        # Never prompt for EDGAR_IDENTITY or inherit an unrelated identity.
        # Every request supplies the validated initialize User-Agent explicitly.
        manager.user_agent_factory = None
        manager.httpx_params.update(verify=True, http2=False, follow_redirects=False)
        atexit.register(temporary.cleanup)
        atexit.register(manager.close)
        return manager
    except (ImportError, OSError, ValueError) as exc:
        raise ProviderError('configuration',
                            'Install the SEC plugin requirements with its configured Python interpreter') from exc


@contextmanager
def edgar_client(identity):
    with client_manager().http_client() as client:
        yield client


class StreamResponse:
    """Expose bounded raw reads without text decoding or decompression."""
    def __init__(self, response):
        self.response = response
        self.headers = Message()
        for key, value in response.headers.multi_items():
            self.headers[key] = value
        encoding = response.headers.get('content-encoding', 'identity').lower()
        if encoding != 'identity':
            raise ProviderError('malformed_data', 'Unexpected source content encoding')
        # Return each socket chunk immediately. Asking HTTPX to fill a chunk
        # would hide slow trickles from the caller's overall deadline check.
        self.chunks = response.iter_raw()
        self.pending = b''

    def geturl(self):
        return str(self.response.url)

    def read(self, size):
        if not self.pending:
            self.pending = next(self.chunks, b'')
        result, self.pending = self.pending[:size], self.pending[size:]
        return result


class EdgarOpener:
    def __init__(self, archive, before_request, monotonic=time.monotonic,
                 client_factory=edgar_client):
        self.archive, self.before_request = archive, before_request
        self.monotonic, self.client_factory = monotonic, client_factory

    @contextmanager
    def open(self, request, timeout):
        # Dependency loading is lazy so protocol validation works even when the
        # environment needs repair. Missing dependencies are configuration errors.
        try:
            import httpx
        except ImportError as exc:
            raise ProviderError('configuration', 'Install the SEC plugin requirements') from exc
        url = request.full_url
        deadline = None if timeout is None else self.monotonic() + timeout
        try:
            with self.client_factory(request.get_header('User-agent')) as client:
                if not isinstance(client, httpx.Client):
                    raise ProviderError('configuration',
                                        'Use an isolated SEC plugin environment with the pinned requirements; httpx2 is unsupported')
                for hop in range(6):
                    validate_url(url, self.archive)
                    if hop:
                        self.before_request()
                    remaining = None if deadline is None else deadline - self.monotonic()
                    if remaining is not None and remaining <= 0:
                        raise TimeoutError('SEC request deadline exceeded')
                    with client.stream('GET', url, headers=dict(request.header_items()),
                                       timeout=remaining, follow_redirects=False) as response:
                        if response.status_code in (301, 302, 303, 307, 308):
                            location = response.headers.get('location')
                            if not location or hop == 5:
                                raise ProviderError('malformed_data', 'Invalid or excessive SEC redirects')
                            url = validate_url(urljoin(url, location), self.archive)
                            continue
                        if response.status_code != 200:
                            headers = Message()
                            for key, value in response.headers.multi_items():
                                headers[key] = value
                            raise HTTPError(url, response.status_code, 'SEC request failed', headers, None)
                        yield StreamResponse(response)
                        return
        except httpx.TimeoutException as exc:
            raise TimeoutError('SEC request timed out') from exc
        except (httpx.RemoteProtocolError, httpx.DecodingError) as exc:
            raise ProviderError('malformed_data', 'Invalid or truncated SEC response') from exc
        except httpx.ProxyError as exc:
            raise ProviderError('configuration', 'Check SEC proxy access configuration') from exc
        except httpx.RequestError as exc:
            cause = exc
            while cause is not None:
                if isinstance(cause, ssl.SSLCertVerificationError):
                    raise ProviderError('configuration', 'Check SEC TLS certificate configuration') from exc
                cause = cause.__cause__
            raise URLError('SEC connection unavailable') from exc
