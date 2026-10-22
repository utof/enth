"""Lane AUG: each stage of this package does what it says, measured on the files it writes.

Standard library plus the system ffmpeg; synthetic recordings (no data files). Skipped when ffmpeg is missing. Every test
runs the chain with one stage forced on and reads the intermediate float files from the work directory.
"""

from __future__ import annotations

import array
import math
import pathlib
import subprocess
import unittest

from augment_support import (
    HAVE_FFMPEG,
    config_text,
    goertzel_db,
    make_store,
    make_world,
    read_mono,
    temp_dir,
    write_noise,
    write_pluck,
    write_rir,
    write_tone,
)
from enth_ml.augment import manifest as mf
from enth_ml.augment.chain import augment_file
from enth_ml.augment.config import parse_config
from enth_ml.degrade import wavio

MANIFEST = mf.load_manifest()

STAGES = {
    "rir": '[stage.rir]\np = 1.0\n[stage.rir.variant.v]\nkind = "recorded"\nsource = "{ref}"\nwet_db = [{wet}, {wet}]\n',
    "bg_noise": '[stage.bg_noise]\np = 1.0\n[stage.bg_noise.variant.v]\nkind = "recorded"\nsource = "{ref}"\nsnr_db = [{snr}, {snr}]\n',
    "mix_in": (
        '[stage.mix_in]\np = 1.0\n[stage.mix_in.variant.v]\nkind = "{kind}"\n{src}rel_db = [{rel}, {rel}]\n'
        "offset_ms = [{off}, {off}]\n"
    ),
    "mic": (
        '[stage.mic]\np = 1.0\n[stage.mic.variant.v]\nkind = "mic-test"\nhp_hz = {hp}\nprox_hz = 150.0\nprox_db = 0.0\n'
        "peak_hz = 3000.0\npeak_gain_db = 0.0\npeak_q = 1.0\nair_hz = 9000.0\nair_db = 0.0\nlp_hz = {lp}\n"
    ),
    "gain": '[stage.gain]\np = 1.0\n[stage.gain.variant.v]\nkind = "level"\npeak_dbfs = [{peak}, {peak}]\n',
    "rate": '[stage.rate]\np = 1.0\n[stage.rate.variant.v]\nkind = "native-rate"\nsr_hz = {sr}\n',
}


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not installed")
class StageCase(unittest.TestCase):
    def setUp(self) -> None:
        self._td = temp_dir()
        self.dir = pathlib.Path(self._td.name)
        self.world = make_world(self.dir / "world")
        self.n = 0

    def tearDown(self) -> None:
        self._td.cleanup()

    def run_stage(self, stage: str, src: pathlib.Path, text: str, *, dirs=None, seed: int = 1, layers=None, deg=None):
        """Run the chain with ``stage`` forced on; returns (record, work dir, stage entry of the record)."""
        self.n += 1
        extra = f'deg = "{deg}"\n' if deg else ""
        cfg = parse_config(config_text(stages=text + f'[profile.p]\nstages = ["{stage}"]\n{extra}'))
        store = make_store(self.dir, dirs if dirs is not None else self.world)
        work = self.dir / f"work{self.n}"
        rec = augment_file(
            src, self.dir / f"out{self.n}.wav", seed=seed, config=cfg, manifest=MANIFEST, store=store, profile="p",
            layers=layers, work_dir=work,
        )
        entry = next(s for s in rec["stages"] if s["stage"] == stage)
        return rec, work, entry

    def files(self, work: pathlib.Path, suffix: str) -> list[pathlib.Path]:
        return sorted(work.glob(f"*-{suffix}.wav"))

    def mono(self, name: str, **kw) -> pathlib.Path:
        path = self.dir / name
        write_pluck(path, channels=1, **kw)
        return path


