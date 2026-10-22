"""The stages this package adds: how a variant's drawn parameters become recorded-file reads and ffmpeg calls.

They take the same ``(ctx, params)`` as DEG's stages (``degrade.stages``) and work on the same 32-bit float working
files, so the merged chain runs them interleaved. The context carries what DEG's does not: the source store, the purpose
(which decides which licences a pick may touch), the partition, the caller's second renders and the list of recorded
files used.

Filters used beyond DEG's: ``pan`` (channels), ``aresample`` (rates), ``volume`` (gain staging), ``highpass``,
``lowpass``, ``lowshelf``, ``highshelf`` and ``equalizer`` (microphone colour), ``adelay`` (offsets). Recorded files are
read from the store after their SHA-256 is checked against the index, and every one used is listed with its licence.
"""

from __future__ import annotations

import hashlib
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ..degrade import ffmpeg, wavio
from ..degrade.stages import Ctx, Skip, _mix_in, _reference_rms, fmt
from . import rir
from .config import AUG_STAGES
from .manifest import LicenceError, Manifest, require_shipped
from .sources import IndexEntry, SourceError, SourceStore

# Components of a microphone's colour above this share of the sample rate are left out (a filter near Nyquist is unstable).
NYQUIST_GUARD = 0.45


@dataclass
class AugCtx(Ctx):
    """DEG's working state plus the recorded sources, the licence rules and the caller's layers."""

    store: SourceStore | None = None
    manifest: Manifest | None = None
    purpose: str = "research"
    partition: str = "train"
    layers: tuple[Path, ...] = ()
    used: list[dict[str, Any]] = field(default_factory=list)


def pick_source(ctx: AugCtx, ref: str, u: float) -> tuple[Path, IndexEntry]:
    """The file ``u`` selects in ``ref``, after the purpose's licence rule and the hash check; recorded in ``ctx.used``."""
    if ctx.store is None or ctx.manifest is None:
        raise SourceError(f"{ref}: this run has no source store")
    use = ctx.manifest.effective_use(ref)
    if ctx.purpose == "shipped-training":
        require_shipped([ref], ctx.manifest)
    elif use == "excluded":
        raise LicenceError(f"{ref} is excluded in the manifest and is never used")
    entry = ctx.store.pick(ref, u, ctx.partition)
    path = ctx.store.path_of(ref, entry, verify=True)
    row = {
        "ref": ref,
        "relpath": entry.relpath,
        "sha256": entry.sha256,
        "licence": ctx.manifest.licence_of(ref),
        "use": use,
    }
    if row not in ctx.used:
        ctx.used.append(row)
    return path, entry


def _channel_filter(nch_in: int, nch_out: int, chan: int) -> str:
    """A ``pan`` that gives ``nch_out`` (1 or 2) channels from a source of ``nch_in``; empty when they already match."""
    if nch_in == nch_out and nch_in <= 2:
        return ""
    if nch_in == 2:  # stereo source, mono clip
        return "pan=mono|c0=0.5*c0+0.5*c1"
    src = f"c{chan}" if nch_in > 2 else "c0"
    return f"pan=mono|c0={src}" if nch_out == 1 else f"pan=stereo|c0={src}|c1={src}"


