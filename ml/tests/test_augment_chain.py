"""Lane AUG: the merged chain end to end. Acceptance: the same seed gives the same bytes (SHA-256 compared).

Standard library plus the system ffmpeg; synthetic recordings (no data files). Skipped when ffmpeg is missing.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import sys
import unittest

from augment_support import HAVE_FFMPEG, ML, config_text, make_store, make_world, recorded_stage, temp_dir, write_pluck
from enth_ml.augment import manifest as mf
from enth_ml.augment.chain import CHAIN_VERSION, RECORD_SCHEMA, augment_file
from enth_ml.augment.config import CONFIG_PATH, MERGED_ORDER, ConfigError, load_config, parse_config
from enth_ml.augment.plan import Availability, make_plan
from enth_ml.augment.sources import SourceError, load_exclusions, partition_of
from enth_ml.degrade import ffmpeg, load_pools, wavio

CFG = load_config(CONFIG_PATH)
EVAL = load_config(CONFIG_PATH.with_name("augment-eval.toml"))
MANIFEST = mf.load_manifest()
POOLS = load_pools()
NOTHING = Availability(lambda ref: False, layers=False)


def sha(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def find_seed(cfg, pred, availability=None, limit: int = 3000) -> int:
    """The first seed whose random plan satisfies ``pred`` (plans need no audio, so this is cheap)."""
    for seed in range(limit):
        if pred(make_plan(cfg, POOLS, seed, availability=availability)):
            return seed
    raise AssertionError("no seed found")


@unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not installed")
class Case(unittest.TestCase):
    def setUp(self) -> None:
        self._td = temp_dir()
        self.dir = pathlib.Path(self._td.name)
        self.world = make_world(self.dir / "world")
        self.store = make_store(self.dir, self.world)
        self.src = self.dir / "in.wav"
        write_pluck(self.src, channels=2, sr=48000, dur=0.6)
        self.layers = [self.dir / "layer.wav"]
        write_pluck(self.layers[0], channels=2, sr=48000, dur=0.5, f0=330.0)

    def tearDown(self) -> None:
        self._td.cleanup()

    def run_chain(self, name: str, **kw) -> tuple[pathlib.Path, dict]:
        out = self.dir / f"{name}.wav"
        kw.setdefault("store", self.store)
        kw.setdefault("config", CFG)
        kw.setdefault("manifest", MANIFEST)
        rec = augment_file(self.src, out, **kw)
        return out, rec


class Determinism(Case):
    def test_the_same_seed_gives_the_same_bytes_for_a_chain_of_every_stage(self) -> None:
        a, ra = self.run_chain("a", seed=11, profile="all", layers=self.layers)
        b, rb = self.run_chain("b", seed=11, profile="all", layers=self.layers)
        self.assertEqual(sha(a), sha(b))
        self.assertEqual(ra, rb)
        applied = {s["stage"] for s in ra["stages"] if s.get("applied")}
        self.assertEqual(applied - {"ingest", "normalise"}, set(MERGED_ORDER))
        self.assertEqual(ra["shipped_ok"], True)
        self.assertEqual(ra["label"]["policy"], "dry-and-wet")
        stages = {w["stage"] for w in ra["label"]["wet_candidates"]}
        self.assertTrue({"room", "rir"} <= stages <= {"room", "rir", "dynamics"}, stages)

    def test_the_same_seed_gives_the_same_bytes_for_random_chains_and_other_seeds_differ(self) -> None:
        shas = set()
        for seed in range(5):
            a, _ = self.run_chain("a", seed=seed, layers=self.layers)
            b, _ = self.run_chain("b", seed=seed, layers=self.layers)
            self.assertEqual(sha(a), sha(b), f"seed {seed}")
            shas.add(sha(a))
        self.assertGreaterEqual(len(shas), 4)

    def test_the_bytes_do_not_depend_on_where_or_under_what_name_it_runs(self) -> None:
        a, ra = self.run_chain("a", seed=3, profile="recorded-room-mp3")
        sub = self.dir / "sub" / "deeper"
        out = sub / "other-name.wav"
        rb = augment_file(
            self.src, out, seed=3, config=CFG, manifest=MANIFEST, store=self.store, profile="recorded-room-mp3",
            work_dir=self.dir / "keep",
        )
        self.assertEqual(sha(a), sha(out))
        self.assertEqual(ra, rb)
        self.assertTrue((self.dir / "keep" / "00-ingest.wav").is_file(), "work_dir keeps the intermediate files")

    def test_the_cli_gives_the_same_bytes_as_the_library_and_prints_a_plan(self) -> None:
        empty = self.dir / "no-data"
        empty.mkdir()
        out1, out2 = self.dir / "cli1.wav", self.dir / "cli2.wav"
        recs = []
        for out in (out1, out2):
            done = self.cli("--in", str(self.src), "--out", str(out), "--seed", "21", "--profile", "mic",
                            "--data-dir", str(empty), "--record", str(out.with_suffix(".json")))
            self.assertEqual(done.returncode, 0, done.stderr)
            recs.append(json.loads(out.with_suffix(".json").read_text()))
        self.assertEqual(sha(out1), sha(out2))
        lib, rec = self.run_chain("lib", seed=21, profile="mic", store=make_store(self.dir, {}, exclusions=load_exclusions()))
        self.assertEqual(sha(out1), sha(lib))
        self.assertEqual(recs[0], rec)
        done = self.cli("--plan", "--seed", "21", "--profile", "recorded-room", "--data-dir", str(empty))
        self.assertNotEqual(done.returncode, 0, "a profile that cannot run here is refused")
        self.assertIn("none of its variants can run", done.stderr)
        done = self.cli("--plan", "--seed", "5", "--data-dir", str(empty))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(json.loads(done.stdout), make_plan(CFG, POOLS, 5, availability=NOTHING))

    def cli(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-m", "enth_ml.augment", *args], cwd=ML, capture_output=True, text=True, check=False)

    def test_cli_actions_without_audio(self) -> None:
        empty = self.dir / "no-data"
        empty.mkdir()
        done = self.cli("--check")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertTrue(done.stdout.startswith("ok"))
        self.assertEqual(self.cli("--sample-seed", "1", "a", "0").stdout.strip(), "6843454643477805539")
        listing = json.loads(self.cli("--list", "--data-dir", str(empty)).stdout)
        self.assertEqual(listing["config"]["name"], "train-v1")
        self.assertFalse(any(s["on_disk"] for s in listing["sources"].values()))
        table = json.loads(self.cli("--manifest", "--data-dir", str(empty)).stdout)
        self.assertEqual({r["ref"] for r in table["sources"]}, set(MANIFEST.all_refs()))
        bad = self.cli("--in", str(self.src), "--out", str(self.dir / "x.wav"), "--seed", "770000000007", "--data-dir", str(empty))
        self.assertEqual(bad.returncode, 1)
        self.assertIn("GR3", bad.stderr)
        self.assertFalse((self.dir / "x.wav").exists())


class Record(Case):
    def test_the_record_pins_everything_the_run_depended_on(self) -> None:
        out, rec = self.run_chain("a", seed=11, profile="all", layers=self.layers)
        json.dumps(rec)
        self.assertEqual((rec["schema"], rec["chain_version"]), (RECORD_SCHEMA, CHAIN_VERSION))
        self.assertEqual(rec["config"], {"name": "train-v1", "version": CFG.version, "file": "augment.toml", "sha256": CFG.sha256})
        self.assertEqual(rec["pools"]["sha256"], POOLS.sha256)
        self.assertEqual(rec["manifest"], {"version": MANIFEST.version, "sha256": MANIFEST.sha256})
        self.assertRegex(rec["exclusions_sha256"] or "x", r"^[0-9a-f]{64}$|^x$")
        self.assertEqual(rec["ffmpeg"], ffmpeg.version_line())
        self.assertEqual(rec["input"]["sha256"], sha(self.src))
        self.assertEqual(rec["output"]["sha256"], sha(out))
        self.assertEqual((rec["purpose"], rec["pool"], rec["seed"], rec["profile"], rec["partition"]), ("shipped-training", "train", 11, "all", "train"))
        self.assertEqual(rec["layers_supplied"], 1)
        self.assertEqual([s["stage"] for s in rec["stages"]][0], "ingest")
        self.assertEqual([s["stage"] for s in rec["stages"]][-1], "normalise")
        self.assertEqual([s["stage"] for s in rec["stages"][1:-1]], list(MERGED_ORDER))

    def test_the_record_holds_no_path_of_this_machine(self) -> None:
        _, rec = self.run_chain("a", seed=11, profile="all", layers=self.layers, work_dir=self.dir / "keep")
        text = json.dumps(rec)
        for needle in (str(self.dir), "/home/", "/tmp/", "vboxuser"):
            self.assertNotIn(needle, text)

    def test_every_recorded_file_used_is_listed_once_with_hash_licence_and_use(self) -> None:
        _, rec = self.run_chain("a", seed=11, profile="all", layers=self.layers)
        refs = {u["ref"] for u in rec["sources_used"]}
        rows = [(u["ref"], u["relpath"]) for u in rec["sources_used"]]
        self.assertEqual(len(set(rows)), len(rows), "no duplicates")
        self.assertIn("openslr-28/pointsource-noises", refs)
        self.assertTrue(any(r.startswith("openslr-28/sim-") or r == "but-reverbdb/rirs" for r in refs), refs)
        for u in rec["sources_used"]:
            path = self.world[u["ref"]] / u["relpath"]
            self.assertEqual(u["sha256"], sha(path))
            self.assertEqual(u["licence"], MANIFEST.licence_of(u["ref"]))
            self.assertEqual(u["use"], "shipped-training")
        self.assertEqual({u["ref"] for u in rec["sources_used"]} - set(self.world), set())

    def test_the_dry_label_and_the_wet_candidates(self) -> None:
        # A DEG room, a recorded room and a compressor each leave a candidate with their measured numbers.
        seed = find_seed(CFG, lambda p: any(e["stage"] == "dynamics" and e["active"] and e["params"]["kind"] == "compressor" for e in p["stages"]),
                         availability=Availability(self.store.available, layers=False))
        _, rec = self.run_chain("comp", seed=seed)
        comp = [w for w in rec["label"]["wet_candidates"] if w["stage"] == "dynamics"]
        self.assertEqual(len(comp), 1, rec["label"])
        self.assertEqual(set(comp[0]) - {"stage", "variant"}, {"ratio", "thr_below_peak_db", "attack_ms", "release_ms", "makeup_db"})
        self.assertEqual(rec["label"]["policy"], "dry-and-wet")
        self.assertIn("clean input render", rec["label"]["dry"])
        _, rec = self.run_chain("room", seed=4, profile="reverb")
        (room,) = rec["label"]["wet_candidates"]
        self.assertEqual((room["stage"], room["recorded"]), ("room", False))
        self.assertTrue(0.1 < room["rt60_s"] < 2.0 and -20.0 <= room["wet_db"] <= 0.0)
        _, rec = self.run_chain("rec", seed=4, profile="recorded-room")
        (wet,) = rec["label"]["wet_candidates"]
        self.assertEqual((wet["stage"], wet["recorded"]), ("rir", True))

    def test_damage_stages_leave_a_plain_damage_label(self) -> None:
        for profile in ("recorded-noise", "mic", "levels", "mp3", "noise-hum"):
            with self.subTest(profile=profile):
                _, rec = self.run_chain("a", seed=2, profile=profile, layers=None)
                self.assertEqual(rec["label"]["policy"], "damage")
                self.assertEqual(rec["label"]["wet_candidates"], [])

    def test_a_clean_sample_is_only_normalised(self) -> None:
        seed = find_seed(CFG, lambda p: p["clean"])
        out, rec = self.run_chain("clean", seed=seed)
        self.assertTrue(rec["clean"])
        self.assertEqual([s["stage"] for s in rec["stages"] if s.get("active")], ["normalise"])
        self.assertEqual(rec["sources_used"], [])
        self.assertTrue(rec["shipped_ok"])
        self.assertEqual(wavio.read_info(out).frames, wavio.read_info(self.src).frames)

    def test_the_input_is_never_modified_and_cannot_be_the_output(self) -> None:
        before = sha(self.src)
        self.run_chain("a", seed=2, profile="all", layers=self.layers)
        self.assertEqual(sha(self.src), before)
        with self.assertRaises(ValueError):
            augment_file(self.src, self.src, seed=1, config=CFG, manifest=MANIFEST, store=self.store)

    def test_sample_formats(self) -> None:
        for fmt in ("s16", "s24", "f32"):
            out, rec = self.run_chain(fmt, seed=2, profile="mic", sample_fmt=fmt)
            self.assertEqual(rec["output"]["sample_fmt"], fmt)
        with self.assertRaises(ValueError):
            self.run_chain("bad", seed=2, sample_fmt="s8")


class Licences(Case):
    def test_a_shipped_config_naming_a_non_shipped_source_is_refused_before_anything_is_written(self) -> None:
        for ref in ("esc-50", "musan/music", "demand", "wham-noise", "freesound/cc-by-nc", "openslr-28/real-air"):
            with self.subTest(ref=ref):
                cfg = parse_config(config_text(stages=recorded_stage("bg_noise", ref) + '[profile.p]\nstages = ["bg_noise"]\n'))
                out = self.dir / "never.wav"
                with self.assertRaises(ConfigError):
                    augment_file(self.src, out, seed=1, config=cfg, manifest=MANIFEST, store=self.store, profile="p")
                self.assertFalse(out.exists())

    def test_a_research_config_may_use_research_only_sources_and_says_so(self) -> None:
        coll = make_world(self.dir / "w2")["openslr-28/pointsource-noises"]
        cfg = parse_config(config_text(purpose="research", stages=recorded_stage("bg_noise", "musan/music") + '[profile.p]\nstages = ["bg_noise"]\n'))
        store = make_store(self.dir, {"musan/music": coll})
        _, rec = self.run_chain("r", seed=1, config=cfg, profile="p", store=store)
        self.assertEqual(rec["purpose"], "research")
        self.assertFalse(rec["shipped_ok"])
        (used,) = rec["sources_used"]
        self.assertEqual((used["ref"], used["use"]), ("musan/music", "research-only"))

    def test_an_excluded_source_is_never_used_whatever_the_purpose(self) -> None:
        cfg = parse_config(config_text(purpose="research", stages=recorded_stage("bg_noise", "wham-noise") + '[profile.p]\nstages = ["bg_noise"]\n'))
        with self.assertRaises(ConfigError):
            augment_file(self.src, self.dir / "never.wav", seed=1, config=cfg, manifest=MANIFEST, store=self.store, profile="p")

    def test_the_eval_config_draws_held_out_collections_and_is_never_shipped_ok(self) -> None:
        world = make_world(self.dir / "w3")
        store = make_store(self.dir, {"mit-ir-survey": world["openslr-28/sim-medium"], "demand": world["openslr-28/pointsource-noises"]})
        for profile, ref in (("recorded-room", "mit-ir-survey"), ("recorded-noise", "demand")):
            with self.subTest(profile=profile):
                _, rec = self.run_chain("e", seed=1, config=EVAL, profile=profile, store=store)
                self.assertEqual((rec["purpose"], rec["pool"], rec["partition"]), ("eval", "heldout", "all"))
                self.assertEqual([u["ref"] for u in rec["sources_used"]], [ref])
                self.assertEqual(rec["sources_used"][0]["use"], "research-only")
                self.assertFalse(rec["shipped_ok"])

    def test_a_file_that_changed_since_it_was_indexed_is_refused_and_nothing_is_written(self) -> None:
        ref = "openslr-28/pointsource-noises"
        self.store.entries(ref)  # build the index now
        for f in (self.world[ref]).iterdir():
            f.write_bytes(f.read_bytes() + b"\0\0\0\0")
        out = self.dir / "never.wav"
        with self.assertRaisesRegex(SourceError, "does not match its indexed SHA-256"):
            augment_file(self.src, out, seed=1, config=CFG, manifest=MANIFEST, store=self.store, profile="recorded-noise")
        self.assertFalse(out.exists())

    def test_a_source_missing_from_disk_is_dropped_from_a_random_plan_and_listed(self) -> None:
        empty = make_store(self.dir, {})
        seed = find_seed(CFG, lambda p: "bg_noise" in {e["stage"] for e in p["stages"] if e["active"]})
        _, rec = self.run_chain("m", seed=seed, store=empty)
        self.assertIn("bg_noise/pointsource", rec["unavailable"])
        self.assertEqual(rec["sources_used"], [])
        self.assertFalse(any(s["stage"] == "bg_noise" and s["active"] for s in rec["stages"]))

    def test_the_dev_partition_draws_other_files_than_the_train_partition(self) -> None:
        store = make_store(self.dir, self.world, dev_percent=50)
        ref = "openslr-28/pointsource-noises"
        for partition in ("train", "dev"):
            for seed in range(4):
                _, rec = self.run_chain("p", seed=seed, profile="recorded-noise", store=store, partition=partition)
                (used,) = rec["sources_used"]
                self.assertEqual(partition_of(ref, used["relpath"], 50), partition)
                self.assertEqual(rec["partition"], partition)


if __name__ == "__main__":
    unittest.main()