class Rir(StageCase):
    def test_the_output_is_the_convolution_of_the_clip_with_the_kernel_it_names(self) -> None:
        # An independent, plain Python convolution of the ingested clip with the kernel file: the kernel's direct
        # impulse sits at 0 (no latency, no filtering of the clip) and the tail follows at its delay.
        one = self.dir / "one"
        one.mkdir()
        write_rir(one / "r.wav", sr=8000, rt60=0.2, seed=3)
        src = self.mono("clip.wav", sr=8000, dur=0.25)
        rec, work, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="openslr-28/sim-small", wet=-6.0), dirs={"openslr-28/sim-small": one})
        self.assertTrue(entry["applied"], entry)
        _, x = wavio.read_wav(work / "00-ingest.wav")
        kernel_path, result_path = self.files(work, "rir")
        _, h = wavio.read_wav(kernel_path)
        _, y = wavio.read_wav(result_path)
        self.assertEqual(len(y), len(x) + entry["result"]["extend_samples"])
        ref = [0.0] * len(y)
        for k, xv in enumerate(x):
            if xv == 0.0:
                continue
            for j, hv in enumerate(h):
                if k + j < len(ref):
                    ref[k + j] += xv * hv
        self.assertLess(max(abs(a - b) for a, b in zip(ref, y, strict=True)), 2e-5)
        self.assertEqual(h[0], 1.0)

    def test_the_kernel_s_wet_part_is_wet_db_below_the_direct(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.3)
        for wet in (-20.0, -3.0):
            with self.subTest(wet=wet):
                _, work, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="openslr-28/sim-medium", wet=wet))
                _, h = wavio.read_wav(self.files(work, "rir")[0])
                self.assertAlmostEqual(10 * math.log10(math.fsum(v * v for v in h[1:])), wet, delta=0.02)
                self.assertAlmostEqual(entry["result"]["kernel"]["wet_db_actual"], wet, delta=0.02)

    def test_a_recording_at_another_rate_leaves_the_direct_part_unfiltered(self) -> None:
        # 15 kHz is above a 16 kHz recording's Nyquist: were the direct part resampled with the recording it would vanish.
        src = self.dir / "hf.wav"
        write_tone(src, (1000.0, 15000.0), sr=48000, dur=0.4)
        rec, work, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="openslr-28/sim-small", wet=-40.0))
        self.assertTrue(entry["result"]["recording"]["resampled"])
        self.assertEqual(entry["result"]["recording"]["sample_rate_hz"], 16000)
        sr, before = read_mono(work / "00-ingest.wav")
        _, after = read_mono(self.files(work, "rir")[1])
        for f in (1000.0, 15000.0):
            self.assertAlmostEqual(goertzel_db(after[:19000], sr, f), goertzel_db(before[:19000], sr, f), delta=0.2, msg=str(f))

    def test_a_stereo_clip_takes_two_channels_of_a_multichannel_recording(self) -> None:
        src = self.dir / "st.wav"
        write_pluck(src, channels=2, sr=16000, dur=0.3)
        rec, work, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="but-reverbdb/rirs", wet=-8.0))
        self.assertEqual(len(entry["result"]["recording"]["channels_used"]), 2)
        self.assertGreaterEqual(entry["result"]["recording"]["channels"], 2)
        self.assertEqual(entry["channels"], 2)
        self.assertEqual(wavio.read_info(self.dir / "out1.wav").channels, 2)
        self.assertEqual(rec["sources_used"][0]["licence"], "CC-BY-4.0")

    def test_the_record_names_the_recording_its_hash_and_its_measured_decay(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.3)
        rec, work, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="openslr-28/sim-large", wet=-8.0))
        res = entry["result"]
        path = self.world["openslr-28/sim-large"] / res["source"]["relpath"]
        self.assertEqual(res["source"]["sha256"], wavio.sha256_file(path))
        self.assertAlmostEqual(res["rt60_s"], 1.0, delta=0.2)
        self.assertEqual(res["rt60_method"], "T20")
        self.assertEqual(res["wet_db"], -8.0)
        self.assertEqual(
            rec["sources_used"],
            [{"ref": "openslr-28/sim-large", "relpath": res["source"]["relpath"], "sha256": res["source"]["sha256"],
              "licence": "Apache-2.0", "use": "shipped-training"}],
        )
        self.assertEqual(rec["label"]["policy"], "dry-and-wet")
        self.assertEqual(rec["label"]["wet_candidates"][0]["stage"], "rir")
        self.assertAlmostEqual(rec["label"]["wet_candidates"][0]["rt60_s"], res["rt60_s"])

    def test_a_recording_without_a_tail_skips_the_stage_and_the_file_is_still_written(self) -> None:
        bare = self.dir / "bare"
        bare.mkdir()
        spike = array.array("f", [0.0] * 4000)
        spike[50] = 1.0
        wavio.write_wav_f32(bare / "s.wav", 16000, 1, spike)
        src = self.mono("clip.wav", sr=16000, dur=0.3)
        rec, _, entry = self.run_stage("rir", src, STAGES["rir"].format(ref="openslr-28/sim-small", wet=-8.0), dirs={"openslr-28/sim-small": bare})
        self.assertFalse(entry["applied"])
        self.assertIn("no tail", entry["skipped"])
        self.assertTrue((self.dir / "out1.wav").is_file())
        self.assertEqual(rec["label"]["wet_candidates"], [])
        self.assertEqual(rec["label"]["policy"], "damage")


