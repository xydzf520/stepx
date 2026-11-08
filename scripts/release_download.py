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

def download_verified_tool(url, destination, expected_sha256, *, asset_api_url=None):
    """Retry transient upstream failures without accepting partial/unverified tools."""
    destination = Path(destination)
    for attempt in range(3):
        partial = None
        try:
            request_url = asset_api_url if attempt and asset_api_url else url
            if attempt:
                parts = urllib.parse.urlsplit(request_url)
                query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
                query.append(('stepx_retry', str(time.time_ns())))
                request_url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
            request = urllib.request.Request(request_url, headers={
                'User-Agent': 'StepX-release-packager', 'Accept': 'application/octet-stream'})
            started = time.monotonic()
            digest = hashlib.sha256()
            with urllib.request.urlopen(request, timeout=30) as response:
                with tempfile.NamedTemporaryFile(dir=destination.parent, suffix='.part', delete=False) as output:
                    partial = Path(output.name)
                    while True:
                        if time.monotonic() - started > 120:
                            raise TimeoutError('Tool archive download exceeded its time budget')
                        # At most one underlying read before checking the deadline again.
                        chunk = response.read1(64 * 1024)
                        if not chunk:
                            break
                        digest.update(chunk)
                        output.write(chunk)
            if digest.hexdigest() != expected_sha256:
                raise RuntimeError('Tool archive SHA256 mismatch')
            os.replace(partial, destination)
            return
        except urllib.error.HTTPError as error:
            if error.code not in {408, 429, 500, 502, 503, 504} or attempt == 2:
                raise
            print(f'Tool download returned HTTP {error.code}; retrying ({attempt + 2}/3)', flush=True)
        except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.IncompleteRead):
            if attempt == 2:
                raise
            print(f'Tool download interrupted; retrying ({attempt + 2}/3)', flush=True)
        finally:
            if partial is not None:
                partial.unlink(missing_ok=True)
        time.sleep(attempt + 1)

