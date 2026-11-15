"""Download pinned release dependencies from their official sources."""
import hashlib
import http.client
import os
from pathlib import Path
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

GITHUB_MIRRORS = (
    'https://ghfast.top/',
    'https://gh-proxy.com/',
)
RETRYABLE_HTTP_CODES = {408, 429, 500, 502, 503, 504}
PROGRESS_WINDOW_SECONDS = 15.0
MIN_WINDOW_BYTES = 128 * 1024
DEFAULT_STALL_TIMEOUT = 120.0


def _candidate_urls(url, asset_api_url):
    if url.startswith('https://github.com/'):
        mirrors = [prefix + url for prefix in GITHUB_MIRRORS]
        if asset_api_url:
            return [url, asset_api_url, *mirrors]
        return [url, *mirrors]
    fallback = asset_api_url or url
    return [url, fallback, fallback]


def download_verified_tool(url, destination, expected_sha256, *, asset_api_url=None,
                           timeout=30, stall_timeout=DEFAULT_STALL_TIMEOUT):
    """Retry transient upstream failures without accepting partial/unverified tools."""
    destination = Path(destination)
    candidates = _candidate_urls(url, asset_api_url)
    attempt = 0
    while attempt < len(candidates):
        partial = None
        candidate = candidates[attempt]
        is_mirror = any(candidate.startswith(prefix) for prefix in GITHUB_MIRRORS)
        try:
            request_url = candidate
            if attempt and not is_mirror:
                parts = urllib.parse.urlsplit(request_url)
                query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
                query.append(('stepx_retry', str(time.time_ns())))
                request_url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
            request = urllib.request.Request(request_url, headers={
                'User-Agent': 'StepX-release-packager', 'Accept': 'application/octet-stream'})
            hostname = urllib.parse.urlsplit(request_url).hostname
            token = (os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN') or '').strip()
            if hostname == 'api.github.com' and token:
                request.add_unredirected_header('Authorization', f'Bearer {token}')
            started = time.monotonic()
            window_started = started
            last_liveness = started
            window_bytes = 0
            digest = hashlib.sha256()
            with urllib.request.urlopen(request, timeout=timeout) as response:
                read_chunk = getattr(response, 'read1', None) or response.read
                with tempfile.NamedTemporaryFile(dir=destination.parent, suffix='.part', delete=False) as output:
                    partial = Path(output.name)
                    while True:
                        now = time.monotonic()
                        if now - window_started >= PROGRESS_WINDOW_SECONDS:
                            if window_bytes >= MIN_WINDOW_BYTES:
                                last_liveness = now
                            window_started = now
                            window_bytes = 0
                        if now - last_liveness > stall_timeout:
                            raise TimeoutError('Tool archive download stalled without sufficient progress')
                        # At most one underlying read before checking liveness again.
                        chunk = read_chunk(64 * 1024)
                        now = time.monotonic()
                        if not chunk:
                            break
                        digest.update(chunk)
                        output.write(chunk)
                        window_bytes += len(chunk)
                        if window_bytes >= MIN_WINDOW_BYTES:
                            elapsed = now - window_started
                            if elapsed <= PROGRESS_WINDOW_SECONDS or (
                                window_bytes / elapsed >= MIN_WINDOW_BYTES / PROGRESS_WINDOW_SECONDS
                            ):
                                last_liveness = now
                            window_started = now
                            window_bytes = 0
                        elif now - window_started >= PROGRESS_WINDOW_SECONDS:
                            window_started = now
                            window_bytes = 0
                        if now - last_liveness > stall_timeout:
                            raise TimeoutError('Tool archive download stalled without sufficient progress')
            if digest.hexdigest() != expected_sha256:
                raise RuntimeError('Tool archive SHA256 mismatch')
            os.replace(partial, destination)
            return
        except urllib.error.HTTPError as error:
            if attempt == 0 and not asset_api_url and url.startswith('https://github.com/') and error.code in RETRYABLE_HTTP_CODES:
                candidates.insert(1, url)
            retryable = is_mirror or error.code in RETRYABLE_HTTP_CODES or (candidate == asset_api_url and error.code == 403)
            if not retryable or attempt == len(candidates) - 1:
                raise
            total = len(candidates)
            print(f'Tool download returned HTTP {error.code}; retrying ({attempt + 2}/{total})', flush=True)
        except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.IncompleteRead):
            if attempt == len(candidates) - 1:
                raise
            total = len(candidates)
            print(f'Tool download interrupted; retrying ({attempt + 2}/{total})', flush=True)
        except RuntimeError:
            if not is_mirror or attempt == len(candidates) - 1:
                raise
            total = len(candidates)
            print(f'Mirror tool archive failed SHA256 check; retrying ({attempt + 2}/{total})', flush=True)
        finally:
            if partial is not None:
                partial.unlink(missing_ok=True)
        time.sleep(attempt + 1)
        attempt += 1

