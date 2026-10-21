"""The stages: how a variant's drawn parameters become ffmpeg calls.

Each stage has a ``sample`` step (pure: draws the stage's parameters from a seeded stream) and an ``apply`` step (runs
ffmpeg on the working file). Working files are 32-bit float WAVs in a temporary directory; between stages Python
measures the level (RMS and peak) where a stage is defined against it (noise and hum against an SNR, clipping against
the peak). One ffmpeg call per filter step keeps every step's input and output inspectable.

Filters used: ``atrim``, ``afade``, ``adelay``, ``apad``, ``afir`` (room), ``highpass``, ``lowpass``, ``equalizer``,
``lowshelf``, ``highshelf`` (device EQ), ``anoisesrc``, ``aevalsrc``, ``tremolo``, ``amix`` (noise and hum),
``alimiter``, ``asoftclip``, ``acompressor`` (dynamics), ``pan`` (mono), ``aresample`` (band limit), ``aeval``
(bit depth), ``ebur128`` (measurement), ``aloop`` (short clips) and ``volume`` (normalisation).
"""

from __future__ import annotations

import math
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import ffmpeg, ir, wavio
from .pools import STAGE_ORDER, Variant, draw, draw_all
from .rng import Rng, sig6

PI = math.pi
# alimiter's ceiling: -1 dBFS.
LIMIT_LIN = 10.0 ** (-1.0 / 20.0)
# Headroom kept by the final gain: the peak never ends above -0.1 dBFS, so the 16-bit file cannot clip.
PEAK_CEILING_DB = -0.1
# Encoder sample rates libshine supports; any other input is resampled to 48 kHz first.
SHINE_RATES = (32000, 44100, 48000)


class Skip(Exception):
    """An active stage that does nothing for this input (a mono input to the mono stage); the reason is recorded."""


def fmt(x: float) -> str:
    """A float for a filter argument: 9 significant digits, no exponent surprises."""
    s = f"{x:.9g}"
    return s


