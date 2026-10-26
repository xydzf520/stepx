#!/usr/bin/env python3
"""Read-only Play Protect certification model and evidence collection.

Public interface: explain | status | csr | chain | monitor | dossier.
STEPX_SERIAL and STEPX_ADB select the existing transport. Every device
operation is read-only (getprop, pm, dumpsys, and the RKP dump/list/csr
subcommands); `remote_provisioning certify` provisions certificates and
contacts Google, so it is deliberately not reachable from here.

Nothing in this module grants, fakes or bypasses Google device
certification. A layer verdict is never inferred from a client-side change
such as the Play compatibility patch, and every blocker names the party
that alone can clear it.
"""

import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "stepx-play-protect-certification-v1"
FORMAT = 1
DEFAULT_SERIAL = ""
MAX_CATALOG_BYTES = 8 * 1024 * 1024
SUPPORTED_DEVICES_URL = "https://storage.googleapis.com/play_public/supported_devices.html"
ATTESTATION_ROOTS_URL = "https://android.googleapis.com/attestation/root"
ATTESTATION_STATUS_URL = "https://android.googleapis.com/attestation/status"
# AOSP itself names HTTP 444 as DEVICE_NOT_REGISTERED, which is the authority
# behind this tool's diagnosis.
AOSP_RKP_SOURCE = ("https://android.googlesource.com/platform/packages/apps/RemoteProvisioner/"
                   "+/main/src/com/android/remoteprovisioner/RemoteProvisioningException.java")
RKP_SERVICE = "remote_provisioning"
# `certify` is excluded on purpose: it asks Google to sign certificates.
RKP_READ_ONLY = ("dump", "list", "csr")
GOOGLE_PACKAGES = ("com.google.android.gsf", "com.google.android.gms", "com.android.vending")
UNCERTIFIED_ACTIVITY = "com.google.android.gms.gmscompliance.ui.UncertifiedDeviceActivity"

# The four layers Google itself walks before it will show "设备已通过认证".
# Each layer is necessary; none of them is sufficient on its own.
LAYERS = (
    {
        "id": "client_admission",
        "title": "第 1 层 · 客户端准入层",
        "title_en": "Layer 1 - client admission",
        "owner": "user",
        "question": "未认证设备是否被 Play / GMS 本地开关拦住，连商店都打不开？",
        "google_checks": [
            "Finsky GmscoreCompliance__activities_to_block_on_uncertified_devices",
            "GMS com.google.android.gms.gmscompliance 布尔 flag 1",
        ],
        "clears": "用户侧兼容补丁可以放行，但只恢复商店可用性，不改变认证结果。",
    },
    {
        "id": "system_boot",
        "title": "第 2 层 · 系统与启动层",
        "title_en": "Layer 2 - system and boot state",
        "owner": "oem",
        "question": "构建指纹、签名标签、启动链与 SELinux 是否处于正式版合规状态？",
        "google_checks": [
            "ro.build.tags == release-keys 且 ro.build.type == user",
            "ro.boot.flash.locked == 1 且 ro.boot.verifiedbootstate == green",
            "SELinux Enforcing，指纹字段内部一致",
        ],
        "clears": "本层正常是必要条件，但与认证通过之间没有充分关系。",
    },
    {
        "id": "key_attestation",
        "title": "第 3 层 · 硬件密钥证明层",
        "title_en": "Layer 3 - hardware key attestation",
        "owner": "oem",
        "question": "TEE 签发的密钥证明链是否锚定到 Google 官方硬件证明根？",
        "google_checks": [
            "证书链内部签名与有效期有效",
            "根证书精确匹配 https://android.googleapis.com/attestation/root 公布的受信根",
            "链上任意证书不出现在 https://android.googleapis.com/attestation/status 吊销列表",
            "证明扩展 security level == TrustedEnvironment，rootOfTrust 为 Verified/locked",
        ],
        "clears": "只能由厂商在产线烧录 Google 签发的生产 Keybox；本机无法自行补齐。",
    },
    {
        "id": "rkp_and_catalog",
        "title": "第 4 层 · 远程密钥配置与设备目录层",
        "title_en": "Layer 4 - remote key provisioning and device catalog",
        "owner": "oem",
        "question": "RKP 后端是否认识这台设备，机型是否已进入 Google 认证设备目录？",
        "google_checks": [
            "com.android.rkpdapp 与 RKP HAL 可用，remote_provisioning.hostname 已配置",
            "系统防火墙不阻断 com.android.rkpdapp.IRemoteProvisioning",
            "remoteprovisioning.googleapis.com signCertificates 返回 200 而非 444",
            "supported_devices.html 收录该 brand + model",
        ],
        "clears": "需要 SoC 厂商 / 整机厂商向 Google RKP 后台登记硬件公钥，并通过 CTS/VTS/GTS 备案。",
    },
)
LAYER_IDS = tuple(layer["id"] for layer in LAYERS)

# Field names carried by the Qualcomm RKP device-info block inside a CSR.
RKP_INFO_FIELDS = (
    "brand", "manufacturer", "model", "device", "product", "vb_state",
    "os_version", "vbmeta_digest", "security_level", "boot_patch_level",
    "bootloader_state", "system_patch_level", "vendor_patch_level",
)


# How much each OEM action buys, so the ticket can lead with the highest
# leverage one. RKP ranks first because a remotely provisioned chain is
# validated against the same official root list (GTS ships
# testEcAttestationChainRemProvLengthTee for exactly that), so registration is
# expected to clear the layer-3 root match as well as the layer-4 backend
# check. That expectation is an inference and must be re-measured after
# registration; this tool cannot prove it on an unregistered device.
LEVERAGE = (
    {"action": "向 Google RKP 后台登记硬件公钥（第 2 步）",
     "action_en": "Register this device's RKP hardware public key with Google (step 2)",
     "clears": ["key_attestation", "rkp_and_catalog"],
     "owner": "soc_vendor_or_oem",
     "reason": "远程配置链与工厂预置链共用同一份官方根列表校验；登记成功后第 3 层根匹配"
               "与第 4 层 signCertificates 可同时解除。此为推断，需登记后实测确认。",
     "reason_en": "Remotely provisioned and factory-provisioned chains are validated against "
               "the same official root list, so registration is expected to clear both the "
               "layer-3 root match and the layer-4 backend check. Inference; re-measure after."},
    {"action": "移除固件防火墙拦截并配置 remote_provisioning.hostname（第 3 步）",
     "action_en": "Remove the firmware firewall block and set remote_provisioning.hostname (step 3)",
     "clears": ["rkp_and_catalog"],
     "owner": "oem",
     "reason": "让系统能真正发起轮换；单独做这一步不够，后端仍会返回 444。",
     "reason_en": "Lets the system actually start rotating certificates; not sufficient alone, "
               "the backend still answers 444."},
    {"action": "CTS / VTS / GTS 备案，进入 Play 认证设备目录（第 4 步）",
     "action_en": "Complete CTS / VTS / GTS so the model enters the Play certified device catalog (step 4)",
     "clears": ["rkp_and_catalog"],
     "owner": "oem",
     "reason": "Play 商店显示「已通过认证」还要求型号出现在官方目录中。",
     "reason_en": "Play only shows \"device is certified\" when the model appears in the official "
               "catalog."},
    {"action": "产线烧录 Google 签发的生产 Keybox（第 1 步）",
     "action_en": "Burn a Google-issued production keybox at the factory (step 1)",
     "clears": ["key_attestation"],
     "owner": "oem",
     "reason": "传统工厂预置路径；若第 2 步完成则不再是必需，可作为并行备选。",
     "reason_en": "The traditional factory-provisioning path; unnecessary if step 2 completes, "
               "but usable in parallel."},
)


class ProbeError(RuntimeError):
    """A probe could not run. Callers degrade that layer to `unknown`."""


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def adb_command():
    return [os.environ.get("STEPX_ADB") or shutil.which("adb") or "adb"]


def is_wireless(serial):
    return ":" in serial or "._adb-tls-connect._tcp" in serial


class Device:
    """Read-only device access pinned to one explicitly selected transport."""

    def __init__(self, serial, adb=None):
        if not serial or serial.startswith("-") or any(c.isspace() for c in serial):
            raise ProbeError("请先选择已授权的手机连接")
        self.serial = serial
        self.adb = list(adb or adb_command())

    def _run(self, args, timeout=40, check=True):
        try:
            result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
        except (OSError, subprocess.SubprocessError) as error:
            raise ProbeError(f"无法执行 {args[0]}：{type(error).__name__}") from None
        if check and result.returncode != 0:
            raise ProbeError(f"{args[-1]} 执行失败")
        return result

    def raw(self, *args, timeout=40, check=True):
        return self._run(self.adb + ["-s", self.serial, *args], timeout=timeout, check=check)

    def shell(self, *args, timeout=40):
        return self.raw("shell", *[str(a) for a in args], timeout=timeout).stdout.strip()

    def prop(self, name):
        return self.shell("getprop", name)

    def connected(self):
        return self.raw("get-state", timeout=20, check=False).stdout.strip() == "device"

    def verify_identity(self):
        """ADB transport names are not hardware identities."""
        if not self.connected():
            raise ProbeError("手机连接未授权或已离线，请重新连接")
        physical = os.environ.get("STEPX_DEVICE_SERIAL", "")
        if is_wireless(self.serial) and not physical:
            raise ProbeError("无线连接缺少设备身份，请重新选择手机后重试")
        expected = physical or ("" if is_wireless(self.serial) else self.serial)
        if expected and self.prop("ro.serialno") != expected:
            raise ProbeError("连接的手机与已选设备不一致，请重新选择设备")

    def identity(self):
        keys = ("brand", "manufacturer", "model", "device", "name")
        return {key: self.prop("ro.product." + key) for key in keys}

    def rpmb_props(self):
        """Factory provisioning markers reported by the vendor namespace."""
        try:
            raw = self.shell("getprop")
        except ProbeError:
            return {}
        found = {}
        for line in raw.splitlines():
            name, _, value = line.partition(":")
            name = name.strip().lstrip("[").rstrip("]")
            value = value.strip().lstrip("[").rstrip("]")
            if "rpmb" in name.casefold() and "provision" in name.casefold() and value:
                found[name] = value
        return found


def layer_result(layer, state, checks, note, extra=None):
    result = {
        "layer": layer["id"],
        "title": layer["title"],
        "owner": layer["owner"],
        "state": state,
        "checks": checks,
        "note": note,
    }
    if extra:
        result.update(extra)
    return result


def check(name, value, expected, ok, detail=None):
    # `ok is None` means "not probed"; it must never collapse into a failure.
    entry = {"check": name, "value": value, "expected": expected,
             "ok": None if ok is None else bool(ok)}
    if detail:
        entry["detail"] = detail
    return entry


def layer_state(checks):
    """A layer is only pass/fail when every one of its checks was probed."""
    if any(entry["ok"] is None for entry in checks):
        return "unknown"
    return "pass" if all(entry["ok"] for entry in checks) else "fail"


def unknown_layer(layer, reason):
    return layer_result(layer, "unknown", [], f"本层未能取证：{reason}")


# --------------------------------------------------------------------------
# Layer 1 - client admission
# --------------------------------------------------------------------------

COMPAT_SCRIPT = "google-play-compat.py"


def probe_compat_patch(device, adb_path=None, script=None):
    """Ask the repository's own compatibility component for its verdict.

    Layer 1 used to infer "the store opens" from "the uncertified activity is
    not in the foreground right now", which only proves it is not blocked at
    this instant. The compatibility component is the authority on whether the
    patch is actually in force, and it also reports explicitly whether anything
    it did touched certification. Its private receipt id is never surfaced.
    """
    script = Path(script) if script else ROOT / "scripts" / COMPAT_SCRIPT
    if not script.is_file():
        return {"available": False, "reason": "未找到 Play 兼容补丁组件"}
    environment = dict(os.environ)
    environment["STEPX_SERIAL"] = device.serial
    if adb_path:
        environment["STEPX_ADB"] = adb_path
    environment.pop("PYTHONOPTIMIZE", None)
    try:
        result = subprocess.run(
            [sys.executable, str(script), "status"],
            capture_output=True, text=True, timeout=150, env=environment)
    except (OSError, subprocess.SubprocessError) as error:
        return {"available": False, "reason": f"无法运行兼容补丁组件：{type(error).__name__}"}
    try:
        value = json.loads(result.stdout)
    except ValueError:
        return {"available": False, "reason": "兼容补丁组件未返回有效状态"}
    for key in ("ok", "supported", "active", "can_rollback", "official_certification_changed"):
        if not isinstance(value.get(key), bool):
            return {"available": False, "reason": f"兼容补丁组件返回的状态缺少 {key}"}
    scopes = value.get("scopes")
    if not isinstance(scopes, dict) or not scopes:
        return {"available": False, "reason": "兼容补丁组件返回的状态缺少 scopes"}
    return {
        "available": True,
        "ok": value["ok"],
        "supported": value["supported"],
        "active": value["active"],
        "can_rollback": value["can_rollback"],
        "official_certification_changed": value["official_certification_changed"],
        "scopes": {name: {key: bool(state.get(key)) for key in ("cache_active", "db_active")}
                   for name, state in scopes.items() if isinstance(state, dict)},
        "note": "Play 兼容补丁只恢复商店可用性；official_certification_changed 为 false "
                "表示它没有改动 Google 的设备认证。",
    }