class BackgroundNoise(StageCase):
    def snr_of(self, work: pathlib.Path) -> float:
        """The SNR of the mix, from the clip before and after: the difference is exactly the added noise."""
        _, x = wavio.read_wav(work / "00-ingest.wav")
        mixed = next(f for f in self.files(work, "bg-noise-mix"))
        _, y = wavio.read_wav(mixed)
        diff = array.array("f", (b - a for a, b in zip(x, y, strict=True)))
        return wavio.to_db(wavio.rms(x)) - wavio.to_db(wavio.rms(diff))

    def test_the_noise_lands_at_the_requested_snr_against_the_clip_s_rms(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.6)
        for snr in (10.0, 30.0, 48.0):
            with self.subTest(snr=snr):
                _, work, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=snr))
                self.assertTrue(entry["applied"], entry)
                self.assertAlmostEqual(self.snr_of(work), snr, delta=0.02)

    def test_the_recorded_noise_is_recorded_in_the_record_with_its_licence(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.6)
        rec, _, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=20.0))
        used = rec["sources_used"][0]
        self.assertEqual((used["ref"], used["licence"], used["use"]), ("openslr-28/pointsource-noises", "CC0-1.0", "shipped-training"))
        self.assertEqual(entry["result"]["source"]["sha256"], used["sha256"])
        self.assertTrue(rec["shipped_ok"])
        self.assertEqual(rec["label"]["policy"], "damage")

    def test_a_noise_shorter_than_the_clip_loops_and_a_longer_one_starts_inside_it(self) -> None:
        short = self.dir / "short"
        short.mkdir()
        write_noise(short / "n.wav", sr=16000, dur=0.2)
        src = self.mono("clip.wav", sr=16000, dur=0.7)
        _, work, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=20.0), dirs={"openslr-28/pointsource-noises": short})
        self.assertTrue(entry["result"]["prepared"]["looped"])
        self.assertAlmostEqual(self.snr_of(work), 20.0, delta=0.02)
        long = self.dir / "long"
        long.mkdir()
        write_noise(long / "n.wav", sr=16000, dur=5.0)
        _, work, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=20.0), dirs={"openslr-28/pointsource-noises": long})
        prep = entry["result"]["prepared"]
        self.assertFalse(prep["looped"])
        self.assertTrue(0.0 <= prep["start_s"] <= 5.0 - 0.7 + 1e-6)

    def test_other_formats_rates_and_channel_counts_are_brought_to_the_clip_s(self) -> None:
        coll = self.dir / "mixed"
        coll.mkdir()
        write_noise(coll / "n.wav", sr=22050, dur=1.5)
        subprocess.run(["ffmpeg", "-nostdin", "-loglevel", "error", "-y", "-i", str(coll / "n.wav"), "-ac", "3" if False else "2", str(coll / "n.flac")], check=True)
        (coll / "n.wav").unlink()
        for channels, name in ((1, "mono-clip.wav"), (2, "stereo-clip.wav")):
            with self.subTest(channels=channels):
                src = self.dir / name
                write_pluck(src, channels=channels, sr=16000, dur=0.5)
                _, work, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=25.0), dirs={"openslr-28/pointsource-noises": coll})
                prep = entry["result"]["prepared"]
                self.assertEqual((prep["source_rate_hz"], prep["source_channels"]), (22050, 2))
                self.assertEqual(entry["channels"], channels)
                self.assertEqual(entry["frames"], 8000)
                self.assertAlmostEqual(self.snr_of(work), 25.0, delta=0.05)

    def test_a_multichannel_noise_gives_one_channel(self) -> None:
        coll = self.dir / "multi"
        coll.mkdir()
        write_rir(coll / "n.wav", sr=16000, dur=1.0, channels=3, rt60=5.0)  # any 3-channel noise-like file will do
        src = self.mono("clip.wav", sr=16000, dur=0.4)
        _, work, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=25.0), dirs={"openslr-28/pointsource-noises": coll})
        self.assertEqual(entry["result"]["prepared"]["source_channels"], 3)
        self.assertAlmostEqual(self.snr_of(work), 25.0, delta=0.05)

    def test_a_silent_excerpt_skips_the_stage(self) -> None:
        coll = self.dir / "silent"
        coll.mkdir()
        wavio.write_wav_f32(coll / "n.wav", 16000, 1, array.array("f", [0.0] * 32000))
        src = self.mono("clip.wav", sr=16000, dur=0.4)
        _, _, entry = self.run_stage("bg_noise", src, STAGES["bg_noise"].format(ref="openslr-28/pointsource-noises", snr=25.0), dirs={"openslr-28/pointsource-noises": coll})
        self.assertFalse(entry["applied"])
        self.assertIn("silent", entry["skipped"])