def prepare_aux(
    ctx: AugCtx, src: Path, *, u_start: float, u_chan: float, delay_samples: int = 0, label: str
) -> tuple[Path, dict[str, Any]]:
    """A float WAV with ``ctx``'s rate, channel count and exact length, cut from ``src`` (a noise, a layer).

    The excerpt starts at ``u_start`` of the slack when the source is longer than the clip, else the source loops. A
    multichannel source gives one channel at ``u_chan``; a mono one is played on both sides of a stereo clip. The excerpt
    starts ``delay_samples`` into the clip.
    """
    try:
        info = wavio.read_info(src)
        material = src
    except wavio.WavError:
        material = ctx.new_path(f"{label}-ingest")
        ctx.call(
            ["-i", "{in}", "-map", "0:a:0", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
            {"in": str(src), "out": str(material)},
        )
        info = wavio.read_info(material)
    if info.frames < 1:
        raise SourceError(f"{src.name}: no samples")
    need = ctx.frames - delay_samples
    if need < 1:
        raise Skip("the offset is beyond the clip")
    need_s = need / ctx.sr
    slack = info.duration_s - need_s
    looped = slack <= 1e-3
    t0 = 0.0 if looped else int(u_start * slack * info.sample_rate) / info.sample_rate
    chan = min(int(u_chan * info.channels), info.channels - 1)
    graph = [f for f in (_channel_filter(info.channels, ctx.channels, chan),) if f]
    if info.sample_rate != ctx.sr:
        graph.append(f"aresample={ctx.sr}")
    graph.append(f"atrim=end_sample={need}")
    if delay_samples:
        graph.append(f"adelay=delays={delay_samples}S:all=1")
    graph.append(f"apad=whole_len={ctx.frames}")
    args: list[str] = []
    if looped:
        args += ["-stream_loop", "-1"]
    if t0 > 0.0:
        args += ["-ss", f"{t0:.9f}"]
    out = ctx.new_path(label)
    args += ["-i", "{in}", "-t", f"{ctx.frames / ctx.sr + 1.0:.6f}", "-af", ",".join(graph), *ffmpeg.OUT_FLAGS]
    ctx.call([*args, "-c:a", "pcm_f32le", "{out}"], {"in": str(material), "out": str(out)})
    got = wavio.read_info(out)
    if (got.sample_rate, got.channels, got.frames) != (ctx.sr, ctx.channels, ctx.frames):
        raise ffmpeg.FfmpegError(
            f"{label}: prepared {got.frames} frames, {got.channels} ch at {got.sample_rate} Hz; "
            f"expected {ctx.frames} frames, {ctx.channels} ch at {ctx.sr} Hz"
        )
    return out, {
        "source_rate_hz": info.sample_rate,
        "source_channels": info.channels,
        "source_s": round(info.duration_s, 4),
        "start_s": round(t0, 6),
        "looped": looped,
    }


# --- mix_in ---------------------------------------------------------------------------------------------------------


def apply_mix_in(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    sig_rms = ctx.level()[0]
    if sig_rms <= 0.0:
        raise Skip("silent input")
    kind = p["kind"]
    info: dict[str, Any] = {}
    if kind == "layer":
        if not ctx.layers:
            raise Skip("no second render was supplied")
        idx = min(int(p["u_layer"] * len(ctx.layers)), len(ctx.layers) - 1)
        src = ctx.layers[idx]
        info["layer"] = {"index": idx, "sha256": wavio.sha256_file(src)}
        u_start = u_chan = 0.0
    elif kind == "layer-recorded":
        src, entry = pick_source(ctx, p["source"], p["u_file"])
        info["source"] = {"ref": p["source"], "relpath": entry.relpath, "sha256": entry.sha256}
        u_start, u_chan = p["u_start"], p["u_chan"]
    else:
        raise ffmpeg.FfmpegError(f"mix_in has no kind {kind!r}")
    delay = min(int(round(p["offset_ms"] / 1000.0 * ctx.sr)), int(0.8 * ctx.frames))
    aux, prep = prepare_aux(ctx, src, u_start=u_start, u_chan=u_chan, delay_samples=delay, label="layer")
    _, ax = wavio.read_wav(aux)
    layer_rms = wavio.rms(ax)
    if layer_rms <= 0.0:
        raise Skip("the second sound is silent")
    gain_db = wavio.to_db(sig_rms) + p["rel_db"] - wavio.to_db(layer_rms)
    _mix_in(ctx, "layer-mix", aux, gain_db)
    ctx.ref_rms = None  # the later noise and hum measure their SNR against the clip with its layer
    return {**info, "prepared": prep, "offset_samples": delay, "gain_db": round(gain_db, 3),
            "signal_rms_dbfs": round(wavio.to_db(sig_rms), 3)}


# --- rir ------------------------------------------------------------------------------------------------------------


def apply_rir(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    src, entry = pick_source(ctx, p["source"], p["u_file"])
    try:
        an = rir.analyse(src, ctx.channels, p["u_chan"])
    except rir.IrError as e:
        raise Skip(str(e)) from e
    tails = an.tails
    if an.sr != ctx.sr:
        raw = ctx.new_path("rir-tail-raw")
        wavio.write_wav_f32(raw, an.sr, ctx.channels, rir.interleave(tails))
        res = ctx.new_path("rir-tail")
        ctx.call(
            ["-i", "{in}", "-af", f"aresample={ctx.sr}", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
            {"in": str(raw), "out": str(res)},
        )
        info, data = wavio.read_wav(res)
        tails = rir.deinterleave(data, info.channels)
    try:
        h, meta = rir.assemble(tails, ctx.sr, an.offset_samples / an.sr, p["wet_db"])
    except rir.IrError as e:
        raise Skip(str(e)) from e
    kernel = ctx.new_path("rir")
    wavio.write_wav_f32(kernel, ctx.sr, ctx.channels, h)
    ring_s = min(0.67 * (an.rt60_s if an.rt60_s else 0.6 * an.tail_s), an.tail_s)
    extend = int(round(ring_s * ctx.sr)) + int(meta["offset_samples"])
    ctx.filter(
        "rir",
        f"[0:a]apad=pad_len={extend}[x];[x][1:a]afir=gtype=none:precision=double",
        aux=[kernel],
        complex_=True,
    )
    return {
        "source": {"ref": p["source"], "relpath": entry.relpath, "sha256": entry.sha256},
        "recording": {
            "sample_rate_hz": an.sr,
            "channels": an.source_channels,
            "frames": an.source_frames,
            "channels_used": an.channels_picked,
            "onset_ms": an.onset_ms,
            "resampled": an.sr != ctx.sr,
        },
        "rt60_s": an.rt60_s,
        "rt60_method": an.rt60_method,
        "tail_s": an.tail_s,
        "wet_db": p["wet_db"],
        "kernel": {**meta, "sha256": hashlib.sha256(h.tobytes()).hexdigest()},
        "extend_samples": extend,
    }


# --- mic ------------------------------------------------------------------------------------------------------------


def apply_mic(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    limit = NYQUIST_GUARD * ctx.sr
    f: list[str] = []
    applied: list[str] = []
    if p["hp_hz"] < limit:
        f.append(f"highpass=f={fmt(p['hp_hz'])}:poles=2")
        applied.append("hp")
    if abs(p["prox_db"]) >= 0.05 and p["prox_hz"] < limit:
        f.append(f"lowshelf=f={fmt(p['prox_hz'])}:width_type=q:w=0.707:g={fmt(p['prox_db'])}")
        applied.append("proximity")
    if abs(p["peak_gain_db"]) >= 0.05 and p["peak_hz"] < limit:
        f.append(f"equalizer=f={fmt(p['peak_hz'])}:width_type=q:w={fmt(p['peak_q'])}:g={fmt(p['peak_gain_db'])}")
        applied.append("presence")
    if abs(p["air_db"]) >= 0.05 and p["air_hz"] < limit:
        f.append(f"highshelf=f={fmt(p['air_hz'])}:width_type=q:w=0.707:g={fmt(p['air_db'])}")
        applied.append("air")
    if p["lp_hz"] < limit:
        f.append(f"lowpass=f={fmt(p['lp_hz'])}:poles=2")
        applied.append("lp")
    if not f:
        raise Skip(f"every component lies above {NYQUIST_GUARD} of the {ctx.sr} Hz sample rate")
    ctx.filter("mic", ",".join(f))
    return {"components": applied}


# --- bg_noise -------------------------------------------------------------------------------------------------------


def apply_bg_noise(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    sig_rms = _reference_rms(ctx)
    src, entry = pick_source(ctx, p["source"], p["u_file"])
    aux, prep = prepare_aux(ctx, src, u_start=p["u_start"], u_chan=p["u_chan"], label="bg-noise")
    _, nx = wavio.read_wav(aux)
    noise_rms = wavio.rms(nx)
    if noise_rms <= 0.0:
        raise Skip("the excerpt of the recorded noise is silent")
    gain_db = wavio.to_db(sig_rms) - p["snr_db"] - wavio.to_db(noise_rms)
    _mix_in(ctx, "bg-noise-mix", aux, gain_db)
    return {
        "source": {"ref": p["source"], "relpath": entry.relpath, "sha256": entry.sha256},
        "prepared": prep,
        "signal_rms_dbfs": round(wavio.to_db(sig_rms), 3),
        "noise_gain_db": round(gain_db, 3),
    }


# --- gain and rate --------------------------------------------------------------------------------------------------


def apply_gain(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    _, pk = ctx.level()
    if pk <= 0.0:
        raise Skip("silent input")
    g = p["peak_dbfs"] - wavio.to_db(pk)
    ctx.filter("gain", f"volume={fmt(g)}dB:precision=double")
    return {"peak_dbfs_before": round(wavio.to_db(pk), 3), "gain_db": round(g, 4)}


def apply_rate(ctx: AugCtx, p: dict[str, Any]) -> dict[str, Any]:
    new = int(p["sr_hz"])
    if new == ctx.sr:
        raise Skip(f"the clip is already at {new} Hz")
    out = ctx.new_path("rate")
    ctx.call(
        ["-i", "{in}", "-af", f"aresample={new}", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
        {"in": str(ctx.cur), "out": str(out)},
    )
    old = ctx.sr
    ctx.cur, ctx.sr = out, new  # Ctx.refresh insists the rate is unchanged, so the new rate is set first
    ctx.refresh()
    return {"from_hz": old, "to_hz": new}


APPLY_AUG: dict[str, Callable[[AugCtx, dict[str, Any]], dict[str, Any] | None]] = {
    "mix_in": apply_mix_in,
    "rir": apply_rir,
    "mic": apply_mic,
    "bg_noise": apply_bg_noise,
    "gain": apply_gain,
    "rate": apply_rate,
}
assert tuple(APPLY_AUG) == AUG_STAGES


def layers_from(paths: Sequence[str | Path] | None) -> tuple[Path, ...]:
    return tuple(Path(p) for p in (paths or ()))
