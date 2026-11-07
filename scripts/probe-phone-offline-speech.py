#!/usr/bin/env python3
"""Read-only, fail-closed audit of the selected phone's OEM speech backend.

This is a capability probe, NOT an ASR/TTS benchmark. A registered local MCP
endpoint is not evidence of local inference. No microphone, model, cloud request,
screen wake, settings mutation, model download, or token access is performed.
--sleep explicitly sends the sleep key for a user-authorized screen-off check.
Only bounded, non-content facts are printed; raw dumps remain in memory.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import time


SERVICE = "com.stepos.aiengine/.service.AIEngineManagerService"
PERMISSION = "com.stepos.aiengine.permission.AI_ENGINE_MANAGER_SERVICE"
SPEECH_MODELS = {"asr": "step-asr", "tts": "step4audio-tts"}
PRESET_MODEL_ROOT = "/system/media/models"


def power_state(text):
    match = re.search(r"mWakefulness=(\w+)", text)
    return match.group(1) if match else "unknown"


def installed_models(text):
    # This is the OTA installation record, not an exhaustive inventory of OEM
    # preset weights. Absence here must never be rendered as "no speech model".
    if "Installed model versions:" not in text:
        return None
    installed = text.split("Installed model versions:", 1)[1].split("Remote update cache:", 1)[0]
    return dict(re.findall(r"^\s+([\w.-]+): v([\w.-]+)\s*$", installed, re.M))


def registered_abilities(text):
    if not re.search(r"Registered abilities\s*\(\d+\)", text):
        return None
    return re.findall(r"^\s+(stepos-(?:asr|tts|vad|sla)[\w-]*)\s+-", text, re.M)


def permission_state(text):
    if "Permission Denial" in text or "SecurityException" in text:
        return "denied"
    # checkCTA is an actual read-only provider method; arbitrary method names
    # returning empty Bundles cannot establish access or inference permission.
    if re.search(r"Result: Bundle\[.*result=(?:true|false)", text):
        return "provider_accessible"
    return "unknown"


def preset_access_state(text):
    if "Permission denied" in text or "Permission Denial" in text:
        return "permission_denied"
    if "No such file or directory" in text:
        return "not_found"
    # Only the directory itself is queried, never recursive model contents.
    if re.search(r"^d\S*\s+.*" + re.escape(PRESET_MODEL_ROOT) + r"/?\s*$", text, re.M):
        return "directory_visible"
    return "unknown"


def speech_command_capability(registered, models, cache):
    # ModelCache is runtime evidence and is independent of the OTA catalog.
    # Do not print raw provider output (holders can contain client identities).
    row = re.search(r"^\s*sla\s+routeVersion=([^\r\n]+)$", cache, re.M)
    fields = dict(re.findall(r"(\w+)=([^\s]+)", "routeVersion=" + row.group(1))) if row else {}
    ready = {"true": True, "false": False}.get(fields.get("ready"))
    backend = fields.get("backend")
    if backend and not re.fullmatch(r"[A-Z0-9_]{1,48}", backend):
        backend = None
    loaded_path = fields.get("loadedPath", "")
    location = "preset" if loaded_path.startswith(PRESET_MODEL_ROOT + "/") else (
        "app_private" if loaded_path.startswith("/data/user/") else "unknown")
    return {"model_id": "sla", "ability_id": "stepos-sla", "purpose": "speech_commands",
            "label": "原厂预置语音指令 SLA", "registered": None if registered is None else "stepos-sla" in registered,
            "model_in_installed_inventory": None if models is None else "sla" in models,
            "model_cache_checked": "----- ModelCache -----" in cache,
            "ready": ready, "backend": backend,
            "loaded_version": fields.get("loadedVersion"), "loaded_location": location,
            "evidence_source": "AIEngine.ModelCache" if row else None,
            "status": "loaded" if ready is True else "not_verified",
            "generic_dictation_verified": False, "tts_verified": False,
            "inference_attempted": False, "offline_inference_verified": False,
            "atomic_action_executed": False}


def capability(kind, registered, models, permission):
    prefix = "stepos-" + kind
    ids = [x for x in registered or [] if x == prefix or x.startswith(prefix + "-")]
    model = SPEECH_MODELS[kind]
    in_inventory = None if models is None else model in models
    # A missing OTA entry is insufficient to rule out a protected system preset.
    installed = True if in_inventory is True else None
    blockers = []
    if registered is None:
        blockers.append("ability_registry_unavailable")
    elif not ids:
        blockers.append("ability_not_registered")
    if in_inventory is not True:
        blockers.append("model_installation_unknown" if in_inventory is None else "model_not_in_installed_inventory")
    if permission != "provider_accessible":
        blockers.append("oem_signature_permission_denied" if permission == "denied" else "oem_access_not_verified")
    # Deliberately never turns a capability listing into a successful inference.
    blockers.append("offline_only_inference_contract_not_verified")
    return {"registered": None if registered is None else bool(ids),
            "installed": installed, "backend": None,
            "registered_abilities": ids, "model_id": model,
            "model_in_installed_inventory": in_inventory,
            "installation_evidence_scope": "ota_inventory_only",
            "preset_installation_verified": False, "status": "unavailable",
            "offline_inference_verified": False, "offline_verified": False, "inference_attempted": False,
            "blockers": blockers}


class Probe:
    def __init__(self, adb, serial):
        self.adb, self.serial = adb, serial
        self.timings, self.power_samples, self.errors = [], [], []
        self.deadline = time.monotonic() + 25

    def run(self, label, args, timeout=8):
        start = time.monotonic()
        remaining = self.deadline - start
        if remaining <= 0:
            self.errors.append({"probe": label, "kind": "total_deadline"})
            return ""
        try:
            p = subprocess.run([self.adb, "-s", self.serial, *args], capture_output=True,
                               text=True, errors="replace", timeout=min(timeout, remaining), check=False)
            text = p.stdout + p.stderr
            if p.returncode and "SecurityException" not in text:
                self.errors.append({"probe": label, "kind": "command_failed", "exit_code": p.returncode})
            return text
        except subprocess.TimeoutExpired:
            self.errors.append({"probe": label, "kind": "timeout"})
            return ""
        finally:
            self.timings.append({"probe": label, "elapsed_ms": round((time.monotonic() - start) * 1000)})

    def screen(self, label):
        state = power_state(self.run("power_" + label, ["shell", "dumpsys", "power", "--brief"]))
        self.power_samples.append({"phase": label, "wakefulness": state})
        return state


def audit(adb, serial, sleep=False, native_probe=None):
    p = Probe(adb, serial)
    state = p.run("transport", ["get-state"]).strip()
    if state != "device":
        raise RuntimeError("Selected transport is not an authorized online ADB device")
    device_serial = p.run("identity", ["shell", "getprop", "ro.serialno"]).strip()
    device_model = p.run("model", ["shell", "getprop", "ro.product.model"]).strip()
    # ADB wireless transport syntax, not a name guessed from the device model.
    wireless = bool(re.fullmatch(r"(?:\[[0-9a-fA-F:%._-]+\]|[A-Za-z0-9_.-]+):\d+", serial))
    if sleep:
        p.run("explicit_sleep", ["shell", "input", "keyevent", "KEYCODE_SLEEP"])
    p.screen("before")
    abilities = p.run("ability_registry", ["shell", "dumpsys", "activity", "service", SERVICE, "--provider=Ability"])
    p.screen("after_registry")
    ota = p.run("installed_models", ["shell", "dumpsys", "activity", "service", SERVICE, "--provider=OTA"])
    model_cache = p.run("model_cache", ["shell", "dumpsys", "activity", "service", SERVICE, "--provider=ModelCache"])
    preset_directory = p.run("preset_directory", ["shell", "ls", "-ld", PRESET_MODEL_ROOT])
    p.screen("after_model_inventory")
    # checkCTA only queries state and does not display a dialog or run inference.
    access = p.run("oem_provider_access", ["shell", "content", "call", "--uri",
                   "content://com.stepos.aiengine.DEEP_BINDER_IPC_CALL", "--method", "checkCTA"])
    p.screen("after_access_check")
    companion = p.run("companion_permission", ["shell", "dumpsys", "package", "com.stepx.companion.app"])
    mcp = {"checked": False, "tools": [], "local_endpoint_is_not_offline_inference": True}
    if native_probe and p.deadline > time.monotonic():
        start = time.monotonic()
        try:
            result = subprocess.run([str(native_probe), serial, "contracts"], capture_output=True,
                                    text=True, timeout=max(0.01, p.deadline-time.monotonic()), check=False)
            if result.returncode == 0:
                for line in result.stdout.splitlines():
                    try:
                        value = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(value, dict) and isinstance(value.get("schemas"), dict):
                        mcp["checked"] = True
                        mcp["tools"] = sorted(k for k in value["schemas"] if
                            re.search(r"speak|speech|(?:^|_)asr(?:_|$)|(?:^|_)tts(?:_|$)|assistant_.*voice", k))
            else:
                p.errors.append({"probe": "native_mcp", "kind": "command_failed", "exit_code": result.returncode})
        except subprocess.TimeoutExpired:
            p.errors.append({"probe": "native_mcp", "kind": "timeout"})
        p.timings.append({"probe": "native_mcp", "elapsed_ms": round((time.monotonic()-start)*1000)})
    p.screen("after")
    registered, models, permission = registered_abilities(abilities), installed_models(ota), permission_state(access)
    sla = speech_command_capability(registered, models, model_cache)
    return {"schema_version": 1, "checked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "device_serial": device_serial if re.fullmatch(r"[\w.-]{1,100}", device_serial) else None,
            "device_model": device_model if len(device_model) <= 100 else None,
            "transport": "wireless" if wireless else "usb_or_other",
            "screen_off": p.power_samples[-1]["wakefulness"] in ("Asleep", "Dozing"),
            "screen_samples": p.power_samples,
            "screen_off_at_all_samples": all(x["wakefulness"] in ("Asleep", "Dozing") for x in p.power_samples),
            "continuous_screen_off_verified": False,
            "oem_provider_access": permission,
            "permission_error": "原厂语音服务需要签名或特权授权，当前调用被手机系统拒绝" if permission == "denied" else None,
            "companion_requests_engine_permission": PERMISSION in companion,
            "installed_model_count": len(models) if models is not None else None,
            "installed_model_count_scope": "ota_inventory_only",
            "ota_inventory": {"checked": models is not None, "model_count": len(models) if models is not None else None,
                              "includes_all_preset_models": False},
            "preset_models": {"root": PRESET_MODEL_ROOT, "root_source": "oem_preinstall_configuration",
                              "query": "directory_metadata_only", "access": preset_access_state(preset_directory),
                              "inventory_complete": False},
            "sla": sla,
            "oem_speech_model_discovered": True if sla["ready"] is True or (models is not None and "sla" in models) else None,
            "native_mcp": mcp,
            "asr": capability("asr", registered, models, permission),
            "tts": capability("tts", registered, models, permission),
            "microphone_opened": False, "cloud_called": False, "cloud_inference_called": False,
            "model_download_started": False, "timings": p.timings, "errors": p.errors}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", nargs="?", help="Explicit authorized ADB transport; no automatic device selection")
    parser.add_argument("--serial", dest="named_serial", help="Explicit authorized ADB transport")
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--sleep", action="store_true", help="Explicitly send KEYCODE_SLEEP before read-only probes")
    parser.add_argument("--require-screen-off", action="store_true")
    parser.add_argument("--native-probe", type=Path, help="Built verify_native_mcp example (read-only contracts mode)")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    serial = args.named_serial or args.serial
    if not serial:
        parser.error("an explicit --serial is required")
    result = audit(args.adb, serial, args.sleep, args.native_probe)
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        # Contains only capability facts; still restrict access to local audit files.
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(output)
    print(output, end="")
    if args.require_screen_off and not result["screen_off_at_all_samples"]:
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