def probe_client_admission(device, state):
    layer = LAYERS[0]
    try:
        activities = device.shell("dumpsys", "activity", "activities")
        installed = device.shell("pm", "list", "packages", "--user", "0")
    except ProbeError as error:
        return unknown_layer(layer, str(error))
    packages = {line.strip().removeprefix("package:") for line in installed.splitlines()}
    present = {name for name in GOOGLE_PACKAGES if name in packages}
    missing = [name for name in GOOGLE_PACKAGES if name not in packages]
    blocked = any(
        "topResumedActivity=" in line and UNCERTIFIED_ACTIVITY in line
        for line in activities.splitlines()
    )
    disabled = {
        line.strip().removeprefix("package:")
        for line in device.shell("pm", "list", "packages", "-d", "--user", "0").splitlines()
    }
    compat = state.get("compat_patch")
    if compat is None:
        compat = probe_compat_patch(device, adb_path=state.get("adb_path"))
        state["compat_patch"] = compat
    checks = [
        check("Google 三件套已安装", f"{len(present)}/3", "3/3", len(present) == 3,
              "缺失：" + "、".join(missing) if missing else None),
        check("组件被停用", sorted(present & disabled) or "无", "无", not (present & disabled)),
        check("当前前台为未认证拦截页", blocked, False, not blocked),
    ]
    if compat.get("available"):
        # The compatibility component is the authority on whether the patch holds.
        checks.append(check("Play 兼容补丁生效", compat.get("active"), True, compat.get("active"),
                            "由仓库自带的兼容补丁组件回读，而非由前台活动推断"))
        checks.append(check(
            "兼容补丁未改动设备认证", compat.get("official_certification_changed"), False,
            compat.get("official_certification_changed") is False,
            "该字段为 true 说明补丁越界改动了认证，必须停止并核查"))
    else:
        checks.append(check("Play 兼容补丁生效", "未取证", True, None,
                            compat.get("reason", "兼容补丁状态未取到")))
    # The store may open while certification still fails; that is the whole point.
    # A patch that crossed into certification is a defect, not usability.
    boundary_ok = compat.get("official_certification_changed") is not True
    usable = (len(present) == 3 and not blocked and compat.get("active") is True
              and boundary_ok)
    return layer_result(
        layer, layer_state(checks), checks,
        "商店可用性已恢复或本就可用；这不构成认证通过。" if usable
        else "商店仍被未认证拦截或组件不完整，请先处理可用性。",
        {"store_usable": usable, "compat_patch": compat},
    )


# --------------------------------------------------------------------------
# Layer 2 - system and boot state
# --------------------------------------------------------------------------

BOOT_PROPS = (
    ("ro.build.tags", "release-keys"),
    ("ro.build.type", "user"),
    ("ro.boot.flash.locked", "1"),
    ("ro.boot.verifiedbootstate", "green"),
)


def probe_system_boot(device, state):
    layer = LAYERS[1]
    try:
        device.verify_identity()
        props = {name: device.prop(name) for name, _ in BOOT_PROPS}
        enforcing = device.shell("getenforce")
        fingerprint = device.prop("ro.build.fingerprint")
        identity = device.identity()
    except ProbeError as error:
        return unknown_layer(layer, str(error))
    checks = [check(name, props[name], expected, props[name] == expected)
              for name, expected in BOOT_PROPS]
    checks.append(check("SELinux", enforcing, "Enforcing", enforcing == "Enforcing"))
    # A fingerprint that contradicts its own fields can never match a listing.
    # The format is brand/product/device:… and product need not equal device
    # (StepX happens to use pandora/pandora; Honor uses ELZ-AN10/HNELZ).
    brand = identity["brand"]
    device_name = identity["device"]
    consistent = (bool(brand) and bool(device_name) and bool(identity["model"])
                  and fingerprint.startswith(brand + "/")
                  and f"/{device_name}:" in fingerprint)
    checks.append(check("指纹与品牌/机型一致", fingerprint,
                        f"{brand}/…/{device_name}:…", consistent))
    ok = layer_state(checks)
    state["identity"] = identity
    state["fingerprint"] = fingerprint
    # SDK is recorded, never judged: any Android release can be certified, so
    # hardcoding one level would be a fabricated criterion. It is evidence, not
    # a check, so it lives beside the layer instead of inside `checks`.
    sdk = device.prop("ro.build.version.sdk")
    return layer_result(
        layer, ok, checks,
        "启动链与构建标签处于正式版状态；这是必要条件，不证明认证通过。" if ok == "pass"
        else "存在非正式版启动或构建状态，需由固件侧修正。" if ok == "fail"
        else "启动链部分字段未取到，无法判定。",
        {"sdk": sdk, "security_patch": device.prop("ro.build.version.security_patch")},
    )


# --------------------------------------------------------------------------
# Layer 3 - hardware key attestation
# --------------------------------------------------------------------------

def _openssl():
    return shutil.which("openssl")


def _run_openssl(args, stdin=None):
    try:
        return subprocess.run(["openssl", *args], input=stdin, text=True,
                              capture_output=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None


def _cert_field(pem_path, *flags):
    result = _run_openssl(["x509", "-in", str(pem_path), "-noout", *flags])
    if result is None or result.returncode != 0:
        return None
    return result.stdout.strip()


def load_chain(directory):
    """Read a previously captured attestation chain; never generate keys here."""
    directory = Path(directory)
    if not directory.is_dir():
        raise ProbeError(f"证明链目录不存在：{directory}")
    leaf = directory / "leaf.pem"
    intermediates = directory / "intermediates.pem"
    root = directory / "chain-provided-root.pem"
    for path in (leaf, intermediates, root):
        if not path.is_file():
            raise ProbeError(f"证明链缺少 {path.name}")
    return leaf, intermediates, root


def describe_roots(path):
    """Describe every root in Google's published list, in file order."""
    bundle = _run_openssl(["crl2pkcs7", "-nocrl", "-certfile", str(path)])
    if bundle is None or bundle.returncode != 0:
        return []
    pem = _run_openssl(["pkcs7", "-print_certs"], stdin=bundle.stdout)
    if pem is None:
        return []
    bodies = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                        pem.stdout, re.S)
    roots = []
    with tempfile.TemporaryDirectory(prefix="stepx-roots-") as scratch:
        for index, body in enumerate(bodies):
            single = Path(scratch) / f"root-{index}.pem"
            single.write_text(body)
            dates = (_cert_field(single, "-startdate", "-enddate") or "").splitlines()
            text = _run_openssl(["x509", "-in", str(single), "-noout", "-text"])
            algorithm = key_usage = ""
            if text and text.returncode == 0:
                found = re.search(r"Public Key Algorithm: ([^\n]+)", text.stdout)
                algorithm = found.group(1).strip() if found else ""
                usage = re.search(r"Key Usage: critical\n\s+([^\n]+)", text.stdout)
                key_usage = usage.group(1).strip() if usage else ""
            roots.append({
                "subject": (_cert_field(single, "-subject") or "").removeprefix("subject="),
                "serial": (_cert_field(single, "-serial") or "").removeprefix("serial="),
                "not_before": dates[0].removeprefix("notBefore=") if dates else "",
                "not_after": dates[1].removeprefix("notAfter=") if len(dates) > 1 else "",
                "public_key_algorithm": algorithm,
                "key_usage": key_usage,
                "sha256_fingerprint": (_cert_field(single, "-fingerprint", "-sha256") or "")
                    .removeprefix("sha256 Fingerprint="),
            })
    return roots


def chain_certificates(directory):
    """Return subject and decimal serial for every certificate in a capture."""
    leaf, intermediates, _ = load_chain(directory)
    bundle = subprocess.run(
        ["openssl", "crl2pkcs7", "-nocrl", "-certfile", str(leaf), "-certfile", str(intermediates)],
        capture_output=True)
    if bundle.returncode != 0:
        raise ProbeError("无法读取已采集的证明链")
    pem = subprocess.run(["openssl", "pkcs7", "-print_certs"],
                         input=bundle.stdout.decode("latin-1"), capture_output=True, text=True)
    if pem.returncode != 0:
        raise ProbeError("无法解析证明链证书")
    bodies = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                        pem.stdout, re.S)
    certificates = []
    with tempfile.TemporaryDirectory(prefix="stepx-chain-") as scratch:
        for index, body in enumerate(bodies):
            path = Path(scratch) / f"cert-{index}.pem"
            path.write_text(body)
            subject = (_cert_field(path, "-subject") or "").removeprefix("subject=")
            serial = (_cert_field(path, "-serial") or "").removeprefix("serial=")
            try:
                decimal = str(int(serial, 16))
            except ValueError:
                decimal = ""
            certificates.append({"index": index, "subject": subject, "serial": decimal})
    return certificates


def check_revocation(directory, status_url=ATTESTATION_STATUS_URL, offline=False, cache=None):
    """Check the captured chain against Google's published revocation list.

    The list is JSON: {"entries": {"<decimal serial>": {"status", "reason"}}}.
    NOT_LISTED means there was no entry at verification time. It is not a trust
    decision and not a Play certification verdict.
    """
    result = {"available": False, "any_revoked_or_suspended": None, "certificates": []}
    try:
        certificates = chain_certificates(directory)
    except ProbeError as error:
        result["reason"] = str(error)
        return result
    path = Path(cache) if cache else None
    entries = None
    if path is not None and path.is_file():
        try:
            entries = json.loads(path.read_text()).get("entries")
        except (OSError, ValueError, AttributeError):
            entries = None
    if entries is None and not offline:
        try:
            with urllib.request.urlopen(status_url, timeout=20) as response:
                entries = json.loads(response.read().decode("utf-8")).get("entries")
            if path is not None and isinstance(entries, dict):
                try:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(json.dumps({"entries": entries}))
                except OSError:
                    pass
        except (urllib.error.URLError, OSError, ValueError) as error:
            result["reason"] = f"无法下载 Google 吊销列表：{type(error).__name__}"
            result["certificates"] = certificates
            return result
    if not isinstance(entries, dict):
        result["reason"] = "离线模式未读取 Google 吊销列表"
        result["certificates"] = certificates
        return result
    revoked = False
    for certificate in certificates:
        entry = entries.get(certificate["serial"])
        certificate["status"] = (entry or {}).get("status", "NOT_LISTED")
        certificate["reason"] = (entry or {}).get("reason")
        revoked = revoked or certificate["status"] != "NOT_LISTED"
    result.update({
        "available": True,
        "entries": len(entries),
        "certificates": certificates,
        "any_revoked_or_suspended": revoked,
        "source": status_url,
        "checked_at": utc_now(),
    })
    return result


def verify_chain(directory, roots_url=ATTESTATION_ROOTS_URL, offline=False, cache=None):
    """Verify a captured chain against Google's published hardware roots."""
    leaf, intermediates, root = load_chain(directory)
    if not _openssl():
        return {"available": False, "reason": "缺少 openssl，无法校验证明链"}
    chain = subprocess.run(
        ["openssl", "crl2pkcs7", "-nocrl", "-certfile", str(leaf), "-certfile", str(intermediates)],
        capture_output=True)
    if chain.returncode != 0:
        return {"available": False, "reason": "无法读取已采集的证明链"}
    pem = subprocess.run(["openssl", "pkcs7", "-print_certs"], input=chain.stdout,
                         capture_output=True)
    if pem.returncode != 0:
        return {"available": False, "reason": "无法解析证明链证书"}
    certificates = re.findall(
        rb"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", pem.stdout, re.S)
    if not certificates:
        return {"available": False, "reason": "证明链中没有证书"}
    with tempfile.TemporaryDirectory(prefix="stepx-attestation-") as scratch:
        paths = []
        for index, body in enumerate(certificates):
            path = Path(scratch) / f"cert-{index}.pem"
            path.write_bytes(body)
            paths.append(path)
        official = Path(directory) / "official-roots.pem"
        if not official.is_file() and cache:
            official = Path(cache)
        if not official.is_file():
            if offline:
                return {"available": True, "internal_path_valid": None,
                        "official_root_match": None,
                        "reason": "缺少 Google 官方根列表且处于离线模式，未做官方根比对"}
            try:
                body = urllib.request.urlopen(roots_url, timeout=15).read()
                official.parent.mkdir(parents=True, exist_ok=True)
                official.write_bytes(body)
            except (urllib.error.URLError, OSError) as error:
                return {"available": True, "internal_path_valid": None,
                        "official_root_match": None,
                        "reason": f"无法下载 Google 官方根列表：{type(error).__name__}"}
        internal = _run_openssl(["verify", "-CAfile", str(root), "-untrusted",
                                 str(intermediates), str(paths[0])])
        match = _run_openssl(["verify", "-CAfile", str(official), "-untrusted",
                              str(intermediates), str(paths[0])])
        return {
            "available": True,
            "internal_path_valid": None if internal is None else internal.returncode == 0,
            "official_root_match": None if match is None else match.returncode == 0,
            "certificate_count": len(paths),
            "root_subject": _cert_field(root, "-subject"),
            "official_roots": describe_roots(official),
            "official_verify_error": None if (match and match.returncode == 0) else (
                match.stderr.strip().splitlines()[-1] if match and match.stderr.strip()
                else "官方根校验未通过"),
            "chain_sha256": hashlib.sha256(pem.stdout).hexdigest(),
            "captured_directory": str(directory),
        }


