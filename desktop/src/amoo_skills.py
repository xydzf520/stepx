"""Fixed phone-side skill installer/exporter. Never executes skill code.

Uses ClawHub's current ownerHandle API, since old phone installers only accept
globally unique bare slugs. Receives a JSON argument; stdout is structured proof.
"""
import base64
import hashlib
import io
import json
import os
import pathlib
import re
import shutil
import socket
import stat
import struct
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

MAX_BYTES = 8 * 1024 * 1024
MAX_TEXT = 1024 * 1024
MAX_FILES = 200
BASE = "https://clawhub.ai"
TEXT_SUFFIXES = {".md", ".txt", ".json", ".yaml", ".yml", ".csv"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def component(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,159}", value), "技能标识无效")
    require(value not in (".", ".."), "技能标识无效")
    return value


def _extract_http_error_detail(error):
    try:
        raw = error.read(2048).decode("utf-8", errors="ignore").strip()
    except Exception:
        return ""
    if not raw:
        return ""
    try:
        parsed = json.loads(raw)
        if isinstance(parsed, dict):
            err_field = parsed.get("error") or parsed.get("message") or parsed.get("detail")
            if isinstance(err_field, dict):
                err_field = err_field.get("message") or err_field.get("code")
            if isinstance(err_field, str) and err_field.strip():
                raw = err_field.strip()
    except Exception:
        pass
    clean = " ".join(raw.split())
    clean = re.sub(r"(?i)(bearer\s+|token=|key=)\S+", r"\1[redacted]", clean)
    return clean[:180]


def request(path, query, limit=MAX_BYTES):
    url = BASE + path + "?" + urllib.parse.urlencode(query)
    req = urllib.request.Request(url, headers={"User-Agent": "StepX-Companion/0.1"})
    try:
        with urllib.request.urlopen(req, timeout=12) as response:
            require(urllib.parse.urlparse(response.url).scheme == "https", "技能下载必须使用 HTTPS")
            data = response.read(limit + 1)
            require(len(data) <= limit, "技能包超过大小上限")
            return data
    except urllib.error.HTTPError as error:
        if error.code == 429:
            retry = error.headers.get("Retry-After", "稍后")
            raise ValueError("技能市场限流，请等待 " + retry + " 后重试") from None
        detail = _extract_http_error_detail(error)
        suffix = ("：" + detail) if detail else ""
        raise ValueError("技能市场请求失败 HTTP " + str(error.code) + suffix) from None


def build_ws_connect_params(token=None):
    resolved_token = (token if token is not None else os.environ.get("OPENCLAW_GATEWAY_TOKEN", "")).strip()
    params = {
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": {
            "id": "stepx-companion-skills",
            "version": "0.1",
            "platform": "linux",
            "mode": "cli",
        },
        "role": "operator",
        "scopes": ["operator.read", "operator.write"],
    }
    if resolved_token:
        params["auth"] = {"token": resolved_token}
    return params


def _ws_send_text(sock, text):
    payload = text.encode("utf-8")
    mask = os.urandom(4)
    header = bytearray([0x81])
    length = len(payload)
    if length < 126:
        header.append(0x80 | length)
    elif length < 65536:
        header.append(0x80 | 126)
        header.extend(struct.pack("!H", length))
    else:
        header.append(0x80 | 127)
        header.extend(struct.pack("!Q", length))
    header.extend(mask)
    masked = bytes(b ^ mask[idx & 3] for idx, b in enumerate(payload))
    sock.sendall(bytes(header) + masked)


def _ws_recv_exact(sock, count):
    chunks = bytearray()
    while len(chunks) < count:
        chunk = sock.recv(count - len(chunks))
        if not chunk:
            raise ValueError("OpenClaw 网关连接意外中断")
        chunks.extend(chunk)
    return bytes(chunks)


