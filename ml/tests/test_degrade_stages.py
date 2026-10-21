"""Lane DEG: each stage does what its parameters say, checked against the intermediate float files.

The chain keeps one working file per ffmpeg step (``work_dir``), so a stage's input and output can be compared:
SNRs to a few thousandths of a dB, a room against the impulse response it was built from, a codec's time alignment.
Standard library plus the system ffmpeg; skipped without it.
"""

from __future__ import annotations

import math
import pathlib
import subprocess
import tempfile
import unittest

import degrade_support as support
from degrade_support import HAVE_FFMPEG, find_seed, have_encoder
from enth_ml.degrade import degrade_file, ir, wavio


def rt60_from(h: list[float], sr: int) -> float:
    """RT60 by Schroeder backward integration, extrapolated from the -5 to -25 dB span."""
    energy = [v * v for v in h[1:]]
    acc, cum = 0.0, []
    for v in reversed(energy):
        acc += v
        cum.append(acc)
    cum.reverse()
    db = [10.0 * math.log10(c / cum[0]) for c in cum]
    first = lambda level: next(i for i, v in enumerate(db) if v < level) / sr  # noqa: E731
    return 3.0 * (first(-25.0) - first(-5.0))


def band_power(x, sr: int, lo: float, hi: float, n: int = 24) -> float:
    """Mean power of ``x`` at ``n`` frequencies between ``lo`` and ``hi`` Hz (Goertzel, Hann window; standard library)."""
    m = len(x)
    x = [v * (0.5 - 0.5 * math.cos(2.0 * math.pi * i / (m - 1))) for i, v in enumerate(x)]  # no leakage floor
    total = 0.0
    for k in range(n):
        f = lo + (hi - lo) * (k + 0.5) / n
        w = 2.0 * math.cos(2.0 * math.pi * f / sr)
        s1 = s2 = 0.0
        for v in x:
            s1, s2 = v + w * s1 - s2, s1
        total += s1 * s1 + s2 * s2 - w * s1 * s2
    return total / n