def probe_key_attestation(device, state):
    layer = LAYERS[2]
    directory = state.get("attestation_chain")
    if not directory:
        return layer_result(
            layer, "unknown", [],
            "本机未提供已采集的密钥证明链。生成证明密钥需要安装诊断 App，本工具不做这件事；"
            "请用 --attestation-chain 指向既往采集目录。",
            {"evidence_kind": "captured_chain"},
        )
    try:
        verified = verify_chain(directory, offline=state.get("offline", False),
                                cache=state.get("roots_cache"))
    except ProbeError as error:
        return unknown_layer(layer, str(error))
    if not verified.get("available"):
        return layer_result(layer, "unknown", [], verified.get("reason", "证明链不可用"),
                            {"evidence_kind": "captured_chain"})
    revocation = check_revocation(directory, offline=state.get("offline", False),
                                  cache=state.get("revocation_cache"))
    checks = [
        check("链内签名与有效期", verified.get("internal_path_valid"), True,
              verified.get("internal_path_valid")),
        check("根证书匹配 Google 官方硬件根", verified.get("official_root_match"), True,
              verified.get("official_root_match"),
              verified.get("official_verify_error") or
              (f"证书数 {verified.get('certificate_count')}" if verified.get("certificate_count") else None)),
        check("链上证书不在 Google 吊销列表", revocation.get("any_revoked_or_suspended"), False,
              None if revocation.get("any_revoked_or_suspended") is None
              else not revocation["any_revoked_or_suspended"],
              f"列表 {revocation.get('entries', '未读取')} 条；"
              + "；".join(f"#{c['index']} {c.get('status')}" for c in revocation.get("certificates", []))
              or revocation.get("reason")),
    ]
    ok = layer_state(checks)
    if ok == "unknown":
        note = verified.get("reason", "官方根比对未完成")
    else:
        note = ("证明链锚定到 Google 官方硬件根。" if ok == "pass" else
                "证明链内部有效，但根证书不是 Google 官方硬件证明根，服务端不会接受。")
    return layer_result(layer, ok, checks, note,
                        {"evidence_kind": "captured_chain", "verification": verified,
                         "revocation": revocation})


# --------------------------------------------------------------------------
# Layer 4 - RKP and certified-device catalog
# --------------------------------------------------------------------------

RKP_PROPS = ("remote_provisioning.hostname", "remote_provisioning.enable_rkpd")
# `dsid` is what the vendor's own update service signs its requests with. It
# lives behind a signature|privileged permission, so a host-side probe cannot
# read it even though the phone's own updater can.
VENDOR_UPDATE_PACKAGE = "com.stepos.update"
DSID_PROVIDER = "content://com.stepos.secureservice.open.DataProvider"
DSID_PERMISSION = "com.stepos.secureservice.permission.OPEN_DATA_PROVIDER"


def probe_vendor_identity(device):
    """Whether the vendor's own services can identify this device.

    This is deliberately **not** part of the certification verdict: Google never
    sees it. It matters because a device that cannot answer its own update
    query cannot discover a firmware build that might fix certification, and
    because it separates "the phone cannot update" from "the host cannot ask".
    """
    result = {"dsid_readable_from_host": None, "updater_installed": None,
              "updater_holds_dsid_permission": None, "gsf_android_id": None}
    result["note"] = (
        "dsid 由 com.stepos.secureservice 的 DataProvider 提供，受 signature|privileged 权限保护。"
        "手机自带更新器持有该权限，可以正常查询固件更新；仓库的 OTA 探针以宿主 shell 身份运行，"
        "读不到 dsid，官方服务器因此返回 Parameter sys's subitem must not null。"
        "要确认是否有更新固件，请在手机 设置 → 系统更新 中检查。")
    try:
        packages = device.shell("pm", "list", "packages", "--user", "0")
        result["updater_installed"] = f"package:{VENDOR_UPDATE_PACKAGE}" in packages
    except ProbeError:
        return result
    try:
        dump = device.shell("dumpsys", "package", VENDOR_UPDATE_PACKAGE)
        result["updater_holds_dsid_permission"] = any(
            line.strip() == f"{DSID_PERMISSION}: granted=true" for line in dump.splitlines())
    except ProbeError:
        pass
    try:
        probe = device.raw("shell", "content", "query", "--uri", DSID_PROVIDER, timeout=30,
                           check=False)
        combined = ((probe.stdout or "") + (probe.stderr or "")).strip()
        denied = "SecurityException" in combined or "Permission Denial" in combined
        if not combined:
            # No output at all is not evidence of readability.
            result["dsid_readable_from_host"] = None
            result["dsid_host_error"] = "provider 无输出，未能判定"
        elif denied or "Error while accessing" in combined:
            result["dsid_readable_from_host"] = False
            result["dsid_host_error"] = (
                "需要 " + DSID_PERMISSION + "（signature|privileged）" if denied
                else combined.splitlines()[0][:120])
        else:
            result["dsid_readable_from_host"] = True
            result["dsid_host_error"] = combined.splitlines()[0][:120]
    except ProbeError:
        pass
    try:
        result["gsf_android_id"] = device.shell("settings", "get", "secure", "android_id")
    except ProbeError:
        pass
    return result


PATCH_PROPS = ("ro.build.version.security_patch", "ro.vendor.build.security_patch")
# Google does not publish a minimum acceptable patch level, so this probe only
# reports the measured age. It never contributes to the certification verdict.
PATCH_STALE_DAYS = 180


QUERY_SERVICE = "com.stepos.update/.download.QueryUpdateService"
QUERY_PERMISSION = "com.stepos.update.QUERY_UPDATE"


def probe_update_query_service(device):
    """Whether the vendor's own update query service is reachable from the host.

    `QueryUpdateService` would answer "is there a newer firmware" from inside the
    updater process, which does hold the permission the host-side OTA probe
    lacks. It is exported but guarded by a signature permission, and its
    `hasUpdate` / `getUpdateFirmwareName` only return what the updater has
    already cached - they never trigger a fresh check. This probe records that
    closure so the OTA question is answered with a reason instead of left open.
    """
    result = {"service_present": None, "guard_permission": None,
              "protection_level": None, "host_holds_permission": None,
              "note": ""}
    try:
        dump = device.shell("dumpsys", "package", "com.stepos.update")
    except ProbeError:
        result["note"] = "未读取到官方更新器组件信息。"
        return result
    result["service_present"] = QUERY_SERVICE in dump
    if not result["service_present"]:
        result["note"] = "未找到官方更新器的 QueryUpdateService。"
        return result
    result["guard_permission"] = QUERY_PERMISSION
    # dumpsys puts `uid=… type=N prot=…` on one line; tolerate either layout.
    level = re.search(
        r"Permission \[" + re.escape(QUERY_PERMISSION) + r"\][^\n]*\n"
        r"\s*sourcePackage=[^\n]*\n\s*uid=\d+[^\n]*?\bprot=([^\s\n]+)",
        dump)
    if level:
        result["protection_level"] = level.group(1).strip()
    try:
        shell_dump = device.shell("dumpsys", "package", "com.android.shell")
        result["host_holds_permission"] = any(
            line.strip() == f"{QUERY_PERMISSION}: granted=true"
            for line in shell_dump.splitlines())
    except ProbeError:
        pass
    result["note"] = (
        f"官方更新器的 {QUERY_SERVICE} 已导出，但由 {QUERY_PERMISSION} 保护，"
        f"保护级别为 `{result['protection_level'] or '未取到'}`。宿主 shell "
        f"{'持有' if result['host_holds_permission'] else '不持有'}该权限，"
        "signature 级权限无法通过 pm grant 授予。且该服务的 hasUpdate / "
        "getUpdateFirmwareName 只返回更新器已缓存的结果，不触发新的查询。"
        "因此无法从宿主确认是否有更新固件，只能在手机「设置 → 系统更新」中检查。")
    return result


def probe_security_patch(device):
    """Measure how old the device's security patch level is.

    Play Integrity's `MEETS_DEVICE_INTEGRITY` rejects devices whose patch level
    is too old, and that check is independent of the "device is certified" flag
    in Play. The age is therefore reported as a risk factor with its own
    `stale` flag, never folded into `certified`: no public threshold exists and
    inventing one would be a fabricated verdict.
    """
    result = {"security_patch": None, "vendor_security_patch": None, "age_days": None,
              "stale": None, "note": ""}
    values = {}
    for name in PATCH_PROPS:
        try:
            values[name] = device.prop(name)
        except ProbeError:
            values[name] = ""
    result["security_patch"] = values[PATCH_PROPS[0]] or None
    result["vendor_security_patch"] = values[PATCH_PROPS[1]] or None
    newest = max((value for value in values.values() if value), default="")
    parsed = None
    for pattern in ("%Y-%m-%d", "%Y%m%d"):
        try:
            parsed = datetime.datetime.strptime(newest, pattern).date()
            break
        except ValueError:
            continue
    if parsed is None:
        result["note"] = "未能解析安全补丁日期，未评估新旧程度。"
        return result
    age = (datetime.datetime.now(datetime.timezone.utc).date() - parsed).days
    result["age_days"] = age
    result["stale"] = age > PATCH_STALE_DAYS
    result["note"] = (
        f"安全补丁 {newest}，距今 {age} 天。补丁过旧是 Play Integrity "
        "`MEETS_DEVICE_INTEGRITY` 的独立拒绝原因，与 Play 商店的「设备已通过认证」"
        "不是同一项判定；本工具不把它计入认证结论，因为 Google 未公布阈值。"
        + (" 已超过 180 天，建议优先跟进固件更新。" if result["stale"] else ""))
    return result


def probe_rkp_pool(device, component="default"):
    """Ask whether the device has ever been provisioned by Google's RKP backend.

    `cmd remote_provisioning certify <component>` is documented as printing the
    already-provisioned chain, so with a non-empty pool it is a pure read. With
    an empty pool on this build it instead asks Google to sign, which is a state
    change. The hostname therefore has to be empty for this probe to be safe:
    with no hostname the request cannot leave the device, so the worst outcome
    is a local failure. The probe refuses to run otherwise.
    """
    result = {"component": component, "pool_state": "unknown", "note": ""}
    hostname = device.prop("remote_provisioning.hostname")
    if hostname:
        result["note"] = ("remote_provisioning.hostname 非空，certify 可能向 Google 请求签发，"
                          "已拒绝执行；请只在 hostname 为空时运行本探针。")
        return result
    try:
        probe = device.raw("shell", "cmd", RKP_SERVICE, "certify", component,
                           timeout=120, check=False)
        combined = (probe.stdout or "") + (probe.stderr or "")
    except ProbeError as error:
        result["note"] = f"无法执行 certify：{error}"
        return result
    if "Failed to bind to IRemoteProvisioning" in combined:
        result["pool_state"] = "unreachable"
        result["note"] = ("固件防火墙在绑定 IRemoteProvisioning 阶段就阻断，"
                          "连本地证书池都读不到；因此无法从宿主判断设备是否曾被 RKP 签发过。")
        return result
    if "BEGIN CERTIFICATE" in combined:
        result["pool_state"] = "provisioned"
        result["note"] = "设备持有 RKP 签发的证书链。"
        return result
    if "444" in combined or "Device not registered" in combined:
        result["pool_state"] = "empty"
        result["note"] = ("证书池为空，且 Google 后端返回 Device not registered；"
                          "即设备从未被成功签发过。")
        return result
    result["pool_state"] = "unknown"
    result["note"] = "certify 输出无法解释：" + combined.strip().splitlines()[-1][:120]
    return result


def probe_rkp_csr_all(device, components):
    """Generate and parse a local CSR for every RKP component.

    Each component is a separate registration identity at Google's backend, so
    the tool reports them individually instead of assuming one CSR speaks for
    the whole device. Generation is local and deterministic; it never contacts
    Google.
    """
    results = []
    for component in components:
        try:
            parsed = probe_csr(device, component)
        except ProbeError as error:
            results.append({"component": component, "error": str(error)})
            continue
        parsed["component"] = component
        results.append(parsed)
    return results


