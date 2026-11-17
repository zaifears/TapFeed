"""
TapFeed camera helper (pytapo).

Runs pytapo for the Rust app and nothing else. One JSON object per line:

    stdin   {"id": 1, "cmd": "connect", "host": "...", ...}
    stdout  {"id": 1, "ok": true, "result": {...}}
            {"id": 1, "ok": false, "code": "ERR_CLOUD_AUTH", "message": "..."}
            {"event": "download_progress", "id": 4, "progress": 120, "total": 300}

stdout carries only protocol lines; anything pytapo prints goes to stderr.
Passwords arrive over stdin only and are never written anywhere.
"""

from __future__ import annotations

import asyncio
import datetime
import json
import os
import sys
import threading
import time
import traceback
from typing import Any, Callable, Dict, List, Optional

# Keep the protocol channel clean before importing pytapo.
_PROTOCOL_OUT = sys.__stdout__
sys.stdout = sys.stderr
for stream in (sys.stdin, _PROTOCOL_OUT, sys.stderr):
    try:
        stream.reconfigure(encoding="utf-8")
    except Exception:
        pass

from pytapo import Tapo  # noqa: E402
from pytapo.media_stream._utils import StreamType  # noqa: E402
from pytapo.version import PYTAPO_VERSION  # noqa: E402

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from tswriter import TsWriter  # noqa: E402
from errors import (  # noqa: E402
    CTX_CONNECT,
    CTX_DOWNLOAD,
    CTX_RECORDINGS,
    CloudLoginError,
    diagnose,
)

PROTOCOL = 1
_write_lock = threading.Lock()


def send(obj: Dict[str, Any]) -> None:
    line = json.dumps(obj, ensure_ascii=False, default=str)
    with _write_lock:
        _PROTOCOL_OUT.write(line + "\n")
        _PROTOCOL_OUT.flush()


class HelperError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


# ---------------------------------------------------------------------------
# login with fallback
# ---------------------------------------------------------------------------
_LOCKOUT_HINTS = ("temporary suspension", "device_blocked", "-40404")
_NETWORK_HINTS = (
    "timed out", "timeout", "connection refused", "unreachable", "no route",
    "max retries exceeded", "name or service not known", "getaddrinfo",
    "connection aborted", "connection reset", "failed to establish",
)


def _login_plan(username: str, password: str, cloud: str, prefer: str) -> List[Dict[str, str]]:
    camera = {"method": "camera", "user": username, "password": password, "cloud": cloud or password}
    admin = {"method": "admin", "user": "admin", "password": cloud or password, "cloud": cloud or password}
    if prefer == "camera":
        return [camera, admin]
    if prefer == "admin":
        return [admin, camera]
    # Documented pytapo method first when a separate cloud password was given.
    return [admin, camera] if cloud else [camera, admin]


FRESH_SECONDS = 60
STALL_SECONDS = 120
BUSY_CODES = (-52405, -52407, -52417, -52435)


def _stream_request(tapo: Tapo, method: str, start: int, end: int) -> Dict[str, Any]:
    if method == "download":
        params = {"download": {"client_id": tapo.getUserID(), "channels": [0], "media_type": 0,
                               "start_time": str(start), "end_time": str(end), "player_id": tapo.playerID}}
    else:
        params = {"playback": {"client_id": tapo.getUserID(), "channels": [0, 1], "scale": "1/1",
                               "start_time": str(start), "end_time": str(end), "event_type": [1, 2]}}
    return {"type": "request", "seq": 1, "params": {**params, "method": "get"}}