class MixIn(StageCase):
    def level_ratio_db(self, work: pathlib.Path) -> float:
        _, x = wavio.read_wav(work / "00-ingest.wav")
        _, y = wavio.read_wav(self.files(work, "layer-mix")[0])
        diff = array.array("f", (b - a for a, b in zip(x, y, strict=True)))
        return wavio.to_db(wavio.rms(diff)) - wavio.to_db(wavio.rms(x))

    def test_a_supplied_render_goes_under_the_clip_at_the_stated_level(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.5, f0=220.0)
        other = self.mono("other.wav", sr=16000, dur=0.5, f0=330.0)
        text = STAGES["mix_in"].format(kind="layer", src="", rel=-12.0, off=0.0)
        rec, work, entry = self.run_stage("mix_in", src, text, layers=[other])
        self.assertTrue(entry["applied"], entry)
        self.assertAlmostEqual(self.level_ratio_db(work), -12.0, delta=0.02)
        self.assertEqual(entry["result"]["layer"]["sha256"], wavio.sha256_file(other))
        self.assertEqual(rec["layers_supplied"], 1)
        self.assertEqual(rec["sources_used"], [])
        self.assertTrue(rec["shipped_ok"])

    def test_the_offset_delays_the_layer_and_the_length_is_the_clip_s(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.5)
        other = self.mono("other.wav", sr=16000, dur=0.5, f0=330.0)
        text = STAGES["mix_in"].format(kind="layer", src="", rel=-6.0, off=200.0)
        _, work, entry = self.run_stage("mix_in", src, text, layers=[other])
        self.assertEqual(entry["result"]["offset_samples"], 3200)
        self.assertEqual(entry["frames"], 8000)
        _, x = wavio.read_wav(work / "00-ingest.wav")
        _, y = wavio.read_wav(self.files(work, "layer-mix")[0])
        self.assertEqual(list(x[:3000]), list(y[:3000]), "nothing is added before the offset")
        self.assertAlmostEqual(self.level_ratio_db(work), -6.0, delta=0.02)

    def test_a_recorded_note_is_resampled_to_the_clip_and_licensed(self) -> None:
        src = self.dir / "clip.wav"
        write_pluck(src, channels=2, sr=48000, dur=0.5)
        text = STAGES["mix_in"].format(kind="layer-recorded", src='source = "tinysol"\n', rel=-9.0, off=0.0)
        rec, work, entry = self.run_stage("mix_in", src, text)
        prep = entry["result"]["prepared"]
        self.assertEqual((prep["source_rate_hz"], prep["source_channels"]), (44100, 1))
        self.assertEqual((entry["sample_rate"], entry["channels"], entry["frames"]), (48000, 2, 24000))
        self.assertAlmostEqual(self.level_ratio_db(work), -9.0, delta=0.02)
        self.assertEqual(rec["sources_used"][0]["licence"], "CC-BY-4.0")
        self.assertTrue(rec["shipped_ok"])

    def test_the_layer_is_chosen_by_the_plan_among_those_supplied(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.4)
        layers = [self.mono(f"l{i}.wav", sr=16000, dur=0.4, f0=200.0 + 50 * i) for i in range(4)]
        text = STAGES["mix_in"].format(kind="layer", src="", rel=-9.0, off=0.0)
        picked = {self.run_stage("mix_in", src, text, layers=layers, seed=s)[2]["result"]["layer"]["index"] for s in range(12)}
        self.assertGreater(len(picked), 1)
        self.assertTrue(picked <= {0, 1, 2, 3})

    def test_the_noise_after_a_layer_is_measured_against_the_clip_with_its_layer(self) -> None:
        # DEG's noise stage runs later in the same chain; its SNR reference must be the level after the layer.
        src = self.mono("clip.wav", sr=16000, dur=0.5)
        other = self.mono("other.wav", sr=16000, dur=0.5, f0=330.0)
        text = STAGES["mix_in"].format(kind="layer", src="", rel=-3.0, off=0.0)
        rec, work, _ = self.run_stage("mix_in", src, text, layers=[other], deg="noise-hum")
        noise = next(s for s in rec["stages"] if s["stage"] == "noise")
        self.assertTrue(noise["applied"], noise)
        _, mixed = wavio.read_wav(self.files(work, "layer-mix")[0])
        _, clip = wavio.read_wav(work / "00-ingest.wav")
        self.assertAlmostEqual(noise["result"]["signal_rms_dbfs"], wavio.to_db(wavio.rms(mixed)), delta=0.002)
        self.assertGreater(wavio.to_db(wavio.rms(mixed)) - wavio.to_db(wavio.rms(clip)), 1.0)