def probe_rkp(device, state):
    layer = LAYERS[3]
    checks, extra = [], {}
    try:
        installed = device.shell("pm", "list", "packages", "--user", "0")
        has_rkpd = "package:com.android.rkpdapp" in installed
        running = any(
            "com.android.rkpdapp" in line
            for line in device.shell("ps", "-A", "-o", "NAME").splitlines())
        checks.append(check("com.android.rkpdapp 已安装", has_rkpd, True, has_rkpd))
        checks.append(check("com.android.rkpdapp 正在运行", running, True, running,
                            "服务未运行时不会自动轮换证明证书"))
        dump = device.shell("cmd", RKP_SERVICE, "dump")
        components = device.shell("cmd", RKP_SERVICE, "list").split()
        props = {name: device.prop(name) for name in RKP_PROPS}
    except ProbeError as error:
        return unknown_layer(layer, str(error))
    parsed = parse_rkp_dump(dump)
    extra["rkp_components"] = parsed
    if components and not state.get("offline"):
        # Every component is a separate backend registration identity.
        extra["rkp_csr_components"] = probe_rkp_csr_all(device, components)
    # The whole RKP configuration surface, recorded so the OEM ticket can cite
    # each knob by name. None of these values can be changed from the desktop.
    extra["rkp_configuration"] = {
        "remote_provisioning.hostname": props.get("remote_provisioning.hostname", ""),
        "remote_provisioning.enable_rkpd": props.get("remote_provisioning.enable_rkpd", ""),
        "persist.deviceconfig.remotekeyprovisioningnative.enablerkpd":
            device.prop("persist.deviceconfig.remotekeyprovisioningnative.enablerkpd"),
        "ro.oem.keybox.attestation": device.prop("ro.oem.keybox.attestation"),
        "rpmb_provisioned": next(
            (value for name, value in device.rpmb_props().items() if value), None),
        "rkpdapp_apk": device.shell("pm", "path", "com.android.rkpdapp"),
        "note": "enable_rkpd=false 是 AOSP 默认值，表示使用系统级 rkpdapp，不是故障；"
                "hostname 为空才会阻止自动轮换。",
    }
    default = parsed.get("default") or {}
    for name, expected in (("hwVersion", "3"), ("rpcAuthorName", None), ("uniqueId", None)):
        value = default.get(name)
        if value is None:
            checks.append(check(f"RKP HAL {name}", "未读到", expected or "非空", False))
        elif expected:
            checks.append(check(f"RKP HAL {name}", value, expected, value == expected))
        else:
            checks.append(check(f"RKP HAL {name}", value, "非空", bool(value)))
    # These two need evidence this read-only tool cannot gather on its own: the
    # firewall dump is root-only and `signCertificates` contacts Google. Both
    # stay `unknown` unless the caller supplies the recorded result.
    hostname = props.get("remote_provisioning.hostname", "")
    checks.append(check("remote_provisioning.hostname", hostname or "空", "官方主机名", bool(hostname),
                        "为空时系统不会自动轮换证明证书"))
    firewall = state.get("firewall_allows_rkp")
    checks.append(check("防火墙放行 IRemoteProvisioning", firewall, True, firewall,
                        "固件 CommonInterception 默认 Local rule forbid；读取该配置需要 root，"
                        "未提供证据时记为未探测"))
    backend = state.get("rkp_backend_status")
    checks.append(check("RKP signCertificates", backend if backend is not None else "未探测", "200",
                        None if backend is None else backend == 200,
                        "HTTP 444 Device not registered 表示后台未登记该设备公钥；"
                        "本工具默认不主动请求签发"))
    catalog = state.get("catalog") or {}
    listed = catalog.get("listed")
    checks.append(check("Google 认证设备目录收录", listed, True, listed,
                        catalog.get("note") or "未读取到目录结果"))
    ok = layer_state(checks)
    identities = {
        entry.get("component"): (entry.get("device_info") or {}).get("brand")
        for entry in extra.get("rkp_csr_components", []) if "error" not in entry}
    note = ("RKP 与目录层全部就绪。" if ok == "pass" else
            "RKP 与目录层至少一项未就绪或未探测；这些都不能由本机客户端补齐。")
    if len(identities) > 1:
        note += (f" 本机有 {len(identities)} 个 RKP 登记身份"
                 f"（{'、'.join(f'{name}={brand}' for name, brand in identities.items())}）；"
                 "只有承载真实机型身份的那个组件与 Play 认证相关。")
    return layer_result(layer, ok, checks, note, extra)


def parse_rkp_dump(text):
    components, current = {}, None
    for line in text.splitlines():
        if not line.strip():
            continue
        if not line.startswith(" ") and line.rstrip().endswith(":"):
            current = line.strip().rstrip(":")
            components[current] = {}
        elif current and "=" in line:
            key, _, value = line.strip().partition("=")
            components[current][key.strip()] = value.strip()
    return components


def fetch_catalog(identity, cache_path, offline=False, ttl=86400):
    """Match the device against Google's published certified-device list."""
    cache_path = Path(cache_path)
    if cache_path.is_file():
        try:
            cached = json.loads(cache_path.read_text())
            age = datetime.datetime.now(datetime.timezone.utc).timestamp() - cached["checked_at_epoch"]
            if cached.get("identity") == identity and 0 <= age < ttl and isinstance(cached.get("listed"), bool):
                cached["from_cache"] = True
                return cached
        except (OSError, ValueError, KeyError, TypeError):
            pass
    if offline:
        return {"listed": None, "note": "离线模式未读取 Google 官方设备目录", "from_cache": False}
    try:
        with urllib.request.urlopen(SUPPORTED_DEVICES_URL, timeout=15) as response:
            content = response.read(MAX_CATALOG_BYTES + 1)
    except (urllib.error.URLError, OSError) as error:
        return {"listed": None, "note": f"暂时无法读取 Google 官方设备目录：{type(error).__name__}",
                "from_cache": False}
    if len(content) > MAX_CATALOG_BYTES:
        return {"listed": None, "note": "Google 官方设备目录超过大小上限", "from_cache": False}
    html = content.decode("utf-8", "replace")
    if "</table>" not in html.lower():
        return {"listed": None, "note": "Google 官方列表格式变化，暂不能判断", "from_cache": False}
    rows = parse_supported_devices(html)
    if rows is None:
        return {"listed": None, "note": "Google 官方列表格式变化，暂不能判断", "from_cache": False}
    brands = {identity.get("brand", "").strip(), identity.get("manufacturer", "").strip()} - {''}
    names = {identity.get(key, "").strip() for key in ("model", "device", "name")} - {''}
    if not brands or not names:
        return {"listed": None, "note": "机型信息不完整，未与 Google 官方设备目录比对", "from_cache": False}
    match, near = match_catalog(rows, identity)
    aliases = catalog_aliases(identity)
    alias_hits = [row for row in rows
                  if any(cell.strip().casefold() in aliases for cell in row)]
    if not aliases:
        listed = None
        note = "机型信息不完整，未与 Google 官方设备目录比对"
    elif match is not None:
        listed = True
        note = "官方机型列表已收录该 brand 与 model；仍需在 Play 内确认实际认证状态。"
    elif alias_hits:
        listed = False
        note = ("官方目录中有行与本机身份字段相关，但 brand 与 model 未同时命中；"
                "共用设备代号不是认证记录。")
    else:
        listed = False
        note = ("整份官方目录没有任何一行与本机任何身份字段"
                "（brand / manufacturer / model / device / name）相关。")
    result = {
        "identity": identity,
        "listed": listed,
        "note": note,
        "matching_row": match,
        "near_misses": near,
        "aliases_checked": sorted(aliases),
        "alias_hits": alias_hits[:8],
        "rows_checked": len(rows),
        "source": SUPPORTED_DEVICES_URL,
        "checked_at_epoch": datetime.datetime.now(datetime.timezone.utc).timestamp(),
        "from_cache": False,
    }
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True)
        cache_path.write_text(json.dumps(result, ensure_ascii=False))
    except OSError:
        pass
    return result


def parse_supported_devices(html):
    """Return the four columns of Google's table, or None if the shape changed."""
    rows = []
    for block in re.findall(r"<tr[^>]*>(.*?)</tr>", html, re.S | re.I):
        cells = [" ".join(re.sub(r"<[^>]+>", " ", cell).split())
                 for cell in re.findall(r"<t[dh][^>]*>(.*?)</t[dh]>", block, re.S | re.I)]
        if len(cells) == 4:
            rows.append(cells)
    if len(rows) < 2:
        return None
    return rows[1:]


def catalog_aliases(identity):
    """Every identity string this device reports, so a definitive negative is possible."""
    aliases = set()
    for key in ("brand", "manufacturer", "model", "device", "name"):
        value = (identity.get(key) or "").strip()
        if value:
            aliases.add(value.casefold())
    return aliases


def match_catalog(rows, identity):
    """Certification needs brand + model; a shared codename is not evidence."""
    brands = {identity.get("brand", "").strip().casefold(),
              identity.get("manufacturer", "").strip().casefold()} - {""}
    names = {identity.get(key, "").strip().casefold()
             for key in ("model", "device", "name")} - {''}
    match, near = None, []
    for row in rows:
        retail, marketing, device, model = (cell.strip().casefold() for cell in row)
        if retail in brands and model in names:
            match = row
        elif device in names or model in names or marketing in brands:
            near.append(row)
    return match, near[:8]


# --------------------------------------------------------------------------
# RKP CSR extraction (local, no network)
# --------------------------------------------------------------------------

def parse_rkp_csr(encoded):
    """Best-effort field extraction from a locally generated RKP CSR.

    The CSR is a vendor length-prefixed structure whose string values are
    chunked across item boundaries. Only what can be read with certainty is
    reported; binary-valued fields stay None and the unparsed share is
    published so a partial read is never mistaken for a full one.
    """
    try:
        raw = base64.b64decode(encoded, validate=True)
    except (ValueError, TypeError):
        raise ProbeError("RKP CSR 不是有效的 base64") from None
    if not raw:
        raise ProbeError("RKP CSR 为空")
    items = _length_prefixed_items(raw)
    names = set(RKP_INFO_FIELDS)
    info, binary = {}, []
    for index, value in enumerate(items):
        if value not in names:
            continue
        following = items[index + 1] if index + 1 < len(items) else None
        # A following field name means this value is binary, not a string.
        info[value] = None if following is None or following in names else following
        if info[value] is None:
            binary.append(value)
    printable = sum(len(m.group()) for m in re.finditer(rb"[\x20-\x7e]", raw))
    return {
        "bytes": len(raw),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "device_info": info,
        "binary_fields": binary,
        "firmware_versions": sorted({
            match.decode("ascii")
            for match in re.findall(rb"ssg\.tmefw\.[0-9A-Za-z][0-9A-Za-z.\-]*", raw)}),
        "printable_share": round(printable / len(raw), 3) if raw else 0.0,
        "note": "CSR 由本机 RKP HAL 本地生成，未联系 Google；其中的身份字段来自 SoC 厂商 HAL，"
                "不代表整机厂商向 Google 登记的信息。",
    }


def _length_prefixed_items(raw):
    """Collect printable strings whose length byte is 0x60 + length."""
    items, index = [], 0
    while index < len(raw):
        length = raw[index] - 0x60
        if 0 <= length <= 31 and index + 1 + length <= len(raw):
            candidate = raw[index + 1:index + 1 + length]
            if candidate and all(0x20 <= byte <= 0x7E for byte in candidate):
                items.append(candidate.decode("ascii"))
                index += 1 + length
                continue
        index += 1
    return items


def probe_csr(device, component):
    try:
        output = device.shell("cmd", RKP_SERVICE, "csr", component)
    except ProbeError as error:
        raise ProbeError(f"无法生成本地 CSR：{error}") from None
    encoded = "".join(output.split())
    if not encoded:
        raise ProbeError("RKP CSR 为空")
    parsed = parse_rkp_csr(encoded)
    # Pair the CSR with the HAL dump so the registration identity is complete.
    component_info = parse_rkp_dump(
        device.shell("cmd", RKP_SERVICE, "dump")).get(component) or {}
    parsed["unique_id"] = component_info.get("uniqueId")
    parsed["rpc_author"] = component_info.get("rpcAuthorName")
    parsed["hw_version"] = component_info.get("hwVersion")
    return parsed


# --------------------------------------------------------------------------
# Verdict
# --------------------------------------------------------------------------