def _ws_recv_json(sock):
    while True:
        head = _ws_recv_exact(sock, 2)
        opcode = head[0] & 0x0F
        masked = bool(head[1] & 0x80)
        length = head[1] & 0x7F
        if length == 126:
            length = struct.unpack("!H", _ws_recv_exact(sock, 2))[0]
        elif length == 127:
            length = struct.unpack("!Q", _ws_recv_exact(sock, 8))[0]
        require(length <= MAX_BYTES, "OpenClaw 网关响应超过大小上限")
        mask_key = _ws_recv_exact(sock, 4) if masked else None
        payload = _ws_recv_exact(sock, length)
        if mask_key:
            payload = bytes(b ^ mask_key[idx & 3] for idx, b in enumerate(payload))
        if opcode == 0x8:
            raise ValueError("OpenClaw 网关已关闭连接")
        if opcode == 0x9:
            # Reply pong
            pong = bytearray([0x8A, 0x80 | min(len(payload), 125)])
            pmask = os.urandom(4)
            pong.extend(pmask)
            pong.extend(bytes(b ^ pmask[idx & 3] for idx, b in enumerate(payload[:125])))
            sock.sendall(bytes(pong))
            continue
        if opcode == 0x1:
            return json.loads(payload.decode("utf-8"))


def direct_ws_rpc(method, params, host="127.0.0.1", port=18789, token=None, timeout=15):
    ws_key = base64.b64encode(os.urandom(16)).decode("ascii")
    with socket.create_connection((host, int(port)), timeout=timeout) as sock:
        sock.settimeout(timeout)
        handshake = (
            f"GET / HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            f"Upgrade: websocket\r\n"
            f"Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {ws_key}\r\n"
            f"Sec-WebSocket-Version: 13\r\n\r\n"
        )
        sock.sendall(handshake.encode("ascii"))
        header_buf = bytearray()
        while b"\r\n\r\n" not in header_buf:
            chunk = sock.recv(1024)
            if not chunk:
                raise ValueError("OpenClaw 网关握手未响应")
            header_buf.extend(chunk)
            require(len(header_buf) <= 16384, "OpenClaw 网关握手响应异常")
        status_line = header_buf.split(b"\r\n", 1)[0].decode("latin1", errors="ignore")
        require("101" in status_line, "OpenClaw 网关拒绝 WebSocket 升级: " + status_line[:120])

        connect_req = {
            "type": "req",
            "id": "stepx-connect",
            "method": "connect",
            "params": build_ws_connect_params(token=token),
        }
        _ws_send_text(sock, json.dumps(connect_req, ensure_ascii=False))
        while True:
            frame = _ws_recv_json(sock)
            if frame.get("type") == "res" and frame.get("id") == "stepx-connect":
                if not frame.get("ok"):
                    err = frame.get("error") or {}
                    msg = err.get("message") if isinstance(err, dict) else str(err)
                    raise ValueError("OpenClaw 网关鉴权失败: " + (msg or "未授权")[:200])
                break

        rpc_id = "stepx-rpc-1"
        _ws_send_text(
            sock,
            json.dumps({"type": "req", "id": rpc_id, "method": method, "params": params}, ensure_ascii=False),
        )
        while True:
            frame = _ws_recv_json(sock)
            if frame.get("type") == "res" and frame.get("id") == rpc_id:
                if not frame.get("ok"):
                    err = frame.get("error") or {}
                    msg = err.get("message") if isinstance(err, dict) else str(err)
                    raise ValueError((msg or f"{method} 调用失败")[:240])
                return frame.get("payload")


def market_search(args):
    query = str(args.get("query") or "").strip()
    require(1 <= len(query) <= 160, "请输入 1–160 字的技能关键词")
    limit = min(max(int(args.get("limit") or 40), 1), 100)
    errors = []
    try:
        payload = direct_ws_rpc(
            "skills.search",
            {"query": query, "limit": limit},
            token=args.get("gatewayToken"),
        )
        if isinstance(payload, dict):
            payload.setdefault("ok", True)
            return payload
    except Exception as ws_err:
        errors.append(str(ws_err)[:200])
    try:
        raw = request("/api/v1/skills/search", {"q": query, "limit": limit}, MAX_TEXT)
        parsed = json.loads(raw)
        if isinstance(parsed, dict):
            parsed.setdefault("ok", True)
            return parsed
        if isinstance(parsed, list):
            return {"ok": True, "skills": parsed}
    except Exception as http_err:
        errors.append(str(http_err)[:200])
    detail = "；".join(e for e in errors if e) or "技能市场未返回有效数据"
    raise ValueError("技能市场搜索失败：" + detail[:320])


def safe_path(value):
    require(isinstance(value, str) and value and "\\" not in value and "\x00" not in value, "技能包路径无效")
    path = pathlib.PurePosixPath(value)
    require(not path.is_absolute() and all(x not in (".", "..") for x in value.split("/")), "技能包包含越界路径")
    return path