class Mic(StageCase):
    def test_a_high_pass_takes_the_low_tone_down_and_leaves_the_mid_tone(self) -> None:
        src = self.dir / "t.wav"
        write_tone(src, (60.0, 1000.0), sr=16000, dur=0.8)
        _, work, entry = self.run_stage("mic", src, STAGES["mic"].format(hp=200.0, lp=7000.0))
        self.assertEqual(entry["result"]["components"], ["hp", "lp"], "the zero-gain components are left out")
        sr, before = read_mono(work / "00-ingest.wav")
        _, after = read_mono(self.files(work, "mic")[0])
        low = goertzel_db(after[2000:], sr, 60.0) - goertzel_db(before[2000:], sr, 60.0)
        mid = goertzel_db(after[2000:], sr, 1000.0) - goertzel_db(before[2000:], sr, 1000.0)
        self.assertLess(low, -15.0)
        self.assertAlmostEqual(mid, 0.0, delta=0.5)

    def test_a_low_pass_takes_the_high_tone_down(self) -> None:
        src = self.dir / "t.wav"
        write_tone(src, (1000.0, 7000.0), sr=16000, dur=0.8)
        _, work, _ = self.run_stage("mic", src, STAGES["mic"].format(hp=30.0, lp=3000.0))
        sr, before = read_mono(work / "00-ingest.wav")
        _, after = read_mono(self.files(work, "mic")[0])
        self.assertLess(goertzel_db(after[2000:], sr, 7000.0) - goertzel_db(before[2000:], sr, 7000.0), -10.0)

    def test_components_near_nyquist_are_left_out_and_a_clip_with_none_left_skips(self) -> None:
        src = self.dir / "t.wav"
        write_tone(src, (500.0,), sr=8000, dur=0.5)
        _, _, entry = self.run_stage("mic", src, STAGES["mic"].format(hp=100.0, lp=14000.0))
        self.assertEqual(entry["result"]["components"], ["hp"])  # the 14 kHz low-pass is above 0.45 of 8 kHz
        _, _, entry = self.run_stage("mic", src, STAGES["mic"].format(hp=3900.0, lp=14000.0))
        self.assertFalse(entry["applied"])
        self.assertIn("sample rate", entry["skipped"])