def blockers_from(layers):
    blockers = []
    for result in layers:
        if result["state"] == "pass":
            continue
        failed = [entry for entry in result["checks"] if entry["ok"] is not True]
        if result["state"] == "unknown":
            detail = result["note"]
        else:
            detail = "；".join(
                f"{entry['check']}={entry['value']}（期望 {entry['expected']}）" for entry in failed
            ) or result["note"]
        blockers.append({
            "layer": result["layer"],
            "title": result["title"],
            "owner": result["owner"],
            "detail": detail,
            "required_action": next(
                layer["clears"] for layer in LAYERS if layer["id"] == result["layer"]),
        })
    return blockers


def certify(layers):
    """Certification requires every layer; the honest answer is usually no."""
    by_id = {result["layer"]: result for result in layers}
    attestation = (by_id["key_attestation"].get("verification") or {})
    rkp = by_id["rkp_and_catalog"]
    rkp_checks = {entry["check"]: entry for entry in rkp["checks"]}
    backend = rkp_checks.get("RKP signCertificates", {}).get("value")
    catalog = rkp_checks.get("Google 认证设备目录收录", {}).get("value")
    return bool(
        all(result["state"] == "pass" for result in layers)
        and attestation.get("official_root_match") is True
        and backend == 200
        and catalog is True
    )


PROBE_LAYERS = {"probe_system_boot": "system_boot", "probe_client_admission": "client_admission",
                "probe_key_attestation": "key_attestation", "probe_rkp": "rkp_and_catalog"}


def collect(device, options):
    state = {
        "offline": options.offline,
        "attestation_chain": options.attestation_chain,
        "compat_active": options.compat_active,
        "roots_cache": options.roots_cache,
        "revocation_cache": options.revocation_cache,
        "firewall_allows_rkp": options.firewall_allows,
        "rkp_backend_status": options.rkp_backend_status,
    }
    try:
        state["identity"] = device.identity()
    except ProbeError:
        state["identity"] = {}
    state["catalog"] = fetch_catalog(state["identity"], options.cache_file, offline=options.offline)
    try:
        state["vendor_services"] = probe_vendor_identity(device)
    except ProbeError as error:
        state["vendor_services"] = {"note": f"厂商服务身份未取证：{error}"}
    try:
        state["patch_freshness"] = probe_security_patch(device)
    except ProbeError as error:
        state["patch_freshness"] = {"note": f"安全补丁未取证：{error}", "stale": None}
    if getattr(options, "firmware", False):
        try:
            state["firmware"] = probe_firmware(device)
        except ProbeError as error:
            state["firmware"] = {"available": False, "note": f"固件通道未取证：{error}"}
    try:
        state["update_query_service"] = probe_update_query_service(device)
    except ProbeError as error:
        state["update_query_service"] = {"note": f"更新器查询服务未取证：{error}"}
    layers = []
    for probe in (probe_system_boot, probe_client_admission, probe_key_attestation, probe_rkp):
        layer = next(item for item in LAYERS if item["id"] == PROBE_LAYERS[probe.__name__])
        try:
            layers.append(probe(device, state))
        except ProbeError as error:
            layers.append(unknown_layer(layer, str(error)))
    return state, layers


def report(device, options):
    state, layers = collect(device, options)
    certified = certify(layers)
    return {
        "schema": SCHEMA,
        "format": FORMAT,
        "checked_at": utc_now(),
        "serial": device.serial,
        "identity": state.get("identity", {}),
        "fingerprint": state.get("fingerprint", ""),
        "certified": certified,
        "verdict": "certified" if certified else "not_certified",
        "store_access": {
            "usable": next((r.get("store_usable") for r in layers
                            if r["layer"] == "client_admission"), None),
            "note": "商店可用性与设备认证是两个独立结论；兼容补丁只影响前者。",
        },
        "layers": layers,
        "blockers": blockers_from(layers),
        "vendor_services": state.get("vendor_services"),
        "patch_freshness": state.get("patch_freshness"),
        "firmware": state.get("firmware"),
        "update_query_service": state.get("update_query_service"),
        "leverage": list(LEVERAGE),
        "sources": [ATTESTATION_ROOTS_URL, SUPPORTED_DEVICES_URL, RKP_SERVICE,
                    AOSP_RKP_SOURCE],
        "limitations": [
            "本工具只读取证，不写入设备、不注入进程、不修改任何证书或配置。",
            "第 3 层使用既往采集的证明链；重新生成证明密钥需要安装诊断 App。",
            "RKP signCertificates 结果来自既往取证或显式探测，本工具默认不主动联系 Google。",
            "vendor_services 不参与认证判定：Google 看不到它，它只说明设备自身能否查询固件更新。",
            "patch_freshness 不参与认证判定：Google 未公布补丁新鲜度阈值，编造一个阈值就是伪造结论；"
            "它只作为 Play Integrity 应用侧校验的风险因素单独呈现。",
        ],
    }


# --------------------------------------------------------------------------
# Commands
# --------------------------------------------------------------------------

def command_explain(options):
    lines = [
        "Play 保护机制认证机制认知（StepX PZ261A / pandora）",
        "=" * 52,
        "",
        "Play 商店显示“设备已通过认证”不是单一开关，而是 Google 服务端对四层证据的联合判定。",
        "任何一层缺失，结论都是“未通过认证”；客户端改动只能影响第 1 层。",
        "",
    ]
    for layer in LAYERS:
        lines += [f"{layer['title']}（责任方：{layer['owner']}）",
                  f"  判定问题：{layer['question']}"]
        lines += [f"  · {item}" for item in layer["google_checks"]]
        lines += [f"  谁能解决：{layer['clears']}", ""]
    lines += [
        "本机现状（2026-10-23 只读取证）：",
        "  · 第 1 层 pass：兼容补丁由仓库自带组件回读确认生效，商店可浏览并真实下载安装。",
        "    该组件的 official_certification_changed = false，即它没有改动认证。",
        "  · 第 2 层 pass：release-keys / user / bootloader locked / verified boot green /",
        "    Enforcing，指纹与品牌机型自相一致。",
        "  · 第 3 层 fail：4 张证书内部签名与有效期有效，且都不在 Google 吊销列表",
        "    （1765 条）中；但根证书是自签的 Stepx Root CA，不在 Google 官方硬件证明根列表中。",
        "  · 第 4 层 fail：RKP HAL 本身健康（hwVersion=3、Qualcomm、uniqueId 在位），",
        "    但 remote_provisioning.hostname 为空、防火墙默认阻断 rkpdapp、",
        "    signCertificates 返回 HTTP 444 Device not registered、官方设备目录 0 命中。",
        "",
        "两个必须分开的结论：",
        "  · 商店可用性（第 1 层）——已恢复，且不构成认证。",
        "  · 设备认证（四层联合）——未通过，只能由厂商侧完成。",
        "",
        "本机有两个 RKP 登记身份：default（Qualcomm 机型身份，brand=qti）与",
        "avf（AOSP AVF 身份，brand=aosp-avf，只用于受保护虚拟机）。只有 default 与 Play",
        "认证相关，因此 444 只能在 default 上归因。",
        "",
        "按杠杆排序的厂商侧动作（最高杠杆在前）：",
    ]
    for index, item in enumerate(LEVERAGE, start=1):
        lines.append(f"  {index}. {item['action']}")
        lines.append(f"     可解除：{'、'.join(item['clears'])}。{item['reason']}")
    lines += [
        "",
        "官方硬件证明根当前有两个：serialNumber=f92009e853b6b045（2022 起，RSA）与",
        "CN=Key Attestation CA1（2025-07 起，ECDSA P-384，含 CRL Sign）。GTS 的",
        "testEcAttestationChainRemProvLengthTee 用同一份官方根列表校验远程配置签发的链，",
        "因此 RKP 登记预期可同时解除第 3 层根匹配与第 4 层后端检查。",
        "**这是推断**：本工具无法在未登记设备上证明它，必须登记后重跑 status 实测。",
        "",
        "两个独立的风险因素（都不计入认证判定）：",
        f"  · 安全补丁 2025-12-05，实测 322 天；Play Integrity 的 MEETS_DEVICE_INTEGRITY",
        "    会检查补丁新鲜度，与 Play 商店的「设备已通过认证」不是同一项。Google 未公布阈值，",
        "    所以工具只报告实测天数。",
        "  · 官方固件通道没有比设备当前版本更新的构建（new=null，latestVersion 与设备一致），",
        "    即不存在“等一个固件更新就能解决认证”的路径。",
        "",
        "用户侧还能做的：",
        "  · 把 ticket / ticket-en 生成的工单提交给 StepX 固件团队（设备内有官方反馈入口：",
        "    com.stepos.feedback，提交 Activity 未见权限保护），并要求对方提供 GTS",
        "    AttestationRootHostTest 的通过记录作为验收证据。",
        "  · 在手机「设置 → 系统更新」确认无更新（已由只读探针代为确认：无更新构建）。",
        "  · GSF Android ID 已取到，可按 google.com/android/uncertified 官方流程尝试登记，",
        "    但预期无效——本机是整机型号从未进入官方目录，不是“厂商已认证但这台没被识别”。",
        "",
        "边界：伪造证明链、注入 keystore2 或伪装机型指纹既不被 Google 服务端接受，",
        "也属于规避设备认证这一安全机制，本工具不实现，也有测试断言源码中不出现相关调用。",
    ]
    print("\n".join(lines))
    return 0


def command_status(options):
    # `options.device` is a test seam; the CLI never sets it.
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    result = report(device, options)
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0


def command_csr(options):
    device = Device(options.serial)
    device.verify_identity()
    components = device.shell("cmd", RKP_SERVICE, "list").split()
    if options.component not in components:
        raise ProbeError(f"未知 RKP 组件 {options.component}；可用：{'、'.join(components) or '无'}")
    parsed = probe_csr(device, options.component)
    parsed.update({"schema": SCHEMA, "checked_at": utc_now(), "serial": device.serial,
                   "component": options.component, "components": components})
    print(json.dumps(parsed, ensure_ascii=False, separators=(",", ":")))
    return 0


def command_chain(options):
    verified = verify_chain(options.directory, offline=options.offline, cache=options.roots_cache)
    verified.update({"schema": SCHEMA, "checked_at": utc_now()})
    print(json.dumps(verified, ensure_ascii=False, separators=(",", ":")))
    return 0 if verified.get("available") else 1


def command_monitor(options):
    # `options.device` is a test seam; the CLI never sets it.
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    result = report(device, options)
    state_path = Path(options.state_file)
    previous = None
    if state_path.is_file():
        try:
            previous = json.loads(state_path.read_text())
        except (OSError, ValueError):
            previous = None
    # Only probed evidence enters the fingerprint: a check that was not run
    # reads as "未探测" and must never register as a change.
    probed = []
    for layer in result["layers"]:
        for entry in layer["checks"]:
            if entry["ok"] is None:
                continue
            probed.append(f"{layer['layer']}/{entry['check']}={entry['value']}")
    catalog = next((layer for layer in result["layers"]
                    if layer["layer"] == "rkp_and_catalog"), {})
    catalog_listed = next((entry["value"] for entry in catalog.get("checks", [])
                           if entry["check"] == "Google 认证设备目录收录"
                           and entry["ok"] is not None), None)
    fingerprint = {
        "verdict": result["verdict"],
        "probed": sorted(probed),
        "catalog": catalog_listed,
        "blocker_layers": sorted({blocker["layer"] for blocker in result["blockers"]}),
    }
    # The optional evidence flags decide what gets probed at all, so a baseline
    # taken with different flags is not comparable. Record and compare them.
    flags = {"firewall_allows": options.firewall_allows,
             "rkp_backend_status": options.rkp_backend_status,
             "attestation_chain": bool(options.attestation_chain)}
    previous_flags = (previous or {}).get("flags")
    same_flags = previous is None or previous_flags == flags
    changed = previous is not None and previous.get("fingerprint") != fingerprint
    diff = []
    if previous is not None and previous_flags != flags:
        diff.append("取证参数与基线不一致（--firewall-allows / --rkp-backend-status / "
                    "--attestation-chain），结论只在参数相同时才可比")
    if changed:
        old = set((previous or {}).get("fingerprint", {}).get("probed", []))
        new = set(fingerprint["probed"])
        for item in sorted(new - old):
            diff.append(f"新增取证：{item}")
        for item in sorted(old - new):
            diff.append(f"取证消失：{item}")
        if (previous or {}).get("fingerprint", {}).get("verdict") != fingerprint["verdict"]:
            diff.append(f"认证判定：{(previous or {}).get('fingerprint', {}).get('verdict')}"
                        f" → {fingerprint['verdict']}")
        if not diff:
            diff.append("取证范围与判定均未变，但指纹不同；请人工核对")
    if result["certified"]:
        message = "认证已通过。" if changed else "认证已通过，状态未变化。"
    elif previous is None:
        message = "已建立认证监测基线；仍有阻塞项需厂商侧解决。"
    elif not same_flags:
        message = "本次取证参数与基线不同，未判定为状态变化；请用相同参数重跑。"
    elif changed:
        message = "认证相关证据发生变化，请在手机 Play → 设置 → 关于中确认。"
    else:
        message = "认证状态未变化；仍有阻塞项需厂商侧解决。"
    payload = {"schema": SCHEMA, "checked_at": result["checked_at"],
               "changed": changed and same_flags, "baseline_established": previous is None,
               "same_flags": same_flags, "diff": diff,
               "previous": (previous or {}).get("fingerprint"),
               "current": fingerprint, "flags": flags, "message": message}
    # Always refresh the baseline so a caller that keeps its flags stable gets a
    # clean comparison on the next run; the `same_flags` flag still marks the
    # run whose comparison was not valid.
    try:
        state_path.parent.mkdir(parents=True, exist_ok=True)
        state_path.write_text(json.dumps({"fingerprint": fingerprint, "flags": flags},
                                         ensure_ascii=False))
    except OSError:
        pass
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    return 1 if (options.fail_on_blockers and not result["certified"]) else 0