class Session:
    def __init__(self) -> None:
        self.tapo: Optional[Tapo] = None
        self.time_correction = 0
        self.ptz: Dict[str, Any] = {"pan_tilt": False, "presets": False}
        self.motor_lock = threading.Lock()
        self.cancel = threading.Event()
        self.busy = threading.Lock()

    # -- commands -----------------------------------------------------------
    def connect(self, req: Dict[str, Any]) -> Dict[str, Any]:
        host = str(req.get("host", "")).strip()
        username = str(req.get("username", "")).strip()
        password = str(req.get("password", ""))
        cloud = str(req.get("cloud_password") or "")
        prefer = str(req.get("prefer") or "")
        if not host or not username or not password:
            raise HelperError("ERR_INPUT", "Camera IP, username and password are required.")

        self.tapo = None
        failures: List[str] = []
        last: Optional[Exception] = None
        for attempt in _login_plan(username, password, cloud, prefer):
            try:
                tapo = Tapo(
                    host=host,
                    user=attempt["user"],
                    password=attempt["password"],
                    cloudPassword=attempt["cloud"],
                    retryStok=True,
                )
                info = tapo.getBasicInfo()
            except Exception as exc:  # noqa: BLE001 - classified below
                last = exc
                low = str(exc).lower()
                failures.append(f"{attempt['method']}: {exc}")
                if any(h in low for h in _LOCKOUT_HINTS) or any(h in low for h in _NETWORK_HINTS):
                    break  # another attempt would only extend a lockout / cannot help
                continue

            self.tapo = tapo
            self.ptz = {"pan_tilt": False, "presets": False}
            try:
                self.time_correction = int(tapo.getTimeCorrection() or 0)
            except Exception:
                self.time_correction = 0
            basic = (info or {}).get("device_info", {}).get("basic_info", {}) if isinstance(info, dict) else {}
            return {
                "login": attempt["method"],
                "model": basic.get("device_model") or "",
                "alias": basic.get("device_alias") or "",
                "firmware": basic.get("sw_version") or "",
                "mac": basic.get("mac") or "",
                "time_correction": self.time_correction,
            }

        detail = "; ".join(failures) or str(last)
        diag = diagnose(CloudLoginError(detail), CTX_CONNECT)
        raise HelperError(diag.code, detail)

    def _need(self) -> Tapo:
        if self.tapo is None:
            raise HelperError("ERR_SD_NOT_AUTHENTICATED", "Not connected to the camera.")
        return self.tapo

    def recording_dates(self, req: Dict[str, Any]) -> List[str]:
        tapo = self._need()
        today = datetime.date.today().strftime("%Y%m%d")
        raw = tapo.getRecordingsList(start_date="20200101", end_date=today)
        dates: List[str] = []
        for item in raw if isinstance(raw, list) else []:
            if isinstance(item, dict):
                inner = item
                if len(item) == 1 and isinstance(next(iter(item.values())), dict):
                    inner = next(iter(item.values()))
                d = inner.get("date") or inner.get("search_date") or item.get("date")
            else:
                d = item
            d = str(d or "")
            if len(d) == 8 and d.isdigit():
                fmt = f"{d[0:4]}-{d[4:6]}-{d[6:8]}"
                if fmt not in dates:
                    dates.append(fmt)
        dates.sort(reverse=True)
        return dates

    def list_recordings(self, req: Dict[str, Any]) -> List[Dict[str, Any]]:
        tapo = self._need()
        date = str(req.get("date", "")).replace("-", "").strip()
        if len(date) != 8:
            raise HelperError("ERR_INPUT", "Date must be YYYY-MM-DD.")
        raw = tapo.getRecordings(date)
        clips: List[Dict[str, Any]] = []
        for item in raw if isinstance(raw, list) else []:
            data = item
            if isinstance(item, dict) and len(item) == 1 and isinstance(next(iter(item.values())), dict):
                data = next(iter(item.values()))
            if not isinstance(data, dict):
                continue
            try:
                start = int(data.get("startTime", data.get("start_time", 0)) or 0)
                end = int(data.get("endTime", data.get("end_time", 0)) or 0)
            except (TypeError, ValueError):
                continue
            if start <= 0 or end <= start:
                continue
            kind = str(data.get("vedio_type") or data.get("video_type") or data.get("type") or "1")
            clips.append({"start": start, "end": end, "motion": kind != "1"})
        clips.sort(key=lambda c: c["start"])
        return clips

    def download(self, req: Dict[str, Any], progress: Callable[[Dict[str, Any]], None]) -> Dict[str, Any]:
        tapo = self._need()
        start, end = int(req["start"]), int(req["end"])
        path = os.path.abspath(str(req["path"]))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if time.time() - FRESH_SECONDS - self.time_correction < end:
            raise HelperError("ERR_DOWNLOAD_EMPTY", "The recording is still in progress.")
        if not self.busy.acquire(blocking=False):
            raise HelperError("ERR_CAMERA_BUSY", "Another download is already running.")
        self.cancel.clear()
        try:
            # .part marks it unfinished; the app deletes both files on failure
            open(path + ".part", "wb").close()
            with open(path, "wb") as out:
                # getUserID() may log in with an event loop of its own, so not inside ours
                requests = [_stream_request(tapo, m, start, end) for m in ("download", "playback")]
                asyncio.run(self._download(tapo, requests, end - start, TsWriter(out), progress))
        finally:
            self.busy.release()

        if self.cancel.is_set():
            raise HelperError("ERR_DOWNLOAD_CANCELLED", "Download cancelled.")
        if os.path.getsize(path) <= 1024:
            raise HelperError("ERR_DOWNLOAD_EMPTY", f"The camera sent no video for this clip ({os.path.basename(path)}).")
        os.remove(path + ".part")
        return {"path": path, "bytes": os.path.getsize(path)}

    async def _download(self, tapo, requests, total, writer, progress) -> None:
        for req in requests:
            method = next(iter(req["params"]))
            last_report = 0.0
            session = tapo.getMediaSession(StreamType.Download)
            session.set_window_size(200)
            async with session:
                stream = session.transceive(json.dumps(req))
                try:
                    while not self.cancel.is_set():
                        try:
                            resp = await asyncio.wait_for(stream.__anext__(), timeout=STALL_SECONDS)
                        except StopAsyncIteration:
                            return
                        except asyncio.TimeoutError:
                            raise HelperError("ERR_CAMERA_TIMEOUT", "The camera stopped sending the recording.")
                        if resp.mimetype == "video/mp2t":
                            done = writer.feed(resp.plaintext)
                            now = time.monotonic()
                            if now - last_report >= 0.5:
                                last_report = now
                                progress({"action": "Downloading", "progress": min(done, total), "total": total})
                            # playback runs at 1x and past end_time; download stops by itself
                            if method == "playback" and done >= total:
                                return
                        elif resp.mimetype == "application/json":
                            try:
                                msg = json.loads(resp.plaintext.decode())
                            except ValueError:
                                continue
                            params = msg.get("params") or {}
                            code = params.get("error_code")
                            if msg.get("type") == "response" and code in BUSY_CODES:
                                raise HelperError("ERR_CAMERA_BUSY", f"Camera busy (code {code}).")
                            if msg.get("type") == "response" and code not in (0, None):
                                if method == "download" and writer.out.tell() == 0:
                                    break  # older firmware: no download request, use playback
                                raise HelperError("ERR_DOWNLOAD_FAILED", f"Camera refused the recording (code {code}).")
                            if params.get("event_type") == "stream_status" and params.get("status") == "finished":
                                return
                    else:
                        return
                finally:
                    await stream.aclose()



    # -- pan / tilt -----------------------------------------------------------
    # Only offered when the camera reports a motor. Moves are small relative
    # steps; the camera's firmware stops at its own mechanical limits and
    # answers with an error there, which is reported, never retried.
    STEP_MAX = 30

    @staticmethod
    def _capability(raw: Any) -> Optional[Dict[str, Any]]:
        """Find motor.capability in a reply, wrapped in "result" or not."""
        if not isinstance(raw, dict):
            return None
        for root in (raw, raw.get("result") if isinstance(raw.get("result"), dict) else None):
            cap = (root or {}).get("motor", {}).get("capability") if root else None
            if isinstance(cap, dict) and cap:
                return cap
        return None

    def ptz_info(self, req: Dict[str, Any]) -> Dict[str, Any]:
        tapo = self._need()
        pan_tilt = presets = False
        how = "none"

        # 1. The capability request, wrapped the way current firmware accepts
        #    (pytapo's own getMotorCapability sends it bare, which newer C200
        #    firmware rejects with -40209).
        attempts = (
            ("wrapped", lambda: tapo.executeFunction("getMotorCapability", {"motor": {"name": ["capability"]}})),
            ("pytapo", tapo.getMotorCapability),
        )
        for name, ask in attempts:
            try:
                cap = self._capability(ask())
            except Exception as exc:
                print(f"motor capability ({name}) failed: {exc}", file=sys.stderr)
                continue
            if cap is not None:
                on = lambda k: str(cap.get(k, "0")) == "1"  # noqa: E731
                pan_tilt = on("relative_move_supported") or on("absolute_move_supported")
                presets = pan_tilt and on("preset_supported")
                how = f"capability ({name})"
                break

        # 2. Only motorised cameras keep a saved-positions list; if the camera
        #    answers with one, it can turn.
        listed: List[Dict[str, str]] = []
        if presets or how == "none":
            try:
                found = tapo.getPresets() or {}
                listed = [{"id": str(k), "name": str(v)} for k, v in found.items()]
                if how == "none":
                    pan_tilt = presets = True
                    how = "saved positions list"
            except Exception as exc:
                print(f"saved positions unavailable: {exc}", file=sys.stderr)

        print(f"pan/tilt: {pan_tilt} (found by {how})", file=sys.stderr)
        self.ptz = {"pan_tilt": pan_tilt, "presets": presets}
        return {"panTilt": pan_tilt, "presets": listed, "detectedBy": how}

    def ptz_move(self, req: Dict[str, Any]) -> Dict[str, Any]:
        tapo = self._need()
        if not self.ptz.get("pan_tilt"):
            raise HelperError("ERR_PTZ_UNSUPPORTED", "This camera has no pan and tilt motor.")
        try:
            dx = max(-self.STEP_MAX, min(self.STEP_MAX, int(req.get("dx", 0))))
            dy = max(-self.STEP_MAX, min(self.STEP_MAX, int(req.get("dy", 0))))
        except (TypeError, ValueError):
            raise HelperError("ERR_INPUT", "dx and dy must be whole numbers.")
        if dx == 0 and dy == 0:
            return {}
        if not self.motor_lock.acquire(blocking=False):
            raise HelperError("ERR_PTZ_BUSY", "The camera is still moving.")
        started = time.monotonic()
        try:
            tapo.moveMotor(dx, dy)
        except Exception as exc:
            print(f"pan/tilt move {dx},{dy} failed after {time.monotonic() - started:.1f}s: {exc}", file=sys.stderr)
            raise HelperError("ERR_PTZ_MOVE", str(exc))
        finally:
            self.motor_lock.release()
        print(f"pan/tilt move {dx},{dy} answered in {time.monotonic() - started:.1f}s", file=sys.stderr)
        return {}

    def ptz_goto(self, req: Dict[str, Any]) -> Dict[str, Any]:
        tapo = self._need()
        if not self.ptz.get("presets"):
            raise HelperError("ERR_PTZ_UNSUPPORTED", "This camera has no saved positions.")
        try:
            tapo.setPreset(str(req.get("id", "")))
        except Exception as exc:
            raise HelperError("ERR_PTZ_MOVE", str(exc))
        return {}

    def disconnect(self, req: Dict[str, Any]) -> Dict[str, Any]:
        self.cancel.set()
        self.tapo = None
        return {}


