"""Lane DEG: the chain end to end. Acceptance: the same seed gives the same bytes (SHA-256 compared).

Standard library plus the system ffmpeg; every test makes its own short synthetic input. Skipped when ffmpeg is missing.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

import degrade_support as support
from degrade_support import HAVE_FFMPEG, ML
from enth_ml.degrade import CHAIN_VERSION, RECORD_SCHEMA, degrade_file, load_pools, make_plan, wavio
from enth_ml.degrade.pools import STAGE_ORDER


def sha(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not installed")
class Determinism(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self._td.name)
        self.src = self.dir / "in.wav"
        support.write_pluck(self.src, dur=0.5)

    def tearDown(self) -> None:
        self._td.cleanup()

    def run_chain(self, name: str, **kw) -> tuple[pathlib.Path, dict]:
        out = self.dir / f"{name}.wav"
        rec = degrade_file(self.src, out, **kw)
        return out, rec

    def test_the_same_seed_gives_the_same_bytes_for_a_chain_of_every_stage(self) -> None:
        for pool in ("train", "heldout"):
            with self.subTest(pool=pool):
                a, ra = self.run_chain(f"a-{pool}", seed=11, pool=pool, profile="all")
                b, rb = self.run_chain(f"b-{pool}", seed=11, pool=pool, profile="all")
                self.assertEqual(sha(a), sha(b))
                self.assertEqual(ra, rb)
                self.assertTrue(all(s.get("applied") for s in ra["stages"] if s.get("active")), "every stage ran")

    def test_the_same_seed_gives_the_same_bytes_for_a_mixed_chain(self) -> None:
        for seed in range(4):
            for pool in ("train", "heldout"):
                a, _ = self.run_chain("a", seed=seed, pool=pool)
                b, _ = self.run_chain("b", seed=seed, pool=pool)
                self.assertEqual(sha(a), sha(b), f"{pool} seed {seed}")

    def test_the_bytes_do_not_depend_on_where_or_under_what_name_it_runs(self) -> None:
        a, _ = self.run_chain("a", seed=3, pool="heldout", profile="mp3")
        sub = self.dir / "sub"
        sub.mkdir()
        copy = sub / "other-name.wav"
        copy.write_bytes(self.src.read_bytes())
        b = sub / "out.wav"
        degrade_file(copy, b, seed=3, pool="heldout", profile="mp3", work_dir=sub / "work")
        self.assertEqual(sha(a), sha(b))

    def test_other_seeds_and_pools_change_the_bytes(self) -> None:
        seen = {
            sha(self.run_chain(f"{pool}{seed}", seed=seed, pool=pool, profile="all")[0])
            for pool in ("train", "heldout")
            for seed in (1, 2)
        }
        self.assertEqual(len(seen), 4)

    def test_the_record_says_exactly_what_was_applied(self) -> None:
        out, rec = self.run_chain("r", seed=5, pool="heldout", profile="all")
        first_line = subprocess.run(["ffmpeg", "-version"], capture_output=True, text=True, check=True).stdout.splitlines()[0]
        self.assertEqual(rec["schema"], RECORD_SCHEMA)
        self.assertEqual(rec["chain_version"], CHAIN_VERSION)
        self.assertEqual(rec["ffmpeg"], first_line.strip())
        self.assertEqual(rec["pools"]["sha256"], load_pools().sha256)
        self.assertEqual((rec["pool"], rec["profile"], rec["seed"]), ("heldout", "all", 5))
        self.assertEqual(rec["input"]["sha256"], sha(self.src))
        self.assertEqual(rec["output"]["sha256"], sha(out))
        self.assertEqual([s["stage"] for s in rec["stages"]], ["ingest", *STAGE_ORDER, "normalise"])
        for s in rec["stages"]:
            if s.get("active"):
                self.assertTrue(s["calls"], s["stage"])  # the ffmpeg arguments of every call
                self.assertTrue(all(isinstance(a, str) for c in s["calls"] for a in c))
                self.assertRegex(s["sha256"] if s["stage"] != "normalise" else "0" * 64, r"^[0-9a-f]{64}$")
        text = json.dumps(rec)
        self.assertNotIn(str(self.dir), text, "the record names no local path")
        self.assertNotIn("@", text)
        self.assertEqual(json.loads(text), rec)

    def test_a_clean_chain_only_normalises(self) -> None:
        pools = load_pools()
        clean_seed = next(s for s in range(500) if make_plan(pools, "train", s)["clean"])
        out, rec = self.run_chain("clean", seed=clean_seed, pool="train")
        self.assertTrue(rec["clean"])
        self.assertFalse(any(s.get("active") for s in rec["stages"] if s["stage"] in STAGE_ORDER))
        self.assertAlmostEqual(support.lufs(out), -23.0, delta=0.15)

    def test_every_output_is_at_minus_23_lufs(self) -> None:
        for pool, profile, seed in (
            ("train", None, 1),
            ("train", "codec", 2),
            ("heldout", "clip-limit", 3),
            ("heldout", "noise-hum", 4),
            ("heldout", "reverb", 5),
            ("heldout", "all", 6),
            ("heldout", "phone-recording", 7),
            ("train", "upload-room", 8),
        ):
            out, rec = self.run_chain("n", seed=seed, pool=pool, profile=profile)
            norm = rec["stages"][-1]["result"]
            self.assertNotIn("peak_limited", norm)
            self.assertAlmostEqual(support.lufs(out), -23.0, delta=0.15, msg=f"{pool}/{profile}/{seed}")

    def test_a_short_sound_survives_every_profile_of_both_pools(self) -> None:
        # Short sounds are valid: nothing is filtered by duration. 60 ms is shorter than a BS.1770 gating block.
        short = self.dir / "short.wav"
        support.write_pluck(short, dur=0.06)
        ps = load_pools()
        for pool_name in ("train", "heldout"):
            for profile in [None, "all", *sorted(ps.pool(pool_name).profiles)]:
                out = self.dir / "short-out.wav"
                rec = degrade_file(short, out, seed=2, pool=pool_name, profile=profile)
                info, x = wavio.read_wav(out)
                self.assertGreater(info.frames, 0, f"{pool_name}/{profile}")
                self.assertGreater(wavio.rms(x), 0.0, f"{pool_name}/{profile}")
                self.assertNotIn("skipped", rec["stages"][-1]["result"], "the short clip was measured, not skipped")

    def test_a_mono_input_skips_the_mono_stage_and_stays_mono(self) -> None:
        mono = self.dir / "mono.wav"
        support.write_pluck(mono, channels=1, dur=0.4)
        rec = degrade_file(mono, self.dir / "o.wav", seed=1, pool="train", profile="all")
        mono_stage = next(s for s in rec["stages"] if s["stage"] == "mono")
        self.assertFalse(mono_stage["applied"])
        self.assertIn("1 channel", mono_stage["skipped"])
        self.assertEqual(rec["output"]["channels"], 1)

    def test_other_sample_rates_come_back_at_their_own_rate(self) -> None:
        for sr in (44100, 22050, 16000):
            src = self.dir / f"in{sr}.wav"
            support.write_pluck(src, sr=sr, dur=0.4)
            for pool in ("train", "heldout"):
                rec = degrade_file(src, self.dir / "o.wav", seed=4, pool=pool, profile="all")
                self.assertEqual(rec["output"]["sample_rate"], sr, f"{pool} {sr}")
        # A band limit at or above the input's rate does nothing, and says so.
        rec = degrade_file(self.dir / "in16000.wav", self.dir / "o.wav", seed=1, pool="train", profile="band-bit")
        band = next(s for s in rec["stages"] if s["stage"] == "band_limit")
        if band["params"]["sr_hz"] >= 16000:
            self.assertFalse(band["applied"])

    def test_sample_formats(self) -> None:
        for fmt, bits, tag in (("s16", 16, 1), ("s24", 24, 1), ("f32", 32, 3)):
            out = self.dir / f"{fmt}.wav"
            rec = degrade_file(self.src, out, seed=1, pool="train", profile="eq", sample_fmt=fmt)
            data = out.read_bytes()
            wave_tag = int.from_bytes(data[20:22], "little")
            if wave_tag == 0xFFFE:  # WAVE_FORMAT_EXTENSIBLE (ffmpeg writes it above 16 bits): the real tag is the GUID's
                wave_tag = int.from_bytes(data[44:46], "little")
            self.assertEqual(wave_tag, tag, fmt)
            self.assertEqual(int.from_bytes(data[34:36], "little"), bits, fmt)
            self.assertEqual(rec["output"]["sample_fmt"], fmt)
        with self.assertRaises(ValueError):
            degrade_file(self.src, self.dir / "x.wav", seed=1, sample_fmt="s8")

    def test_the_output_header_carries_no_ffmpeg_version(self) -> None:
        out, _ = self.run_chain("h", seed=1, pool="train", profile="mp3")
        self.assertNotIn(b"Lavf", out.read_bytes())
        self.assertNotIn(b"LIST", out.read_bytes()[:64])


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not installed")
class Cli(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self._td.name)
        self.src = self.dir / "x.wav"
        support.write_pluck(self.src, dur=0.4)

    def tearDown(self) -> None:
        self._td.cleanup()

    def cli(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-m", "enth_ml.degrade", *args], cwd=ML, capture_output=True, text=True, check=False
        )

    def test_the_documented_command_works_and_is_deterministic(self) -> None:
        shas = []
        for i in range(2):
            out, rec = self.dir / f"y{i}.wav", self.dir / f"y{i}.json"
            done = self.cli("--in", str(self.src), "--out", str(out), "--seed", "7", "--pool", "heldout", "--record", str(rec))
            self.assertEqual(done.returncode, 0, done.stderr)
            record = json.loads(rec.read_text(encoding="utf-8"))
            self.assertEqual(record["pool"], "heldout")
            self.assertEqual(record["output"]["sha256"], sha(out))
            shas.append(sha(out))
        self.assertEqual(shas[0], shas[1])

    def test_without_record_the_json_goes_to_stdout(self) -> None:
        done = self.cli("--in", str(self.src), "--out", str(self.dir / "y.wav"), "--seed", "1", "--profile", "eq")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(json.loads(done.stdout)["profile"], "eq")

    def test_list_and_plan_need_no_audio(self) -> None:
        listing = json.loads(self.cli("--list").stdout)
        self.assertIn("shine-cbr", listing["pools"]["heldout"]["stages"]["codec"]["variants"])
        plan = json.loads(self.cli("--plan", "--pool", "heldout", "--profile", "mp3", "--seed", "3").stdout)
        self.assertEqual([e["stage"] for e in plan["stages"] if e["active"]], ["codec"])

    def test_errors_are_reported_not_traced(self) -> None:
        done = self.cli("--in", str(self.dir / "missing.wav"), "--out", str(self.dir / "y.wav"))
        self.assertEqual(done.returncode, 1)
        self.assertIn("error", done.stderr)
        self.assertNotIn("Traceback", done.stderr)
        done = self.cli("--in", str(self.src), "--out", str(self.dir / "y.wav"), "--profile", "nope")
        self.assertEqual(done.returncode, 1)
        self.assertIn("nope", done.stderr)


if __name__ == "__main__":
    unittest.main()