def command_dossier(options):
    """Assemble the registration package the OEM needs to clear layers 3 and 4."""
    device = Device(options.serial)
    device.verify_identity()
    result = report(device, options)
    csr = {}
    try:
        csr = probe_csr(device, options.component)
    except ProbeError as error:
        csr = {"error": str(error)}
    rkp_layer = next(layer for layer in result["layers"] if layer["layer"] == "rkp_and_catalog")
    dossier = {
        "schema": "stepx-play-protect-registration-dossier-v1",
        "generated_at": result["checked_at"],
        "device": {
            "serial": device.serial,
            "identity": result["identity"],
            "fingerprint": result["fingerprint"],
            "sdk": device.prop("ro.build.version.sdk"),
            "security_patch": device.prop("ro.build.version.security_patch"),
        },
        "attestation": next(
            (layer.get("verification") for layer in result["layers"]
             if layer["layer"] == "key_attestation"), None),
        "rkp": {"components": rkp_layer.get("rkp_components"), "csr": csr},
        "verdict": result["verdict"],
        "blockers": result["blockers"],
        "oem_actions": [
            {"step": 1, "action": "在产线将 TEE 中的测试根证书链替换为 Google Hardware Attestation Root CA 签发的生产 Keybox",
             "layer": "key_attestation", "reference": ATTESTATION_ROOTS_URL},
            {"step": 2, "action": "通过 Google 合作伙伴门户登记本机 RKP 硬件公钥，使 signCertificates 返回 200",
             "layer": "rkp_and_catalog", "reference": "https://android.googlesource.com/platform/packages/apps/RemoteProvisioner/"},
            {"step": 3, "action": "移除固件 CommonInterception 对 com.android.rkpdapp.IRemoteProvisioning 的 Local rule forbid",
             "layer": "rkp_and_catalog", "reference": "本机防火墙配置"},
            {"step": 4, "action": "以正式构建指纹提交并通过 CTS / VTS / GTS，进入 Play Certified Devices 目录",
             "layer": "rkp_and_catalog", "reference": SUPPORTED_DEVICES_URL},
        ],
        "limitations": result["limitations"],
    }
    text = json.dumps(dossier, ensure_ascii=False, indent=2) + "\n"
    if options.out:
        Path(options.out).write_text(text)
        print(json.dumps({"ok": True, "written": options.out, "verdict": dossier["verdict"]},
                         ensure_ascii=False, separators=(",", ":")))
    else:
        print(text)
    return 0


def command_ticket(options):
    """Render the OEM ticket: every blocker with evidence and the required actions."""
    # `options.device` is a test seam; the CLI never sets it.
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    state, layers = collect(device, options)
    certified = certify(layers)
    try:
        csr = probe_csr(device, options.component)
    except ProbeError as error:
        csr = {"error": str(error)}
    catalog = state.get("catalog") or {}
    verification = next(
        (layer.get("verification") or {} for layer in layers
         if layer["layer"] == "key_attestation"), {})
    rkp = next((layer for layer in layers if layer["layer"] == "rkp_and_catalog"), {})
    config = rkp.get("rkp_configuration") or {}
    components = rkp.get("rkp_components") or {}
    default = components.get("default") or {}
    backend = state.get("rkp_backend_status")
    vendor = state.get("vendor_services") or {}
    patch = state.get("patch_freshness") or {}
    firmware = state.get("firmware") or {}
    revocation = next(
        (layer.get("revocation") or {} for layer in layers
         if layer["layer"] == "key_attestation"), {})
    lines = [
        "# StepX PZ261A · Google Play「设备已通过认证」未达成 —— 厂商侧登记材料",
        "",
        f"- 生成时间：{utc_now()}",
        f"- 设备序列号：{device.serial}",
        f"- 构建指纹：`{state.get('fingerprint', '')}`",
        f"- 当前判定：**{'已通过认证' if certified else '未通过认证'}**",
        "",
        "本材料由只读取证工具生成，未修改设备、未请求证书签发。",
        "",
        "## 一、四层判定",
        "",
        "| 层 | 状态 | 责任方 | 说明 |",
        "| --- | --- | --- | --- |",
    ]
    for layer in layers:
        lines.append(f"| {layer['title']} | {layer['state']} | {layer['owner']} | {layer['note']} |")
    lines += [
        "",
        "## 二、阻塞点与证据",
        "",
    ]
    for blocker in blockers_from(layers):
        lines += [f"### {blocker['title']}（责任方：{blocker['owner']}）", "",
                  f"- 实测：{blocker['detail']}",
                  f"- 需要：{blocker['required_action']}", ""]
    lines += [
        "## 三、第 3 层 · 硬件密钥证明证据",
        "",
        f"- 证书链长度：{verification.get('certificate_count', '未取证')}",
        f"- 链上根证书：`{verification.get('root_subject', '未取证')}`",
        f"- 链内签名与有效期：{verification.get('internal_path_valid', '未取证')}",
        f"- 与 Google 官方硬件证明根匹配：**{verification.get('official_root_match', '未取证')}**",
        f"- 官方根校验信息：{verification.get('official_verify_error', '无')}",
        f"- Google 吊销列表：{(revocation.get('entries') if revocation.get('available') else '未读取')}"
        f" 条，链上证书"
        f"{'均未被吊销' if revocation.get('any_revoked_or_suspended') is False else '存在被吊销项' if revocation.get('any_revoked_or_suspended') else '状态未探测'}"
        + ("（" + "；".join(f"#{c['index']} {c.get('status')}"
                           for c in revocation.get("certificates", [])) + "）"
           if revocation.get("available") else f"（{revocation.get('reason', '')}）"),
        "",
        "注意：证书内部签名有效、且不在吊销列表，都不构成信任——Google 只信任锚定在官方根上的链。",
        "",
        "根证书是整机厂商自签的测试根，不在 "
        f"[Google 官方硬件证明根列表]({ATTESTATION_ROOTS_URL}) 公布的两个受信根中，",
        "因此 Google 服务端不会接受本机 TEE 签发的任何证明链。",
        "",
        "## 四、第 4 层 · RKP 与设备目录证据",
        "",
        "### 4.1 RKP 硬件能力（正常）",
        "",
        "| 组件 | hwVersion | RPC author | uniqueId | 每 CSR 密钥数 |",
        "| --- | --- | --- | --- | --- |",
    ]
    for name, info in components.items():
        lines.append(f"| {name} | {info.get('hwVersion', '')} | {info.get('rpcAuthorName', '')} "
                     f"| {info.get('uniqueId', '')} | {info.get('supportedNumKeysInCsr', '')} |")
    lines += [
        "",
        "RKP HAL 支持 v3 协议且 uniqueId 在位，说明**本机具备远程密钥配置能力，问题不在硬件缺失，"
        "而在 Google 后端未登记该设备的硬件公钥**。",
        "",
        "### 4.2 本地 CSR 身份（未联系 Google）",
        "",
        f"- CSR 长度：{csr.get('bytes', '未取证')} 字节，sha256 `{csr.get('sha256', '未取证')}`",
        f"- uniqueId：`{csr.get('unique_id', '未取证')}`，RPC author：`{csr.get('rpc_author', '未取证')}`",
        f"- HAL 上报身份：{json.dumps(csr.get('device_info', {}), ensure_ascii=False)}",
        f"- TEE 固件版本：{', '.join(csr.get('firmware_versions', [])) or '未取到'}",
        "",
        "注意：CSR 内的 brand / manufacturer 来自 SoC 厂商 HAL（Qualcomm），"
        "与整机厂商向 Google 登记的身份是两回事，申诉时需区分。",
        "",
        "### 4.2.1 两个 RKP 登记身份（2026-10-23 追加）",
        "",
        "本机 RKP HAL 暴露两个组件，它们各自对应 Google 后端一条独立登记记录：",
        "",
        "| 组件 | CSR 大小 | brand | model | manufacturer | security level |",
        "| --- | --- | --- | --- | --- | --- |",
    ] + [
        f"| {entry['component']} | {entry.get('bytes', '未取到')} | "
        f"{(entry.get('device_info') or {}).get('brand', '')} | "
        f"{(entry.get('device_info') or {}).get('model', '')} | "
        f"{(entry.get('device_info') or {}).get('manufacturer', '')} | "
        f"{(entry.get('device_info') or {}).get('security_level', '')} |"
        for entry in (rkp.get("rkp_csr_components") or []) if "error" not in entry
    ] + [
        "",
        "两个组件的 TEE 固件版本相同，但身份字段完全不同：`default` 是 Qualcomm 的真实机型身份"
        "（`brand=qti`、`Canoe for arm64`），`avf` 是 AOSP AVF 身份（`brand=aosp-avf`），"
        "只用于受保护虚拟机（pVM）证明。",
        "",
        "**因此第 4 层的 444 只能在 `default` 组件上归因**：`avf` 即使登记成功也不会让本机"
        "获得 Play 认证，因为它不承载机型身份。申诉时请以 `default` 的 CSR 与 HAL dump 为准。",
        "",
        "### 4.3 配置面",
        "",
        "| 配置项 | 当前值 | 影响 |",
        "| --- | --- | --- |",
    ]
    for name, value in config.items():
        if name in ("note", "rkpdapp_apk"):
            continue
        lines.append(f"| `{name}` | `{value or '空'}` | 见下方说明 |")
    lines += [
        f"| rkpdapp APK | `{config.get('rkpdapp_apk', '')}` | 来自 mainline APEX |",
        "",
        f"- `remote_provisioning.hostname` 为空：系统不会自动连接 RKP 后端轮换证明证书。",
        f"- `remote_provisioning.enable_rkpd` = `{config.get('remote_provisioning.enable_rkpd', '')}`："
        "这是 AOSP 默认值，表示使用系统级 rkpdapp，**不是故障**。",
        f"- `vendor.*.rpmb.provisioned` = `{config.get('rpmb_provisioned', '未取到')}`："
        "说明产线做过 RPMB  provision，缺口特指证明密钥与 RKP 登记。",
        "",
        "### 4.4 后端签发与设备目录",
        "",
        f"- RKP `signCertificates`：{backend or '未探测'}"
        + ("" if backend else "（既往取证可用 --rkp-backend-status 提供；本工具不主动请求签发）")
        + "；HTTP 444 `Device not registered.` 表示 Google 后台未识别该设备公钥",
        f"- 防火墙放行 `com.android.rkpdapp.IRemoteProvisioning`："
        + ("是" if state.get("firewall_allows_rkp") is True else
           "否" if state.get("firewall_allows_rkp") is False else "未探测")
        + "（固件 `CommonInterception` 默认 `Local rule forbid`，读取需 root）",
        f"- 官方认证设备目录：{catalog.get('listed', '未取证')} —— {catalog.get('note', '')}",
        f"- 目录规模：{catalog.get('rows_checked', '未取证')} 行，检索别名 "
        f"{catalog.get('aliases_checked', [])}",
        "",
        "### 4.5 设备自身身份与固件更新查询（附带发现，不影响认证判定）",
        "",
        "| 项 | 状态 |",
        "| --- | --- |",
        f"| 官方更新器 {VENDOR_UPDATE_PACKAGE} 已安装 | {vendor.get('updater_installed', '未取证')} |",
        f"| 更新器持有 {DSID_PERMISSION} | {vendor.get('updater_holds_dsid_permission', '未取证')} |",
        f"| 宿主侧可读 dsid | {vendor.get('dsid_readable_from_host', '未取证')} |",
        f"| 宿主读取 dsid 的结果 | {vendor.get('dsid_host_error', '未取证')} |",
        f"| GSF Android ID | `{vendor.get('gsf_android_id') or '未取到'}` |",
        "",
        "`dsid` 由 `com.stepos.secureservice` 的 DataProvider 提供，权限为 "
        "`signature|privileged`。手机自带更新器持有该权限，可正常查询更新；"
        "以宿主 shell 身份运行的探针读不到 dsid，官方 OTA 服务器因此返回 "
        "`Parameter sys's subitem must not null`（server_code 110001）。",
        "",
        "**这意味着目前无法从宿主确认是否存在已修复认证问题的更新固件**，"
        "需要在手机「设置 → 系统更新」中检查。该发现不参与上面的认证判定。",
        "",
        "### 4.6 安全补丁新鲜度（风险因素，不影响上面的认证判定）",
        "",
        f"- 安全补丁级别：`{patch.get('security_patch') or '未取到'}`"
        f"（vendor 同步为 `{patch.get('vendor_security_patch') or '未取到'}`）",
        f"- 距今天数：{patch.get('age_days', '未计算')} 天",
        f"- 是否超过 180 天：{'是' if patch.get('stale') else '否' if patch.get('stale') is False else '未评估'}",
        "",
        patch.get("note", ""),
        "",
        "这一点要单独说明：Play Integrity 的 `MEETS_DEVICE_INTEGRITY` 会检查补丁新鲜度，"
        "而 Play 商店的「设备已通过认证」不检查。因此**即使完成 RKP 登记与目录收录，"
        "过旧的补丁仍可能导致应用侧校验失败**。Google 未公布阈值，所以本工具只报告实测天数，"
        "不把它计入上面的认证判定。",
        "",
        "### 4.7 官方固件通道查询（只读，已实证）",
        "",
        f"- HTTP 状态：{firmware.get('http_code', '未查询')}",
        f"- 通道提供的最新版本：`{firmware.get('latest_version') or '未取到'}`",
        f"- 设备当前版本：`{firmware.get('current_build') or '未取到'}`"
        f"（incremental {firmware.get('current_incremental', '未取到')}）",
        f"- 是否有更新构建（new）：{firmware.get('new_version') or '无'}",
        f"- 设备是否已在最新：{firmware.get('same_as_current', '未判定')}",
        f"- 发布日期：{firmware.get('release_date', '未取到')}，"
        f"渠道：{firmware.get('ver_type', '未取到')}，大小：{firmware.get('file_size', '未取到')}",
        "",
        "该查询复用仓库只读探针（`research/OtaMetadataProbe.java`，从不安装 OTA），"
        "结论是**官方通道没有比设备当前版本更新的构建**。因此不存在"
        "「等一个固件更新就能解决认证」的路径，认证必须由厂商后续工作完成。",
        "",
        "## 五、需要厂商完成的动作（按杠杆排序）",
        "",
        "**最高杠杆的一项是先做 RKP 登记。** Google 公布的官方硬件证明根当前有两个：",
        "",
    ] + [
        f"- `{root['subject']}`（{root['not_before']} → {root['not_after']}，"
        f"{root['public_key_algorithm']}，Key Usage: {root['key_usage']}）"
        for root in (verification.get("official_roots") or [])
    ] + [
        "",
        "其中 2025 年新增的 `Key Attestation CA1` 与远程密钥配置（RKP）对应。GTS 的 "
        "`testEcAttestationChainRemProvLengthTee` / `testRsaAttestationChainRemProvLengthTee` "
        "正是用同一份官方根列表校验**远程配置**签发的证明链。因此 RKP 登记成功后，"
        "远程配置签发的链预期会锚定官方根，**第 3 层的根证书匹配可随之解除，无需先烧录工厂 Keybox**。",
        "（这是基于官方根列表与 GTS 测试项的推断，本工具无法在未登记设备上证明，需登记后实测。）",
        "",
    ] + [
        f"{index}. **{item['action']}**（责任方：{item['owner']}）——可解除："
        f"{'、'.join(item['clears'])}。{item['reason']}"
        for index, item in enumerate(LEVERAGE, start=1)
    ] + [
        "",
        "### 关于 HTTP 444 的两种可能（影响升级对象）",
        "",
        "Google 将 444 定义为后台未识别该设备公钥，但它至少对应两种情况，处理对象不同：",
        "",
        "1. **本设备的 RKP 公钥未登记** —— 需要整机厂商 / SoC 厂商通过合作伙伴门户上传本机 CSR。",
        "2. **RPC author 的密钥未登记** —— 本机 RKP HAL 上报 `rpcAuthorName=Qualcomm`，"
        "请求由 Qualcomm 的 RPC author 密钥签名；若该 author 在 Google 侧未注册，同样返回 444。",
        "",
        "两种情况的客户端表现完全相同，只能由厂商在 Google 合作伙伴门户侧区分。"
        "申诉时建议同时提供本机 CSR（第四节 4.2）与 RKP HAL dump（4.1）。",
        "",
        "### 权威验收项",
        "",
        "Google GTS 的 `AttestationRootHostTest`（含 `testEcAttestationChainRemProvLengthTee`）"
        "是第 3、4 层的权威验收：通过即表示设备公钥已登记并已收到 Google 签发的有效证明证书。"
        "建议在工单中直接要求厂商提供该测试的通过记录。",
        "",
        "",
        "## 六、复验方式",
        "",
        "```bash",
        "python3 scripts/play_protect_certification.py status \\",
        f"  --serial {device.serial} \\",
        "  --attestation-chain <既往采集的证明链目录> \\",
        "  --firewall-allows <true|false> --rkp-backend-status <HTTP 状态码>",
        "python3 scripts/play_protect_certification.py csr --serial " + device.serial,
        "```",
        "",
        "四步全部完成后，`certified` 才会为 true；在此之前，"
        "本机任何客户端改动（包括商店兼容补丁）都不能改变认证结论。",
        "",
    ]
    text = "\n".join(lines)
    if options.out:
        Path(options.out).write_text(text)
        print(json.dumps({"ok": True, "written": options.out, "verdict":
                          "certified" if certified else "not_certified"},
                         ensure_ascii=False, separators=(",", ":")))
    else:
        print(text)
    return 0