@dataclass
class Ctx:
    """The working state of one chain run."""

    dir: Path
    sr: int
    channels: int
    frames: int
    cur: Path
    n: int = 0
    calls: list[list[str]] = field(default_factory=list)
    # The clip's RMS before any additive noise: noise and hum both measure their SNR against it, so a hum after noise
    # is not referenced to the noise's own power.
    ref_rms: float | None = None

    def new_path(self, label: str, ext: str = "wav") -> Path:
        self.n += 1
        return self.dir / f"{self.n:02d}-{label}.{ext}"

    def call(self, args: list[str], files: dict[str, str]) -> str:
        self.calls.append(list(args))
        return ffmpeg.run(args, files)

    def refresh(self) -> None:
        info = wavio.read_info(self.cur)
        if info.sample_rate != self.sr:
            raise ffmpeg.FfmpegError(f"{self.cur.name}: sample rate changed to {info.sample_rate} (expected {self.sr})")
        self.channels, self.frames = info.channels, info.frames

    def filter(self, label: str, graph: str, *, aux: list[Path] | None = None, complex_: bool = False) -> Path:
        """Run ``graph`` on the current file (plus ``aux`` inputs as ``[1:a]``...); the result becomes current."""
        out = self.new_path(label)
        args = ["-i", "{in}"]
        files = {"in": str(self.cur), "out": str(out)}
        for i, p in enumerate(aux or []):
            args += ["-i", f"{{aux{i}}}"]
            files[f"aux{i}"] = str(p)
        args += ["-filter_complex" if complex_ else "-af", graph, *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"]
        self.call(args, files)
        self.cur = out
        self.refresh()
        return out

    def read(self) -> tuple[wavio.WavInfo, Any]:
        return wavio.read_wav(self.cur)

    def level(self) -> tuple[float, float]:
        """(RMS over all samples, absolute peak) of the current file."""
        _, x = self.read()
        return wavio.rms(x), wavio.peak(x)


# --- sampling -----------------------------------------------------------------------------------------------------

Instance = Callable[[str], int]


def _eq_components(p: dict[str, Any], variant: Variant, rng: Rng) -> dict[str, Any]:
    spec = variant.params
    out: dict[str, Any] = {"kind": p["kind"]}

    def present(key: str) -> bool:
        return rng.chance(float(spec[key])) if key in spec else True

    if "hp_hz" in spec and present("p_hp"):
        out["hp"] = {"f_hz": p["hp_hz"], "order": int(p.get("hp_order", 2))}
    if "lp_hz" in spec and present("p_lp"):
        out["lp"] = {"f_hz": p["lp_hz"], "order": int(p.get("lp_order", 2))}
    peaks = []
    for _ in range(int(p.get("n_peaks", 0))):
        peaks.append(
            {
                "f_hz": draw(spec["peak_hz"], rng),
                "gain_db": draw(spec["peak_gain_db"], rng),
                "q": draw(spec["peak_q"], rng),
            }
        )
    if peaks:
        out["peaks"] = peaks
    if "tilt_db_oct" in spec and present("p_tilt"):
        out["tilt_db_oct"] = p["tilt_db_oct"]
    return out


def sample_stage(stage: str, variant: Variant, rng: Rng, inst: Instance) -> dict[str, Any]:
    """Draw the parameters of ``variant`` for ``stage``. Pure: the same stream gives the same parameters."""
    p = draw_all(variant, rng)
    if stage == "eq":
        return _eq_components(p, variant, rng)
    if stage == "room":
        p["seed"] = inst("ir")
    elif stage == "noise":
        p["seed"] = inst("noise")
    elif stage == "hum":
        p["phases"] = [sig6(rng.uniform(0.0, 2.0 * PI)) for _ in range(int(p["n_harm"]))]
    elif stage == "mono":
        p["louder"] = rng.choice(["L", "R"])
    elif stage == "bit_depth" and p["kind"] == "quantise":
        p["seed"] = inst("dither")
    return p


# --- applying -----------------------------------------------------------------------------------------------------


def apply_attack_cut(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    cut = min(p["cut_ms"] / 1000.0, 0.3 * ctx.frames / ctx.sr)
    k = int(round(cut * ctx.sr))
    if k < 1:
        raise Skip("cut is under one sample")
    ctx.filter("attack-cut", f"atrim=start_sample={k},asetpts=PTS-STARTPTS")
    return {"cut_samples": k}


def apply_tail_trunc(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    dur = ctx.frames / ctx.sr
    keep = min(p["keep_s"], 0.85 * dur)
    e = max(int(round(keep * ctx.sr)), 1)
    if e >= ctx.frames:
        raise Skip("nothing to cut")
    nf = max(int(round(min(p["fade_ms"] / 1000.0, 0.5 * keep) * ctx.sr)), 1)
    ctx.filter("tail-trunc", f"atrim=end_sample={e},afade=t=out:ss={e - nf}:ns={nf}")
    return {"kept_samples": e, "fade_samples": nf}


def apply_lead_pad(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    d = int(round(p["pad_ms"] / 1000.0 * ctx.sr))
    if d < 1:
        raise Skip("pad is under one sample")
    ctx.filter("lead-pad", f"adelay=delays={d}S:all=1")
    return {"pad_samples": d}


def apply_room(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    h, meta = ir.synthesize(ctx.sr, ctx.channels, p, int(p["seed"]))
    path = ctx.new_path("ir")
    wavio.write_wav_f32(path, ctx.sr, ctx.channels, h)
    # Let the room ring on after the clip ends, down to about -40 dB (two thirds of the RT60), and the predelay.
    extend = int(round((p.get("predelay_ms", 0.0) / 1000.0 + 0.67 * p["rt60_s"]) * ctx.sr))
    ctx.filter(
        "room",
        f"[0:a]apad=pad_len={extend}[x];[x][1:a]afir=gtype=none:precision=double",
        aux=[path],
        complex_=True,
    )
    return {"ir": meta, "extend_samples": extend}


def apply_eq(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    f: list[str] = []
    for key, name in (("hp", "highpass"), ("lp", "lowpass")):
        if key in p:
            f += [f"{name}=f={fmt(p[key]['f_hz'])}:poles=2"] * (int(p[key]["order"]) // 2)
    for pk in p.get("peaks", []):
        f.append(f"equalizer=f={fmt(pk['f_hz'])}:width_type=q:w={fmt(pk['q'])}:g={fmt(pk['gain_db'])}")
    if "tilt_db_oct" in p:
        # A tilt of T dB per octave, approximated by a pair of shelves 3.3 octaves apart (300 Hz to 3 kHz).
        half = 1.66 * p["tilt_db_oct"]
        f.append(f"lowshelf=f=300:width_type=q:w=0.707:g={fmt(-half)}")
        f.append(f"highshelf=f=3000:width_type=q:w=0.707:g={fmt(half)}")
    if not f:
        raise Skip("no EQ component drawn")
    ctx.filter("eq", ",".join(f))
    return {}


def noise_source(label: str, color: str, sr: int, dur_s: float, seeds: list[int], post: str = "") -> str:
    """A filtergraph fragment that ends in pad ``[label]``: seeded noise, one independent source per channel."""
    parts = [
        f"anoisesrc=d={dur_s:.6f}:c={color}:r={sr}:a=1:s={s}[{label}{c}]" for c, s in enumerate(seeds)
    ]
    tail = f",{post}" if post else ""
    if len(seeds) == 1:
        return f"{parts[0][: parts[0].rindex('[')]}{tail}[{label}]"
    joined = ";".join(parts)
    pads = "".join(f"[{label}{c}]" for c in range(len(seeds)))
    return f"{joined};{pads}amerge=inputs={len(seeds)},aformat=channel_layouts=stereo{tail}[{label}]"


def _mix_in(ctx: Ctx, label: str, aux: Path, gain_db: float) -> None:
    ctx.filter(
        label,
        f"[1:a]volume={fmt(gain_db)}dB[n];[0:a][n]amix=inputs=2:normalize=0:duration=first",
        aux=[aux],
        complex_=True,
    )


def _reference_rms(ctx: Ctx) -> float:
    if ctx.ref_rms is None:
        ctx.ref_rms = ctx.level()[0]
    if ctx.ref_rms <= 0.0:
        raise Skip("silent input")
    return ctx.ref_rms


def apply_noise(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    sig_rms = _reference_rms(ctx)
    post = []
    if "bp_lo_hz" in p:
        post.append(f"highpass=f={fmt(p['bp_lo_hz'])}:poles=2,lowpass=f={fmt(p['bp_hi_hz'])}:poles=2")
    if "am_hz" in p:
        post.append(f"tremolo=f={fmt(p['am_hz'])}:d={fmt(p['am_depth'])}")
    post.append(f"atrim=end_sample={ctx.frames}")  # exactly the clip's length, so the RMS below is the mixed part's
    seeds = [int(p["seed"]) + 2 * c for c in range(ctx.channels)]
    graph = noise_source("n", p["color"], ctx.sr, ctx.frames / ctx.sr + 0.05, seeds, ",".join(post))
    noise = ctx.new_path("noise")
    ctx.call(
        ["-filter_complex", graph, "-map", "[n]", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
        {"out": str(noise)},
    )
    _, nx = wavio.read_wav(noise)
    noise_rms = wavio.rms(nx)
    if noise_rms <= 0.0:
        raise ffmpeg.FfmpegError("the noise source produced silence")
    gain_db = wavio.to_db(sig_rms) - p["snr_db"] - wavio.to_db(noise_rms)
    _mix_in(ctx, "noise-mix", noise, gain_db)
    return {"signal_rms_dbfs": round(wavio.to_db(sig_rms), 3), "noise_gain_db": round(gain_db, 3)}


def apply_hum(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    sig_rms = _reference_rms(ctx)
    n = int(p["n_harm"])
    amps = [k ** -p["slope"] for k in range(1, n + 1)]
    hum_rms = math.sqrt(sum(a * a for a in amps) / 2.0)
    scale = sig_rms * 10.0 ** (-p["snr_db"] / 20.0) / hum_rms
    expr = "+".join(
        f"{scale * a:.9g}*sin(2*PI*{k * p['f0_hz']:.9g}*t+{ph:.6g})"
        for k, (a, ph) in enumerate(zip(amps, p["phases"], strict=True), start=1)
    )
    layout = "mono" if ctx.channels == 1 else "stereo"
    dur = ctx.frames / ctx.sr + 0.05
    graph = (
        f"aevalsrc=exprs='{'|'.join([expr] * ctx.channels)}':s={ctx.sr}:d={dur:.6f}:c={layout}[h];"
        "[0:a][h]amix=inputs=2:normalize=0:duration=first"
    )
    ctx.filter("hum", graph, complex_=True)
    return {"signal_rms_dbfs": round(wavio.to_db(sig_rms), 3), "hum_scale": float(f"{scale:.6g}")}


def apply_dynamics(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    _, pk = ctx.level()
    if pk <= 0.0:
        raise Skip("silent input")
    kind = p["kind"]
    if kind == "compressor":
        thr = min(1.0, max(pk * 10.0 ** (-p["thr_below_peak_db"] / 20.0), 0.000976563))
        makeup = min(64.0, max(1.0, 10.0 ** (p["makeup_db"] / 20.0)))
        graph = (
            f"acompressor=threshold={fmt(thr)}:ratio={fmt(p['ratio'])}:attack={fmt(p['attack_ms'])}"
            f":release={fmt(p['release_ms'])}:makeup={fmt(makeup)}"
        )
        info: dict[str, Any] = {"peak_dbfs_before": round(wavio.to_db(pk), 3), "threshold_lin": float(fmt(thr))}
    else:
        pre = p["gain_db"] - wavio.to_db(pk)  # the peak lands gain_db above full scale
        if kind == "limiter":
            tail = f"alimiter=limit={fmt(LIMIT_LIN)}:attack={fmt(p['attack_ms'])}:release={fmt(p['release_ms'])}:level=0:latency=1"
        else:
            tail = f"asoftclip=type={kind}"
        graph = f"volume={fmt(pre)}dB,{tail}"
        info = {"peak_dbfs_before": round(wavio.to_db(pk), 3), "pre_gain_db": round(pre, 4)}
    ctx.filter("dynamics", graph)
    return info


def apply_mono(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    if ctx.channels != 2:
        raise Skip(f"input has {ctx.channels} channel(s)")
    g = 10.0 ** (p["imbalance_db"] / 40.0)
    gl, gr = (g, 1.0 / g) if p["louder"] == "L" else (1.0 / g, g)
    wl, wr = sig6(0.5 * gl), sig6(0.5 * p["r_polarity"] * gr)
    ctx.filter("mono", f"pan=mono|c0={fmt(wl)}*c0{wr:+.9g}*c1")
    return {"weights": [wl, wr]}


def apply_band_limit(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    low = int(p["sr_hz"])
    if low >= ctx.sr:
        raise Skip(f"input rate {ctx.sr} Hz is not above {low} Hz")
    ctx.filter("band-limit", f"aresample={low},aresample={ctx.sr}")
    return {"nyquist_hz": low / 2.0}


def _encode_args(p: dict[str, Any]) -> list[str]:
    enc = p["encoder"]
    if enc == "libmp3lame":
        if p["mode"] == "vbr":
            return ["-q:a", str(p["qscale"])]
        extra = ["-abr", "1"] if p["mode"] == "abr" else []
        return [*extra, "-b:a", f"{p['bitrate_kbps']}k"]
    if enc == "libshine":
        return ["-b:a", f"{p['bitrate_kbps']}k"]
    if enc == "aac":
        return ["-aac_coder", p["aac_coder"], "-b:a", f"{p['bitrate_kbps']}k"]
    if enc == "libopus":
        return ["-application", p["application"], "-b:a", f"{p['bitrate_kbps']}k"]
    if enc == "libopencore_amrnb":
        return ["-b:a", str(p["bitrate_bps"])]
    if enc == "libgsm":
        return []
    raise ffmpeg.FfmpegError(f"no encoder arguments for {enc!r}")


def apply_codec(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    enc = p["encoder"]
    if enc not in ffmpeg.encoders():
        raise ffmpeg.FfmpegError(f"this ffmpeg has no {enc} encoder")
    if ctx.channels > 2:
        raise ffmpeg.FfmpegError("codecs here take mono or stereo")
    pre: list[str] = []
    if p.get("enc_mono") and ctx.channels == 2:
        pre.append("pan=mono|c0=0.5*c0+0.5*c1")
    if "enc_rate_hz" in p:
        pre.append(f"aresample={p['enc_rate_hz']}")
    elif enc == "libshine" and ctx.sr not in SHINE_RATES:
        pre.append("aresample=48000")
    coded = ctx.new_path("coded", p["ext"])
    args = ["-i", "{in}"]
    if pre:
        args += ["-af", ",".join(pre)]
    args += [*ffmpeg.OUT_FLAGS, "-c:a", enc, *_encode_args(p), "{coded}"]
    ctx.call(args, {"in": str(ctx.cur), "coded": str(coded)})
    # Decode back to the clip's rate and channels. ffmpeg compensates the encoder delay of every codec used here (it
    # reads LAME's gapless tag and the MP4 and Ogg Opus trims), so only the length needs fixing: cut or pad with
    # silence to the length the clip had before the codec.
    post = []
    if p.get("enc_mono") and ctx.channels == 2:
        post.append("pan=stereo|c0=c0|c1=c0")
    post += [f"aresample={ctx.sr}", f"atrim=end_sample={ctx.frames}", f"apad=whole_len={ctx.frames}"]
    frames = ctx.frames
    out = ctx.new_path("decoded")
    ctx.call(
        ["-i", "{coded}", "-af", ",".join(post), *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
        {"coded": str(coded), "out": str(out)},
    )
    ctx.cur = out
    ctx.refresh()
    if ctx.frames != frames:
        raise ffmpeg.FfmpegError(f"codec round trip changed the length from {frames} to {ctx.frames} samples")
    size = coded.stat().st_size
    return {"coded_bytes": size, "effective_kbps": round(size * 8.0 / (frames / ctx.sr) / 1000.0, 2)}


def apply_bit_depth(ctx: Ctx, p: dict[str, Any]) -> dict[str, Any]:
    if p["kind"] == "law":
        coded = ctx.new_path("law")
        codec = "pcm_mulaw" if p["law"] == "mulaw" else "pcm_alaw"
        ctx.call(["-i", "{in}", *ffmpeg.OUT_FLAGS, "-c:a", codec, "{coded}"], {"in": str(ctx.cur), "coded": str(coded)})
        out = ctx.new_path("decoded")
        ctx.call(
            ["-i", "{coded}", *ffmpeg.OUT_FLAGS, "-c:a", "pcm_f32le", "{out}"],
            {"coded": str(coded), "out": str(out)},
        )
        ctx.cur = out
        ctx.refresh()
        return {"codec": codec}
    bits = int(p["bits"])
    q = float(2 ** (bits - 1))
    quant = f"clip(floor(val(ch)*{q:.0f}+0.5),-{q:.0f},{q - 1:.0f})/{q:.0f}"
    if p["dither"]:
        # TPDF dither: two independent uniform sources of +-0.5 LSB each, added before rounding.
        dur = ctx.frames / ctx.sr + 0.05
        a = 0.5 / q
        seeds_a = [int(p["seed"]) + 2 * c for c in range(ctx.channels)]
        seeds_b = [int(p["seed"]) + 2 * (ctx.channels + c) for c in range(ctx.channels)]
        graph = (
            f"{_dither_source('da', ctx.sr, dur, a, seeds_a)};{_dither_source('db', ctx.sr, dur, a, seeds_b)};"
            f"[0:a][da][db]amix=inputs=3:normalize=0:duration=first,aeval=exprs='{quant}':c=same"
        )
        ctx.filter("bit-depth", graph, complex_=True)
    else:
        ctx.filter("bit-depth", f"aeval=exprs='{quant}':c=same")
    return {"levels": int(q * 2)}


def _dither_source(label: str, sr: int, dur: float, amp: float, seeds: list[int]) -> str:
    parts = [f"anoisesrc=d={dur:.6f}:c=white:r={sr}:a={amp:.9g}:s={s}[{label}{c}]" for c, s in enumerate(seeds)]
    if len(seeds) == 1:
        return parts[0].replace(f"[{label}0]", f"[{label}]")
    pads = "".join(f"[{label}{c}]" for c in range(len(seeds)))
    return f"{';'.join(parts)};{pads}amerge=inputs={len(seeds)},aformat=channel_layouts=stereo[{label}]"


# --- loudness normalisation and the final file --------------------------------------------------------------------

_R128_I = re.compile(r"lavfi\.r128\.I=(-?[0-9.]+)")


def measure_lufs(ctx: Ctx, path: Path, extra: str = "") -> float | None:
    """Integrated loudness of ``path`` by ffmpeg's ``ebur128`` filter (libebur128, the meter enth's own is a port of).

    ``loudnorm``'s own reading differs from it by up to a dB on clips of a second or less, so it is not used here.
    None when the clip holds no gating block (under 400 ms), is under 100 ms, or is below -70 LUFS.
    """
    graph = f"{extra}ebur128=metadata=1:framelog=quiet,ametadata=mode=print:key=lavfi.r128.I"
    err = ctx.call(["-loglevel", "info", "-i", "{in}", "-af", graph, "-f", "null", "-"], {"in": str(path)})
    values = _R128_I.findall(err)
    if not values:  # ebur128 reports once per 100 ms of audio: a shorter clip prints nothing
        return None
    lufs = float(values[-1])  # the last frame carries the integrated loudness of the whole clip
    return lufs if math.isfinite(lufs) and lufs > -70.0 else None


def normalise_and_write(ctx: Ctx, target_lufs: float, out_path: Path, sample_fmt: str) -> dict[str, Any]:
    """Bring the clip to ``target_lufs`` with one linear gain and write the final WAV."""
    lufs = measure_lufs(ctx, ctx.cur)
    method = "ebur128"
    if lufs is None:
        # A clip under 400 ms holds no BS.1770 gating block (the meter reads -70). Loop it to 1.5 s and measure that:
        # a looped clip has the same mean power as the clip, which is what enth's own meter reads for short sounds.
        loops = max(int(math.ceil(1.5 * ctx.sr / ctx.frames)), 1)
        lufs = measure_lufs(ctx, ctx.cur, extra=f"aloop=loop={loops}:size={ctx.frames},")
        method = "ebur128 on the clip looped to 1.5 s"
    gain_db = None
    result: dict[str, Any] = {"target_lufs": target_lufs, "measured_lufs": None if lufs is None else round(lufs, 3), "method": method}
    _, pk = ctx.level()
    if lufs is None or pk <= 0.0:
        gain_db = 0.0
        result["skipped"] = "silent: nothing to normalise"
    else:
        gain_db = target_lufs - lufs
        ceiling = PEAK_CEILING_DB - wavio.to_db(pk)
        if gain_db > ceiling:
            gain_db = ceiling
            result["peak_limited"] = True
    result["gain_db"] = round(gain_db, 4)
    codec = {"s16": "pcm_s16le", "s24": "pcm_s24le", "f32": "pcm_f32le"}[sample_fmt]
    ctx.call(
        ["-i", "{in}", "-af", f"volume={gain_db:.6f}dB:precision=double", *ffmpeg.OUT_FLAGS, "-c:a", codec, "{out}"],
        {"in": str(ctx.cur), "out": str(out_path)},
    )
    return result


APPLY: dict[str, Callable[[Ctx, dict[str, Any]], dict[str, Any] | None]] = {
    "attack_cut": apply_attack_cut,
    "tail_trunc": apply_tail_trunc,
    "lead_pad": apply_lead_pad,
    "room": apply_room,
    "eq": apply_eq,
    "noise": apply_noise,
    "hum": apply_hum,
    "dynamics": apply_dynamics,
    "mono": apply_mono,
    "band_limit": apply_band_limit,
    "codec": apply_codec,
    "bit_depth": apply_bit_depth,
}
assert tuple(APPLY) == STAGE_ORDER