def install(args):
    owner, slug = component(args["owner"]), component(args["slug"])
    query = {"ownerHandle": owner}
    detail = json.loads(request("/api/v1/skills/" + slug, query, MAX_TEXT))
    require(detail.get("owner", {}).get("handle", "").lower() == owner.lower(), "技能发布者与所选条目不一致")
    require(detail.get("skill", {}).get("slug") == slug, "技能标识与所选条目不一致")
    moderation = detail.get("moderation") or {}
    require(not moderation.get("isMalwareBlocked") and not moderation.get("isSuspicious"), "此技能被市场标记，不自动安装")
    version = detail.get("latestVersion", {}).get("version")
    require(isinstance(version, str) and re.fullmatch(r"[a-zA-Z0-9_.+-]{1,100}", version), "市场未确认可安装版本")
    metadata = json.loads(request("/api/v1/skills/" + slug + "/versions/" + urllib.parse.quote(version, safe=""), query, MAX_TEXT))
    release = metadata.get("version", {})
    require(release.get("version") == version and metadata.get("skill", {}).get("slug") == slug, "技能版本不一致")
    security = release.get("security") or {}
    require(security.get("status") not in ("malicious", "suspicious", "blocked"), "此版本未通过市场安全检查")
    manifest = release.get("files", [])
    require(isinstance(manifest, list) and 0 < len(manifest) <= MAX_FILES, "技能文件清单无效或过大")
    expected = {}
    for entry in manifest:
        name = str(safe_path(entry.get("path")))
        require(name not in expected and name != "_meta.json", "技能清单路径重复")
        digest = entry.get("sha256", "")
        require(re.fullmatch(r"[a-fA-F0-9]{64}", digest), "技能清单缺少文件校验")
        size = entry.get("size")
        require(isinstance(size, int) and 0 <= size <= MAX_BYTES, "技能文件大小无效")
        expected[name] = (digest.lower(), size)
    require(sum(size for _, size in expected.values()) <= MAX_BYTES, "技能解压后超过大小上限")
    require("SKILL.md" in expected, "技能缺少 SKILL.md")
    data = request("/api/v1/download", {"ownerHandle": owner, "slug": slug, "version": version})
    archive = zipfile.ZipFile(io.BytesIO(data))
    require(len(archive.infolist()) <= MAX_FILES + 1, "技能压缩包条目过多")
    contents = {}
    meta = None
    for item in archive.infolist():
        name = str(safe_path(item.filename))
        require(name not in contents and not item.is_dir(), "技能压缩包路径重复或无效")
        require(not stat.S_ISLNK(item.external_attr >> 16), "技能压缩包包含符号链接")
        require(item.file_size <= MAX_BYTES and not (item.flag_bits & 1), "技能压缩包不受支持")
        raw = archive.read(item)
        if name == "_meta.json":
            require(meta is None, "技能来源记录重复")
            meta = json.loads(raw)
            continue
        require(name in expected, "技能包包含清单外文件")
        digest, size = expected[name]
        require(len(raw) == size and hashlib.sha256(raw).hexdigest() == digest, "技能文件完整性校验失败")
        contents[name] = raw
    require(set(contents) == set(expected), "技能压缩包缺少清单文件")
    require(meta and meta.get("slug") == slug and meta.get("version") == version, "技能来源记录不一致")
    # Plain instruction content is validated before installing, but never executed.
    skill_text = contents["SKILL.md"].decode("utf-8")
    require(len(contents["SKILL.md"]) <= MAX_TEXT and skill_text.strip(), "技能说明为空或过大")
    match = re.search(r"(?m)^name:\s*(.+)$", skill_text)
    skill_name = match.group(1).strip().strip("\"'") if match else slug
    require(skill_name not in args.get("existingNames", []), "手机已存在同名技能，不会覆盖")
    workspace = pathlib.Path(args["workspace"])
    require(workspace.is_absolute() and str(workspace.resolve()).startswith("/root/.openclaw/"), "手机技能工作目录异常")
    require(not workspace.is_symlink(), "手机技能工作目录不能为符号链接")
    root = workspace / "skills"
    root.mkdir(exist_ok=True)
    require(not root.is_symlink() and root.resolve().parent == workspace.resolve(), "手机技能目录越界")
    target = root / (owner.lower() + "--" + slug)
    require(not target.exists() and not target.is_symlink(), "此发布者的技能已安装，不会覆盖")
    stage = pathlib.Path(tempfile.mkdtemp(prefix=".stepx-install-", dir=root))
    try:
        for name, raw in contents.items():
            file = stage.joinpath(*safe_path(name).parts)
            file.parent.mkdir(parents=True, exist_ok=True)
            with file.open("xb") as out:
                out.write(raw)
            file.chmod(0o644)
        provenance = {"version": 1, "registry": BASE, "ownerHandle": owner, "slug": slug,
                      "installedVersion": version, "installedAt": int(time.time() * 1000),
                      "archiveSha256": hashlib.sha256(data).hexdigest(), "installer": "stepx-owner-qualified-v1",
                      "files": {name: digest for name, (digest, _) in expected.items()}}
        (stage / ".clawhub").mkdir()
        (stage / ".clawhub" / "origin.json").write_text(json.dumps(provenance), encoding="utf-8")
        # No shared lockfile rewrite: other phone/client installs may run concurrently.
        # rename() cannot overwrite a populated existing skill directory.
        require(not target.exists(), "技能安装目录已存在")
        stage.rename(target)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    installed = (target / "SKILL.md").read_bytes()
    require(hashlib.sha256(installed).hexdigest() == expected["SKILL.md"][0], "手机安装后校验失败")
    return {"ok": True, "owner": owner, "slug": slug, "version": version, "name": skill_name,
            "targetDir": str(target), "filePath": str(target / "SKILL.md"), "files": len(contents),
            "sha256": expected["SKILL.md"][0], "bytes": sum(len(raw) for raw in contents.values())}