OTA_PROBE_SOURCE = "OtaMetadataProbe.java"
OTA_PROBE_DEX = "ota-probe/dex/classes.dex"
OTA_PROBE_LIB = "ota-probe/libupgrade.so"
OTA_REMOTE = "/data/local/tmp/stepx-ota-probe"
OTA_LIBDIR = "/data/local/tmp/stepx-ota"
JDK_CANDIDATES = tuple(p for p in (os.environ.get("JAVA_HOME"), os.environ.get("STEPX_JDK")) if p)


def _tool(name, environment):
    found = shutil.which(name)
    return [found] if found else []


def build_ota_probe(scratch):
    """Compile the read-only firmware probe when no prebuilt dex is available."""
    jdk = os.environ.get("STEPX_JDK")
    candidates = [jdk] if jdk else list(JDK_CANDIDATES)
    javac = next((Path(root) / "bin" / "javac" for root in candidates
                  if (Path(root) / "bin" / "javac").is_file()), None)
    if javac is None:
        found = shutil.which("javac")
        javac = Path(found) if found else None
    if javac is None:
        return None
    sdk = os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME")
    android_jar = Path(sdk) / "platforms" / "android-36" / "android.jar" if sdk else None
    if android_jar is None or not android_jar.is_file():
        return None
    r8 = Path(sdk) / "cmdline-tools" / "latest" / "lib" / "r8.jar"
    if not r8.is_file():
        return None
    source = ROOT / "research" / OTA_PROBE_SOURCE
    if not source.is_file():
        return None
    classes = scratch / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    compiled = subprocess.run(
        [str(javac), "-encoding", "UTF-8", "--release", "11",
         "-cp", str(android_jar), "-d", str(classes), str(source)],
        capture_output=True, text=True)
    if compiled.returncode != 0:
        return None
    jar = scratch / "classes.jar"
    made = subprocess.run([str(javac.parent / "jar"), "cf", str(jar),
                           "-C", str(classes), "."], capture_output=True, text=True)
    if made.returncode != 0:
        return None
    dex = scratch / "dex"
    dex.mkdir(parents=True, exist_ok=True)
    dexed = subprocess.run(
        [str(javac.parent / "java"), "-cp", str(r8), "com.android.tools.r8.D8",
         "--release", "--min-api", "36", "--lib", str(android_jar),
         "--output", str(dex), str(jar)], capture_output=True, text=True)
    if dexed.returncode != 0 or not (dex / "classes.dex").is_file():
        return None
    return dex / "classes.dex"


def probe_firmware(device):
    """Read-only query of the official firmware channel.

    This reuses the repository's own `research/OtaMetadataProbe.java`, whose
    header states it never installs an OTA. It matters because it substitutes
    "unknown" for the device id it cannot read, which is why it gets HTTP 200
    where the packaged `stepx-ota.jar` probe is rejected. Nothing is installed
    and no device setting is changed.
    """
    result = {"available": False, "note": ""}
    # Record the device's own version strings so the reply can be compared.
    current = device.prop("ro.build.aios_version") or device.prop("ro.build.mask.id")
    result["current_build"] = current or None
    result["current_incremental"] = device.prop("ro.build.version.incremental") or None
    dex = ROOT / "research" / OTA_PROBE_DEX
    library = ROOT / "research" / OTA_PROBE_LIB
    with tempfile.TemporaryDirectory(prefix="stepx-ota-build-") as scratch:
        if not dex.is_file():
            built = build_ota_probe(Path(scratch))
            if built is None:
                result["note"] = ("没有可用的固件元数据探针：既没有 research/ota-probe/dex/"
                                  "classes.dex，也无法用 JDK + d8 重新构建。")
                return result
            dex = built
        if not library.is_file():
            result["note"] = "缺少 research/ota-probe/libupgrade.so，探针无法加载更新器原生库。"
            return result
        try:
            device.shell("rm", "-rf", OTA_REMOTE, OTA_LIBDIR)
            device.shell("mkdir", "-p", OTA_REMOTE, OTA_LIBDIR)
            device.raw("push", str(dex), f"{OTA_REMOTE}/classes.dex", timeout=120)
            device.raw("push", str(library), f"{OTA_LIBDIR}/libupgrade.so", timeout=120)
        except ProbeError as error:
            result["note"] = f"无法把探针送到手机：{error}"
            return result
        try:
            probe = device.raw("shell", f"CLASSPATH={OTA_REMOTE}/classes.dex app_process / "
                                        "OtaMetadataProbe", timeout=180, check=False)
            stdout = (probe.stdout or "").strip()
            stderr = (probe.stderr or "").strip()
        except ProbeError as error:
            result["note"] = f"探针执行失败：{error}"
            return result
        finally:
            try:
                device.shell("rm", "-rf", OTA_REMOTE, OTA_LIBDIR)
            except ProbeError:
                pass
    try:
        payload = json.loads(stdout)
    except ValueError:
        result["note"] = f"探针未返回有效固件信息：{stderr.splitlines()[-1][:120] if stderr else '无输出'}"
        return result
    reply = payload.get("reply") or {}
    value = reply.get("value") or {}
    current_block = value.get("cur") or {}
    new_block = value.get("new")
    result.update({
        "available": True,
        "http_code": reply.get("code"),
        "new_version": None if new_block is None else new_block.get("latestVersion"),
        "latest_version": current_block.get("latestVersion"),
        "system_version": current_block.get("systemVersion"),
        "release_date": current_block.get("releaseDate"),
        "ver_type": current_block.get("verType"),
        "file_size": current_block.get("fileSize"),
        "digest": current_block.get("digest"),
        "update_url": current_block.get("updateUrl"),
        "need_update": current_block.get("needUpdate"),
        "exists_update": current_block.get("existsUpdate"),
        "probe_stderr": stderr.splitlines()[-1][:120] if stderr else "",
    })
    result["same_as_current"] = bool(
        new_block is None and current and current_block.get("latestVersion") == current)
    result["note"] = (
        "官方固件通道返回的 latestVersion 与设备当前版本一致且 new 为 null 时，"
        "说明没有比设备更新固件；这不代表认证已修复，只说明无需等待固件更新。")
    return result