class Gain(StageCase):
    def test_the_peak_lands_where_it_is_asked(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.5)
        for peak in (-45.0, -20.0, -3.0):
            with self.subTest(peak=peak):
                _, work, entry = self.run_stage("gain", src, STAGES["gain"].format(peak=peak))
                _, y = wavio.read_wav(self.files(work, "gain")[0])
                self.assertAlmostEqual(wavio.to_db(wavio.peak(y)), peak, delta=0.01)
                self.assertTrue(entry["applied"])

    def test_a_silent_clip_skips_it(self) -> None:
        src = self.dir / "silent.wav"
        wavio.write_wav_f32(src, 16000, 1, array.array("f", [0.0] * 8000))
        _, _, entry = self.run_stage("gain", src, STAGES["gain"].format(peak=-20.0))
        self.assertFalse(entry["applied"])


class Rate(StageCase):
    def test_the_file_is_resampled_with_the_pitch_and_the_duration_kept(self) -> None:
        src = self.dir / "t.wav"
        write_tone(src, (440.0,), sr=48000, dur=0.5)
        rec, work, entry = self.run_stage("rate", src, STAGES["rate"].format(sr=16000))
        self.assertEqual(entry["result"], {"from_hz": 48000, "to_hz": 16000})
        out = wavio.read_info(self.dir / "out1.wav")
        self.assertEqual((out.sample_rate, out.frames), (16000, 8000))
        self.assertEqual(rec["output"]["sample_rate"], 16000)
        sr, y = read_mono(self.dir / "out1.wav")
        self.assertGreater(goertzel_db(y, sr, 440.0), goertzel_db(y, sr, 520.0) + 30.0)

    def test_a_rate_the_clip_already_has_does_nothing(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.3)
        _, _, entry = self.run_stage("rate", src, STAGES["rate"].format(sr=16000))
        self.assertFalse(entry["applied"])

    def test_upsampling_works_and_the_loudness_is_still_normalised(self) -> None:
        src = self.mono("clip.wav", sr=16000, dur=0.6)
        rec, _, entry = self.run_stage("rate", src, STAGES["rate"].format(sr=44100))
        self.assertEqual(wavio.read_info(self.dir / "out1.wav").sample_rate, 44100)
        norm = rec["stages"][-1]["result"]
        self.assertIsNotNone(norm["measured_lufs"])


if __name__ == "__main__":
    unittest.main()