def export(args):
    main = pathlib.Path(args["filePath"])
    require(main.is_absolute() and main.name == "SKILL.md" and main.is_file() and not main.is_symlink(), "手机技能说明路径无效")
    roots = [pathlib.Path(p).resolve() for p in args["allowedRoots"] if p and pathlib.Path(p).is_absolute()]
    base = main.parent.resolve()
    require(any(base.is_relative_to(root) for root in roots), "手机技能不在声明目录内")
    files, total, omitted = [], 0, 0
    for index, path in enumerate(sorted(base.rglob("*"))):
        require(index < 1000, "手机技能目录条目过多")
        if len(files) >= MAX_FILES:
            raise ValueError("手机技能文本文件过多")
        if path.is_symlink():
            omitted += 1
            continue
        if not path.is_file() or ".clawhub" in path.parts:
            continue
        if path.suffix.lower() not in TEXT_SUFFIXES:
            omitted += 1
            continue
        require(path.resolve().is_relative_to(base), "手机技能引用文件越界")
        require(path.stat().st_size <= MAX_TEXT, "技能引用文本过大")
        raw = path.read_bytes()
        total += len(raw)
        require(total <= MAX_TEXT, "技能引用文本总量过大")
        try:
            content = raw.decode("utf-8")
        except UnicodeDecodeError:
            omitted += 1
            continue
        files.append({"path": str(path.relative_to(base)), "content": content})
    require(any(file["path"] == "SKILL.md" for file in files), "未读取到手机 SKILL.md")
    return {"name": args["name"], "files": files, "omitted": omitted}


def origins(args):
    result = {}
    root = pathlib.Path(args["workspace"]) / "skills"
    require(root.is_absolute() and str(root.resolve()).startswith("/root/.openclaw/"), "手机技能目录异常")
    for main_path in args["files"][:MAX_FILES]:
        main = pathlib.Path(main_path)
        if not main.is_absolute() or main.is_symlink() or not main.resolve().is_relative_to(root.resolve()):
            continue
        origin = main.parent / ".clawhub" / "origin.json"
        if origin.is_file() and not origin.is_symlink() and origin.stat().st_size <= MAX_TEXT:
            data = json.loads(origin.read_text(encoding="utf-8"))
            if data.get("installer") == "stepx-owner-qualified-v1" and data.get("registry") == BASE:
                owner, slug = component(data["ownerHandle"]), component(data["slug"])
                result[main_path] = {"reference": "@" + owner + "/" + slug, "version": data["installedVersion"]}
    return result


if __name__ == "__main__":
    try:
        args = json.loads(sys.argv[1])
        action = args.get("action")
        require(action in ("install", "export", "origins", "search"), "未知技能操作")
        result = (
            install(args)
            if action == "install"
            else origins(args)
            if action == "origins"
            else market_search(args)
            if action == "search"
            else export(args)
        )
        print(json.dumps(result, ensure_ascii=False))
    except Exception as error:
        print(json.dumps({"ok": False, "error": str(error)[:400]}, ensure_ascii=False))
        sys.exit(1)
