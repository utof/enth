"""Checkpoints that survive a crash (lane TR2): atomic writes, a manifest with checksums, rotation, and a fallback.

A run directory holds ``ckpt/``:

* ``periodic-<step>.pt``: written every ``ckpt_every_minutes`` (and when a run is stopped or ends); **the newest two
  are kept**, older ones are deleted only after the new one and the manifest are on disk;
* ``best.pt``: the checkpoint with the lowest dev best-of-K so far; replaced only by a better one, never rotated;
* ``manifest.json`` (``enth.train-ckpt/1``): per file its step, epoch, position in the epoch, kind, size, SHA-256 and
  time; each ``.pt`` also has a ``.pt.sha256`` sidecar, so a lost manifest still leaves every checksum.

Every file is written as ``<name>.tmp``, flushed and ``fsync``-ed, then renamed over its final name (and the directory
synced where the file system allows), so a crash mid-write leaves the previous file intact and at worst a stray
``.tmp``. ``load_latest`` tries the newest periodic checkpoint first and falls back to the older one, then to
``best.pt``, skipping any whose bytes do not match their checksum or that ``torch.load`` refuses.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import io
import json
import os
import pathlib
import re

CKPT_SCHEMA = "enth.train-ckpt/1"
DIR = "ckpt"
KEEP_PERIODIC = 2
_PERIODIC = re.compile(r"^periodic-(\d+)\.pt$")


def _fsync_dir(path: pathlib.Path) -> None:
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass  # some file systems (9P, drvfs) refuse a directory fsync; the rename is still atomic there
    finally:
        os.close(fd)


def atomic_write_bytes(path: pathlib.Path, data: bytes) -> None:
    """``path`` gets ``data`` or keeps its old bytes, whatever happens during the write."""
    path = pathlib.Path(path)
    tmp = path.with_name(path.name + ".tmp")
    with open(tmp, "wb") as fh:
        fh.write(data)
        fh.flush()
        os.fsync(fh.fileno())
    os.replace(tmp, path)
    _fsync_dir(path.parent)


def atomic_write_json(path: pathlib.Path, obj) -> None:
    atomic_write_bytes(path, (json.dumps(obj, indent=1) + "\n").encode())


def _utc() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


class CheckpointStore:
    def __init__(self, run_dir: pathlib.Path | str, keep: int = KEEP_PERIODIC) -> None:
        self.run_dir = pathlib.Path(run_dir)
        self.dir = self.run_dir / DIR
        self.keep = keep

    # -- manifest ---------------------------------------------------------------------------------------------------
    @property
    def manifest_path(self) -> pathlib.Path:
        return self.dir / "manifest.json"

    def manifest(self) -> dict:
        """The manifest, or one rebuilt from the sidecars when it is missing or unreadable."""
        try:
            m = json.loads(self.manifest_path.read_text())
            if m.get("schema") == CKPT_SCHEMA and isinstance(m.get("entries"), list):
                return m
        except (OSError, ValueError):
            pass
        return self._rebuild()

    def _rebuild(self) -> dict:
        entries = []
        if self.dir.exists():
            for p in sorted(self.dir.glob("*.pt")):
                side = p.with_name(p.name + ".sha256")
                try:
                    info = json.loads(side.read_text())
                except (OSError, ValueError):
                    info = {"file": p.name, "sha256": None}
                entries.append(info)
        return {"schema": CKPT_SCHEMA, "entries": entries, "rebuilt": True}

    def entries(self, kind: str | None = None) -> list[dict]:
        es = self.manifest()["entries"]
        return [e for e in es if kind is None or e.get("kind") == kind]

    def has_checkpoint(self) -> bool:
        return self.dir.exists() and any(self.dir.glob("*.pt"))

    # -- writing ----------------------------------------------------------------------------------------------------
    def save(self, state: dict, kind: str, meta: dict) -> dict:
        """Write ``state`` as a ``periodic`` (also used for stops and the end) or ``best`` checkpoint; returns its
        manifest entry. ``meta`` (step, epoch, position, dev loss, reason) goes into the entry."""
        import torch

        self.dir.mkdir(parents=True, exist_ok=True)
        buf = io.BytesIO()
        torch.save(state, buf)
        data = buf.getvalue()
        name = "best.pt" if kind == "best" else f"periodic-{int(meta['step']):09d}.pt"
        entry = {
            "file": name,
            "kind": "best" if kind == "best" else "periodic",
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
            "utc": _utc(),
            **meta,
        }
        atomic_write_bytes(self.dir / name, data)
        atomic_write_json(self.dir / (name + ".sha256"), entry)
        m = self.manifest()
        es = [e for e in m["entries"] if e.get("file") != name]
        es.append(entry)
        if kind != "best":
            periodic = sorted((e for e in es if e.get("kind") == "periodic"), key=lambda e: int(e.get("step", -1)))
            drop = periodic[: max(0, len(periodic) - self.keep)]
            es = [e for e in es if e not in drop]
        else:
            drop = []
        atomic_write_json(self.manifest_path, {"schema": CKPT_SCHEMA, "entries": es})
        # Only now that the new file and the manifest are durable: remove what rotated out (and strays).
        keep_files = {e["file"] for e in es}
        for e in drop:
            for p in (self.dir / e["file"], self.dir / (e["file"] + ".sha256")):
                p.unlink(missing_ok=True)
        for p in self.dir.glob("periodic-*.pt"):
            if p.name not in keep_files and _PERIODIC.match(p.name):
                p.unlink(missing_ok=True)
                p.with_name(p.name + ".sha256").unlink(missing_ok=True)
        return entry

    # -- reading ----------------------------------------------------------------------------------------------------
    def verify(self, entry: dict) -> bool:
        p = self.dir / entry["file"]
        try:
            data = p.read_bytes()
        except OSError:
            return False
        if entry.get("bytes") is not None and len(data) != int(entry["bytes"]):
            return False
        return entry.get("sha256") is None or hashlib.sha256(data).hexdigest() == entry["sha256"]

    def candidates(self) -> list[dict]:
        """Resume order: periodic checkpoints newest first, then ``best``."""
        es = self.entries()
        periodic = sorted((e for e in es if e.get("kind") == "periodic"), key=lambda e: -int(e.get("step", -1)))
        best = [e for e in es if e.get("kind") == "best"]
        # Files on disk that the manifest does not know (a crash between the rename and the manifest): try them too.
        known = {e["file"] for e in es}
        extra = []
        if self.dir.exists():
            for p in self.dir.glob("periodic-*.pt"):
                if p.name not in known and (m := _PERIODIC.match(p.name)):
                    try:
                        info = json.loads(p.with_name(p.name + ".sha256").read_text())
                    except (OSError, ValueError):
                        info = {"file": p.name, "sha256": None, "step": int(m.group(1)), "kind": "periodic"}
                    extra.append(info)
        extra.sort(key=lambda e: -int(e.get("step", -1)))
        allp = sorted(periodic + extra, key=lambda e: -int(e.get("step", -1)))
        return allp + best

    def load_latest(self, log=print) -> tuple[dict | None, dict | None]:
        """The newest checkpoint that passes its checksum and loads; ``(None, None)`` when there is none."""
        import torch  # here, not at the top: ``status`` and ``stop`` never pay for importing torch

        for e in self.candidates():
            if not self.verify(e):
                log(f"checkpoint {e['file']}: missing, truncated or failed its SHA-256; trying an older one")
                continue
            try:
                state = torch.load(self.dir / e["file"], map_location="cpu", weights_only=False)
            except Exception as err:  # noqa: BLE001 - any unreadable file falls back
                log(f"checkpoint {e['file']}: does not load ({err!r}); trying an older one")
                continue
            return state, e
        return None, None

    def best(self) -> dict | None:
        b = self.entries("best")
        return b[0] if b else None