class ImpulseResponses(unittest.TestCase):
    base = {"kind": "diffuse", "rt60_s": 1.0, "hf_ratio": 1.0, "predelay_ms": 0.0, "n_early": 10, "er_fraction": 0.01,
            "wet_db": -6.0, "early_ms": 20.0, "damp_hz": 3000.0}

    def test_the_decay_time_is_the_rt60_asked_for(self) -> None:
        for rt60 in (0.3, 1.0, 2.4):
            h, _ = ir.synthesize(48000, 1, {**self.base, "rt60_s": rt60}, seed=1001)
            self.assertAlmostEqual(rt60_from(list(h), 48000) / rt60, 1.0, delta=0.06, msg=str(rt60))

    def test_wet_db_is_the_reverberant_to_direct_energy_ratio(self) -> None:
        for wet in (-20.0, -6.0, 0.0):
            h, meta = ir.synthesize(48000, 2, {**self.base, "wet_db": wet}, seed=7)
            self.assertAlmostEqual(meta["wet_db_actual"], wet, delta=0.05)
            self.assertEqual(h[0], 1.0)  # the direct impulse
            self.assertEqual(h[1], 1.0)

    def test_same_seed_same_ir_and_channels_differ(self) -> None:
        a, ma = ir.synthesize(48000, 2, self.base, seed=5)
        b, mb = ir.synthesize(48000, 2, self.base, seed=5)
        c, mc = ir.synthesize(48000, 2, self.base, seed=7)
        self.assertEqual(ma, mb)
        self.assertNotEqual(ma["sha256"], mc["sha256"])
        left, right = list(a[2::2]), list(a[3::2])
        self.assertNotEqual(left, right)
        corr = math.fsum(x * y for x, y in zip(left, right)) / math.sqrt(
            math.fsum(x * x for x in left) * math.fsum(x * x for x in right)
        )
        self.assertLess(abs(corr), 0.05)

    def test_the_flutter_room_repeats_at_its_spacing(self) -> None:
        p = {**self.base, "kind": "flutter", "flutter_ms": 30.0, "n_early": 5, "er_fraction": 0.99, "rt60_s": 5.0}
        h, _ = ir.synthesize(48000, 1, p, seed=3)
        spacing = int(0.030 * 48000)
        for k in range(1, 4):
            window = [abs(h[i]) for i in range(1 + k * spacing - 60, 1 + k * spacing + 60)]
            self.assertGreater(max(window), 0.05, f"echo {k}")


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not installed")
class Stages(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self._td.name)
        self.src = self.dir / "in.wav"
        support.write_pluck(self.src, dur=0.6)
        self.out = self.dir / "out.wav"
        self.work = self.dir / "work"

    def tearDown(self) -> None:
        self._td.cleanup()

    def run_chain(self, seed: int, pool: str, profile: str | None, src: pathlib.Path | None = None) -> tuple[dict, pathlib.Path]:
        rec = degrade_file(src or self.src, self.out, seed=seed, pool=pool, profile=profile, work_dir=self.work)
        return rec, self.work

    def step(self, name: str) -> pathlib.Path:
        found = sorted(self.work.glob(f"*-{name}.wav"))
        self.assertEqual(len(found), 1, f"{name}: {found}")
        return found[0]

    @staticmethod
    def stage(rec: dict, name: str) -> dict:
        return next(s for s in rec["stages"] if s["stage"] == name)

    def test_noise_and_hum_hit_their_snr(self) -> None:
        for pool, seed in (("train", 1), ("heldout", 1), ("heldout", 2), ("heldout", 5)):
            with self.subTest(pool=pool, seed=seed):
                rec, work = self.run_chain(seed, pool, "noise-hum")
                clean = wavio.read_wav(work / "00-ingest.wav")[1]
                noisy = wavio.read_wav(self.step("noise-mix"))[1]
                humming = wavio.read_wav(self.step("hum"))[1]
                added_noise = [b - a for a, b in zip(clean, noisy, strict=True)]
                added_hum = [c - b for b, c in zip(noisy, humming, strict=True)]
                # Both SNRs are against the clip's RMS before any added noise.
                snr_noise = 20 * math.log10(wavio.rms(clean) / wavio.rms(added_noise))
                snr_hum = 20 * math.log10(wavio.rms(clean) / wavio.rms(added_hum))
                self.assertAlmostEqual(snr_noise, self.stage(rec, "noise")["params"]["snr_db"], delta=0.01)
                self.assertAlmostEqual(snr_hum, self.stage(rec, "hum")["params"]["snr_db"], delta=0.01)

    def test_stereo_noise_is_independent_between_channels(self) -> None:
        # White noise: a low-colour noise has too few independent samples in 0.6 s for a correlation to settle.
        self.run_chain(find_seed("train", "noise-hum", "noise", "white"), "train", "noise-hum")
        noise = wavio.read_wav(self.step("noise"))[1]
        left, right = noise[0::2], noise[1::2]
        corr = math.fsum(x * y for x, y in zip(left, right)) / math.sqrt(
            math.fsum(x * x for x in left) * math.fsum(x * x for x in right)
        )
        self.assertLess(abs(corr), 0.05)

    def test_dynamics_put_the_peak_where_the_variant_says(self) -> None:
        cases = {
            ("train", "hard-clip"): lambda peak: self.assertAlmostEqual(peak, 1.0, places=4),
            ("train", "tanh-clip"): lambda peak: self.assertLess(peak, 1.0),
            ("train", "limiter"): lambda peak: self.assertAlmostEqual(peak, 10 ** (-1 / 20), delta=0.002),
            ("heldout", "limiter-hot"): lambda peak: self.assertAlmostEqual(peak, 10 ** (-1 / 20), delta=0.002),
            ("heldout", "hard-clip-hot"): lambda peak: self.assertAlmostEqual(peak, 1.0, places=4),
            ("heldout", "atan-clip"): lambda peak: self.assertLess(peak, 1.0),
        }
        for (pool, variant), check in cases.items():
            with self.subTest(pool=pool, variant=variant):
                profile = "clip-limit"
                seed = find_seed(pool, profile, "dynamics", variant)
                rec, _ = self.run_chain(seed, pool, profile)
                self.assertTrue(self.stage(rec, "dynamics")["variant"].endswith(variant))
                check(wavio.peak(wavio.read_wav(self.step("dynamics"))[1]))

    def test_a_room_is_the_synthetic_impulse_response_convolved_with_the_input(self) -> None:
        impulse = self.dir / "impulse.wav"
        support.write_impulse(impulse, at=4000, amp=0.5)
        for variant in ("hall", "flutter"):
            with self.subTest(variant=variant):
                seed = find_seed("heldout", "reverb", "room", variant)
                rec, _ = self.run_chain(seed, "heldout", "reverb", src=impulse)
                room = self.stage(rec, "room")
                p = room["params"]
                h, meta = ir.synthesize(48000, 1, p, p["seed"])
                self.assertEqual(meta["sha256"], room["result"]["ir"]["sha256"])
                y = wavio.read_wav(self.step("room"))[1]
                n = min(len(h), len(y) - 4000)
                amp = int(0.5 * 32767) / 32768  # the impulse as stored in the 16-bit input
                err = max(abs(y[4000 + i] - amp * h[i]) for i in range(n))
                self.assertLess(err, 1e-6)
                self.assertEqual(len(y), 24000 + room["result"]["extend_samples"], "the room rings on past the clip")

    def test_mono_applies_the_recorded_weights(self) -> None:
        for pool in ("train", "heldout"):
            rec, work = self.run_chain(3, pool, "mono")
            w = self.stage(rec, "mono")["result"]["weights"]
            both = wavio.read_wav(work / "00-ingest.wav")[1]
            mono = wavio.read_wav(self.step("mono"))[1]
            err = max(abs(m - (w[0] * both[2 * i] + w[1] * both[2 * i + 1])) for i, m in enumerate(mono))
            self.assertLess(err, 1e-6)
            self.assertEqual(rec["output"]["channels"], 1)

    def test_band_limit_removes_what_is_above_the_new_nyquist(self) -> None:
        noise_in = self.dir / "noise.wav"
        # White noise has energy everywhere, so what is left above the cut is the resampler's stop band.
        from enth_ml.degrade.rng import Rng

        import array
        import wave

        rng, pcm = Rng(5), array.array("h")
        for _ in range(48000):
            v = int((rng.random() - 0.5) * 20000)
            pcm.extend((v, v))
        with wave.open(str(noise_in), "wb") as w:
            w.setnchannels(2)
            w.setsampwidth(2)
            w.setframerate(48000)
            w.writeframes(pcm.tobytes())
        for pool in ("train", "heldout"):
            rec, _ = self.run_chain(1, pool, "band-bit", src=noise_in)
            ny = self.stage(rec, "band_limit")["result"]["nyquist_hz"]
            info, x = wavio.read_wav(self.step("band-limit"))
            x = x[0::2][12000:12000 + 8192]
            below = band_power(x, 48000, 0.1 * ny, 0.8 * ny)
            above = band_power(x, 48000, 1.2 * ny, min(2.0 * ny, 23000.0))
            self.assertLess(10 * math.log10(above / below), -60.0, f"{pool}: nyquist {ny}")

    def test_bit_depth_leaves_only_the_levels_it_should(self) -> None:
        for pool, variant in (("train", "quantise"), ("heldout", "quantise-low")):
            seed = find_seed(pool, "band-bit", "bit_depth", variant)
            rec, _ = self.run_chain(seed, pool, "band-bit")
            p = self.stage(rec, "bit_depth")["params"]
            q = 2 ** (p["bits"] - 1)
            x = wavio.read_wav(self.step("bit-depth"))[1]
            if p["dither"]:
                # TPDF dither is added before rounding: levels stay on the grid and the error spreads over +-1 LSB.
                pass
            for v in x[::7]:
                self.assertAlmostEqual(v * q, round(v * q), places=3, msg=f"{pool} {p}")
            self.assertLessEqual(len(set(x)), 2 * q)
        for pool, variant in (("train", "mulaw"), ("heldout", "alaw")):
            seed = find_seed(pool, "band-bit", "bit_depth", variant)
            rec, _ = self.run_chain(seed, pool, "band-bit")
            self.assertEqual(self.stage(rec, "bit_depth")["result"]["codec"], "pcm_" + variant)
            self.assertLessEqual(len(set(wavio.read_wav(self.step("decoded"))[1])), 256)

    def test_trim_and_pad_lengths(self) -> None:
        rec, work = self.run_chain(1, "train", "trim")
        n0 = wavio.read_info(work / "00-ingest.wav").frames
        cut = self.stage(rec, "attack_cut")["result"]["cut_samples"]
        kept = self.stage(rec, "tail_trunc")["result"]["kept_samples"]
        pad = self.stage(rec, "lead_pad")["result"]["pad_samples"]
        self.assertEqual(self.stage(rec, "attack_cut")["frames"], n0 - cut)
        self.assertEqual(self.stage(rec, "tail_trunc")["frames"], kept)
        self.assertEqual(self.stage(rec, "lead_pad")["frames"], kept + pad)
        self.assertEqual(rec["output"]["frames"], kept + pad)
        x = wavio.read_wav(self.step("lead-pad"))[1]
        self.assertEqual(max(abs(v) for v in x[: 2 * pad]), 0.0, "the padding is silence")
        tail = wavio.read_wav(self.step("tail-trunc"))[1]
        self.assertLess(abs(tail[-1]), 1e-3, "the truncated tail fades to zero")

    def test_codecs_keep_length_and_stay_in_time(self) -> None:
        burst = self.dir / "burst.wav"
        support.write_burst(burst)
        onset_in = 0.4 * 48000

        def onset(path: pathlib.Path) -> int:
            info, x = wavio.read_wav(path)
            level = 0.2 * wavio.peak(x)
            return next(i for i in range(0, len(x), info.channels) if abs(x[i]) > level) // info.channels

        cases = [("train", "lame-cbr"), ("train", "aac-lc"), ("train", "opus"), ("heldout", "shine-cbr"),
                 ("heldout", "lame-vbr"), ("heldout", "lame-abr"), ("heldout", "aac-fast"), ("heldout", "opus-voip"),
                 ("heldout", "amr-nb"), ("heldout", "gsm")]
        encoder = {"lame-cbr": "libmp3lame", "lame-vbr": "libmp3lame", "lame-abr": "libmp3lame", "aac-lc": "aac",
                   "aac-fast": "aac", "opus": "libopus", "opus-voip": "libopus", "shine-cbr": "libshine",
                   "amr-nb": "libopencore_amrnb", "gsm": "libgsm"}
        for pool, variant in cases:
            if not have_encoder(encoder[variant]):
                continue
            with self.subTest(variant=variant):
                profile = "mp3" if variant in ("lame-cbr", "shine-cbr", "lame-vbr", "lame-abr") else "codec"
                if pool == "heldout" and variant in ("aac-fast", "opus-voip"):
                    profile = "codec-other"
                elif pool == "heldout" and variant in ("amr-nb", "gsm"):
                    profile = "phone-codec"
                seed = find_seed(pool, profile, "codec", variant)
                rec, _ = self.run_chain(seed, pool, profile, src=burst)
                codec = self.stage(rec, "codec")
                self.assertEqual(codec["frames"], 48000, "the round trip keeps the clip's length")
                self.assertEqual(rec["output"]["channels"], 2, "phone codecs come back to stereo")
                self.assertEqual(rec["output"]["sample_rate"], 48000)
                shift_ms = (onset(self.step("decoded")) - onset_in) / 48.0
                # ffmpeg trims the gapless delay of LAME, AAC and Opus; shine's own delay is not trimmed (up to about 10 ms
                # at 32 kbps) and AMR and GSM look ahead.
                limit = {"amr-nb": 20.0, "gsm": 20.0, "shine-cbr": 12.0}.get(variant, 5.0)
                self.assertLess(abs(shift_ms), limit, f"{variant}: onset shifted by {shift_ms:.1f} ms")
                self.assertGreater(codec["result"]["coded_bytes"], 100)
                for f in self.work.iterdir():
                    f.unlink()

    def test_the_held_out_mp3_encoder_is_not_lame(self) -> None:
        if not have_encoder("libshine"):
            self.skipTest("this ffmpeg has no libshine")
        seed = find_seed("heldout", "mp3", "codec", "shine-cbr")
        rec, _ = self.run_chain(seed, "heldout", "mp3")
        first = self.stage(rec, "codec")["calls"][0]
        self.assertIn("libshine", first)
        self.assertNotIn("libmp3lame", first)


if __name__ == "__main__":
    unittest.main()