# ---------------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------------
def _error_reply(req_id: Any, exc: BaseException, ctx: str) -> Dict[str, Any]:
    if isinstance(exc, HelperError):
        code, message = exc.code, str(exc)
    else:
        code, message = diagnose(exc, ctx).code, f"{type(exc).__name__}: {exc}"
        traceback.print_exc(file=sys.stderr)
    return {"id": req_id, "ok": False, "code": code, "message": message}


def main() -> None:
    session = Session()
    contexts = {"connect": CTX_CONNECT, "download": CTX_DOWNLOAD}

    def run(req: Dict[str, Any]) -> None:
        req_id = req.get("id")
        cmd = req.get("cmd")
        try:
            if cmd == "hello":
                result: Any = {"protocol": PROTOCOL, "pytapo": PYTAPO_VERSION}
            elif cmd == "connect":
                result = session.connect(req)
            elif cmd == "recording_dates":
                result = session.recording_dates(req)
            elif cmd == "list_recordings":
                result = session.list_recordings(req)
            elif cmd == "download":
                result = session.download(
                    req, lambda p: send({"event": "download_progress", "id": req_id, **p})
                )
            elif cmd == "ptz_info":
                result = session.ptz_info(req)
            elif cmd == "ptz_move":
                result = session.ptz_move(req)
            elif cmd == "ptz_goto":
                result = session.ptz_goto(req)
            elif cmd == "disconnect":
                result = session.disconnect(req)
            else:
                raise HelperError("ERR_UNKNOWN_COMMAND", f"Unknown command: {cmd}")
            send({"id": req_id, "ok": True, "result": result})
        except BaseException as exc:  # noqa: BLE001 - every failure must answer
            send(_error_reply(req_id, exc, contexts.get(cmd, CTX_RECORDINGS)))

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            send({"id": None, "ok": False, "code": "ERR_PROTOCOL", "message": "Bad JSON line."})
            continue
        cmd = req.get("cmd")
        if cmd == "cancel":
            session.cancel.set()
            send({"id": req.get("id"), "ok": True, "result": {}})
        elif cmd == "shutdown":
            send({"id": req.get("id"), "ok": True, "result": {}})
            break
        elif cmd == "download":
            # Downloads run beside the reader so "cancel" can still arrive.
            threading.Thread(target=run, args=(req,), daemon=True).start()
        else:
            run(req)


if __name__ == "__main__":
    main()