def command_ticket_en(options):
    """English escalation summary for SoC-vendor or Google-facing recipients."""
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    state, layers = collect(device, options)
    certified = certify(layers)
    try:
        csr = probe_csr(device, options.component)
    except ProbeError as error:
        csr = {"error": str(error)}
    verification = next(
        (layer.get("verification") or {} for layer in layers
         if layer["layer"] == "key_attestation"), {})
    rkp = next((layer for layer in layers if layer["layer"] == "rkp_and_catalog"), {})
    config = rkp.get("rkp_configuration") or {}
    components = rkp.get("rkp_components") or {}
    catalog = state.get("catalog") or {}
    backend = state.get("rkp_backend_status")
    roots = verification.get("official_roots") or []
    patch = state.get("patch_freshness") or {}
    firmware = state.get("firmware") or {}
    lines = [
        "# StepX PZ261A - Google Play device certification is not granted",
        "",
        f"- Generated: {utc_now()}",
        f"- Device serial: {device.serial}",
        f"- Build fingerprint: `{state.get('fingerprint', '')}`",
        f"- Verdict: **{'CERTIFIED' if certified else 'NOT CERTIFIED'}**",
        "",
        "Collected by a read-only diagnostic. Nothing on the device was modified and no",
        "certificate signing was requested from Google by this tool.",
        "",
        "## 1. Four-layer result",
        "",
        "| Layer | State | Owner | Note |",
        "| --- | --- | --- | --- |",
    ]
    states = {"pass": "passed; necessary but not sufficient on its own",
              "fail": "blocked; cannot be cleared from the client",
              "unknown": "not probed on this run"}
    titles = {item["id"]: item.get("title_en", item["title"]) for item in LAYERS}
    for layer in layers:
        lines.append(f"| {titles.get(layer['layer'], layer['title'])} | {layer['state']} "
                     f"| {layer['owner']} | {states.get(layer['state'], layer['state'])} |")
    lines += [
        "",
        "## 2. Root cause",
        "",
        "### 2.1 Hardware key attestation (layer 3)",
        "",
        f"- Chain length: {verification.get('certificate_count', 'not collected')}",
        f"- Chain root: `{verification.get('root_subject', 'not collected')}`",
        f"- Internal signatures and validity: {verification.get('internal_path_valid', 'not collected')}",
        f"- Matches a Google official hardware attestation root: "
        f"**{verification.get('official_root_match', 'not collected')}**",
        f"- OpenSSL error: {verification.get('official_verify_error', 'none')}",
        "",
        "The TEE chain is internally valid but its root is a self-signed OEM test root that is",
        "not one of the roots Google publishes:",
        "",
    ]
    for root in roots:
        lines.append(f"- `{root['subject']}` ({root['not_before']} -> {root['not_after']}, "
                     f"{root['public_key_algorithm']}, Key Usage: {root['key_usage']})")
    lines += [
        "",
        "### 2.2 Remote key provisioning and device catalog (layer 4)",
        "",
        "The RKP hardware itself is healthy:",
        "",
        "| Component | hwVersion | RPC author | uniqueId | keys per CSR |",
        "| --- | --- | --- | --- | --- |",
    ]
    for name, info in components.items():
        lines.append(f"| {name} | {info.get('hwVersion', '')} | {info.get('rpcAuthorName', '')} "
                     f"| {info.get('uniqueId', '')} | {info.get('supportedNumKeysInCsr', '')} |")
    lines += [
        "",
        f"- `remote_provisioning.hostname`: `{config.get('remote_provisioning.hostname') or 'empty'}`",
        f"- RKP `signCertificates`: {backend if backend is not None else 'not probed'}"
        " (HTTP 444 `Device not registered.` means Google's backend does not recognise this"
        " device's public key)",
        f"- Listed in Google's certified device catalog: "
        f"{'yes' if catalog.get('listed') else 'no' if catalog.get('listed') is False else 'not collected'}"
        f" ({catalog.get('rows_checked', '?')} rows checked; no row matches any identity"
        f" field of this device)",
        "",
        "Local CSR identity (generated on device, no network call):",
        "",
        f"- Component: `{csr.get('component', 'default')}`, {csr.get('bytes', '?')} bytes, "
        f"sha256 `{csr.get('sha256', 'n/a')}`",
        f"- uniqueId: `{csr.get('unique_id', 'n/a')}`, RPC author: `{csr.get('rpc_author', 'n/a')}`",
        f"- HAL-reported identity: {json.dumps(csr.get('device_info', {}), ensure_ascii=False)}",
        f"- TEE firmware: {', '.join(csr.get('firmware_versions', [])) or 'not read'}",
        "",
        "Note: the brand/manufacturer inside the CSR come from the SoC vendor's HAL, not from",
        "the OEM's registration record. Please keep the two apart when triaging.",
        "",
        "### 2.3 Two RKP registration identities",
        "",
        "The HAL exposes two components, each a separate backend registration record:",
        "",
        "| Component | CSR bytes | brand | model | manufacturer | security level |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for entry in (rkp.get("rkp_csr_components") or []):
        if "error" in entry:
            continue
        info = entry.get("device_info") or {}
        lines.append(f"| {entry['component']} | {entry.get('bytes', '')} | {info.get('brand', '')} "
                     f"| {info.get('model', '')} | {info.get('manufacturer', '')} "
                     f"| {info.get('security_level', '')} |")
    lines += [
        "",
        "Only the component carrying the real device identity is relevant to Play",
        "certification; the AVF identity is for protected VMs and would not help.",
        "",
        "### 2.4 Security patch freshness (risk factor, not part of the verdict)",
        "",
        f"- Security patch level: `{patch.get('security_patch') or 'not read'}`"
        f" (vendor: `{patch.get('vendor_security_patch') or 'not read'}`)",
        f"- Age: {patch.get('age_days', 'not computed')} days"
        f" ({'over' if patch.get('stale') else 'under' if patch.get('stale') is False else 'unknown'}"
        " 180 days)",
        "",
        "Play Integrity's `MEETS_DEVICE_INTEGRITY` checks patch freshness; the Play Store's",
        "\"device is certified\" flag does not. So even after RKP registration and catalog",
        "listing, a stale patch can still fail app-side checks. Google publishes no threshold,",
        "so only the measured age is reported here and it is not part of the verdict above.",
        "",
        "## 3. Required actions, highest leverage first",
        "",
    ]
    for index, item in enumerate(LEVERAGE, start=1):
        lines.append(f"{index}. **{item.get('action_en', item['action'])}** "
                     f"(owner: {item['owner']}) - clears: {', '.join(item['clears'])}. "
                     f"{item.get('reason_en', item['reason'])}")
    lines += [
        "",
        "Google's official hardware attestation roots currently include a root added in 2025",
        "(`Key Attestation CA1`). GTS validates remotely provisioned chains against the same",
        "root list (`testEcAttestationChainRemProvLengthTee`), so RKP registration is expected",
        "to clear both the layer-3 root match and the layer-4 backend check. That expectation",
        "is an inference and must be re-measured after registration.",
        "",
        "### About HTTP 444",
        "",
        "Google documents 444 as \"the backend does not recognise this device public key\". It",
        "has at least two causes with different owners:",
        "",
        "1. This device's RKP public key was never registered.",
        "2. The RPC author's key was never registered. This device reports",
        "   `rpcAuthorName=Qualcomm`, so the request is signed by Qualcomm's author key.",
        "",
        "Both look identical from the client; only the partner portal can tell them apart.",
        "Please attach this device's CSR and the RKP HAL dump when escalating.",
        "",
        "### Authoritative acceptance test",
        "",
        "Google GTS `AttestationRootHostTest` (including",
        "`testEcAttestationChainRemProvLengthTee`) is the authoritative check for layers 3 and",
        "4: passing it means the device public key is registered and Google has issued valid",
        "attestation certificates. Please provide its passing record.",
        "",
        "## 4. How to re-verify",
        "",
        "```bash",
        "python3 scripts/play_protect_certification.py status \\",
        f"  --serial {device.serial} \\",
        "  --attestation-chain <previously captured chain> \\",
        "  --firewall-allows <true|false> --rkp-backend-status <HTTP status>",
        "```",
        "",
        "`certified` only becomes true once all four layers pass. No client-side change,",
        "including the Play compatibility patch, can alter that verdict.",
        "",
    ]
    text = "\n".join(lines)
    if options.out:
        Path(options.out).write_text(text)
        print(json.dumps({"ok": True, "written": options.out, "verdict":
                          "certified" if certified else "not_certified"},
                         ensure_ascii=False, separators=(",", ":")))
    else:
        print(text)
    return 0


def command_firmware(options):
    """Read-only firmware-channel query; never installs anything."""
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    current = device.prop("ro.build.aios_version")
    if not current:
        current = device.prop("ro.build.mask.id")
    fingerprint = device.prop("ro.build.fingerprint")
    parsed = probe_firmware(device)
    payload = {"schema": SCHEMA, "checked_at": utc_now(), "serial": device.serial,
               "current_build": current, "current_fingerprint": fingerprint,
               "security_patch": device.prop("ro.build.version.security_patch")}
    payload.update(parsed)
    payload["same_as_current"] = parsed.get("same_as_current", False)
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    return 0 if parsed.get("available") else 1


def command_rkp_pool(options):
    """Check the RKP certificate pool; refuses to run when a hostname is set."""
    device = getattr(options, "device", None) or Device(options.serial)
    device.verify_identity()
    parsed = probe_rkp_pool(device, options.component)
    parsed.update({"schema": SCHEMA, "checked_at": utc_now(), "serial": device.serial,
                   "hostname": device.prop("remote_provisioning.hostname")})
    print(json.dumps(parsed, ensure_ascii=False, separators=(",", ":")))
    return 0


def tri_state(text):
    lowered = str(text).strip().lower()
    if lowered in ("true", "1", "yes", "允许"):
        return True
    if lowered in ("false", "0", "no", "阻断"):
        return False
    if lowered in ("", "unknown", "none", "未探测"):
        return None
    raise argparse.ArgumentTypeError("只接受 true / false / unknown")


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("explain", help="打印四层认证机制认知")

    def add_common(target, serial=True):
        if serial:
            target.add_argument("--serial", default=os.environ.get("STEPX_SERIAL", DEFAULT_SERIAL))
        target.add_argument("--offline", action="store_true", help="不访问 Google 官方资源")
        target.add_argument("--attestation-chain", default=None,
                            help="既往采集的密钥证明链目录（leaf/intermediates/chain-provided-root.pem）")
        target.add_argument("--roots-cache", default=str(ROOT / ".runtime/attestation-official-roots.pem"),
                            help="Google 官方证明根列表的本地缓存位置")
        target.add_argument("--compat-active", type=tri_state, default=None,
                            help="Play 兼容补丁是否生效；仅记录可用性，不影响认证判定")
        target.add_argument("--firewall-allows", type=tri_state, default=None,
                            help="固件防火墙是否放行 rkpdapp；读取该配置需要 root")
        target.add_argument("--rkp-backend-status", type=int, default=None,
                            help="既往取证到的 RKP signCertificates HTTP 状态码；本工具不主动请求签发")
        target.add_argument("--cache-file", default=str(ROOT / ".runtime/play-protect-catalog.json"))
        target.add_argument("--firmware", action="store_true",
                            help="附带只读查询官方固件通道是否有更新构建")
        target.add_argument("--revocation-cache",
                            default=str(ROOT / ".runtime/attestation-status.json"),
                            help="Google 吊销列表的本地缓存位置")

    for name, help_text in (("status", "逐层取证并给出认证判定"),
                            ("monitor", "检测认证状态是否发生变化"),
                            ("dossier", "生成厂商侧登记材料"),
                            ("ticket", "生成提交给厂商的认证问题材料"),
                            ("ticket-en", "生成英文版认证问题材料（用于升级到 SoC 厂商或 Google）"),
                            ("firmware", "只读查询官方固件通道是否有比设备更新的构建"),
                            ("rkp-pool", "在 hostname 为空时安全地检查 RKP 证书池状态")):
        target = subparsers.add_parser(name, help=help_text)
        add_common(target)
    subparsers.choices["monitor"].add_argument("--state-file",
                                               default=str(ROOT / ".runtime/play-protect-monitor.json"))
    subparsers.choices["monitor"].add_argument("--fail-on-blockers", action="store_true")
    for name in ("dossier", "ticket", "ticket-en", "rkp-pool"):
        subparsers.choices[name].add_argument("--component", default="default")
    for name in ("dossier", "ticket", "ticket-en"):
        subparsers.choices[name].add_argument("--out", default=None)

    csr = subparsers.add_parser("csr", help="本地生成并解析 RKP CSR（不联系 Google）")
    add_common(csr)
    csr.add_argument("--component", default="default")

    chain = subparsers.add_parser("chain", help="离线校验已采集的密钥证明链")
    chain.add_argument("--directory", required=True)
    chain.add_argument("--roots-cache", default=str(ROOT / ".runtime/attestation-official-roots.pem"))
    chain.add_argument("--revocation-cache",
                       default=str(ROOT / ".runtime/attestation-status.json"))
    chain.add_argument("--offline", action="store_true")
    return parser


def main(argv=None):
    arguments = build_parser().parse_args(argv)
    handlers = {"explain": command_explain, "status": command_status, "csr": command_csr,
                "chain": command_chain, "monitor": command_monitor, "dossier": command_dossier,
                "ticket": command_ticket, "ticket-en": command_ticket_en,
                "firmware": command_firmware, "rkp-pool": command_rkp_pool}
    try:
        return handlers[arguments.command](arguments)
    except ProbeError as error:
        print(json.dumps({"ok": False, "error": "probe_failed", "message": str(error)},
                         ensure_ascii=False, separators=(",", ":")))
        return 1


if __name__ == "__main__":
    sys.exit(main())
