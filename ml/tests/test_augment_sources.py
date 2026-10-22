"""Lane AUG: collections on disk are indexed by hash, split by path, read without trust, and filtered by GR3's lists.

Standard library only; no ffmpeg, no data files.
"""

from __future__ import annotations

import os
import pathlib
import unittest

from augment_support import build_manifest, make_collection, make_store, make_world, source_block, temp_dir, write_noise
from enth_ml.augment import manifest as mf
from enth_ml.augment.sources import (
    GR3_SEED_RANGES,
    IndexEntry,
    SourceError,
    SourceStore,
    build_index,
    default_data_root,
    file_sha256,
    index_digest,
    index_text,
    load_exclusions,
    parse_index,
    partition_of,
    seed_in_gr3_block,
)
from enth_ml.augment.stages import AugCtx, pick_source

MANIFEST = mf.load_manifest()


class Case(unittest.TestCase):
    def setUp(self) -> None:
        self._td = temp_dir()
        self.dir = pathlib.Path(self._td.name)

    def tearDown(self) -> None:
        self._td.cleanup()


class Indexing(Case):
    def test_the_index_is_sorted_hashed_and_skips_symlinks_and_other_files(self) -> None:
        root = make_collection(self.dir / "c", "noise", 5)
        (root / "sub").mkdir()
        write_noise(root / "sub" / "z.wav", seed=9)
        (root / "notes.txt").write_text("not audio")
        (root / "link.wav").symlink_to(root / "noise-000.wav")
        entries = build_index(root)
        self.assertEqual([e.relpath for e in entries], sorted(e.relpath for e in entries))
        self.assertEqual(len(entries), 6)
        self.assertNotIn("link.wav", [e.relpath for e in entries])
        self.assertNotIn("notes.txt", [e.relpath for e in entries])
        for e in entries:
            self.assertEqual(e.sha256, file_sha256(root / e.relpath))
            self.assertEqual(e.bytes, (root / e.relpath).stat().st_size)

    def test_a_prefix_selects_a_part_of_a_directory(self) -> None:
        root = self.dir / "c"
        root.mkdir()
        for name in ("noise-free-sound-1.wav", "noise-sound-bible-1.wav"):
            write_noise(root / name)
        self.assertEqual([e.relpath for e in build_index(root, prefix="noise-free-sound")], ["noise-free-sound-1.wav"])

    def test_the_text_round_trips_and_the_digest_follows_the_content(self) -> None:
        root = make_collection(self.dir / "c", "noise", 3)
        entries = build_index(root)
        self.assertEqual(parse_index(index_text(entries)), entries)
        before = index_digest(entries)
        (root / "noise-001.wav").write_bytes((root / "noise-001.wav").read_bytes() + b"x")
        self.assertNotEqual(index_digest(build_index(root)), before)
        self.assertEqual(index_digest(reversed(entries)), before, "the digest does not depend on the order given")

    def test_a_malformed_index_is_refused(self) -> None:
        for bad in ("a.wav\tnothex\t3\n", "a.wav\t" + "0" * 64 + "\tx\n", "just-one-field\n"):
            with self.subTest(bad=bad[:20]), self.assertRaises(SourceError):
                parse_index(bad)

    def test_a_missing_directory_is_an_error(self) -> None:
        with self.assertRaises(SourceError):
            build_index(self.dir / "nowhere")


class Splitting(Case):
    def test_the_split_is_stable_proportionate_and_depends_on_the_reference(self) -> None:
        self.assertEqual(partition_of("r", "a.wav"), partition_of("r", "a.wav"))
        names = [f"file-{i:05d}.wav" for i in range(8000)]
        dev = sum(1 for n in names if partition_of("r", n, 5) == "dev")
        self.assertAlmostEqual(dev / len(names), 0.05, delta=0.01)
        self.assertNotEqual([partition_of("r", n) for n in names[:400]], [partition_of("s", n) for n in names[:400]])
        self.assertEqual(sum(1 for n in names if partition_of("r", n, 0) == "dev"), 0)
        self.assertEqual(sum(1 for n in names if partition_of("r", n, 100) == "dev"), len(names))
        # Golden values: a change here re-assigns every file between training and validation.
        self.assertEqual([partition_of("openslr-28/sim-small", f"Room{i:03d}/x.wav", 50) for i in range(6)],
                         ["dev", "train", "train", "train", "train", "dev"])

    def test_train_and_dev_partition_the_files_and_all_is_their_union(self) -> None:
        root = make_collection(self.dir / "c", "noise", 30)
        store = SourceStore(MANIFEST, dirs={"x": root}, dev_percent=30)
        tr, dv, al = (store.entries("x", p) for p in ("train", "dev", "all"))
        self.assertTrue(tr and dv)
        self.assertEqual(sorted(e.relpath for e in tr + dv), sorted(e.relpath for e in al))
        self.assertFalse({e.relpath for e in tr} & {e.relpath for e in dv})
        with self.assertRaises(ValueError):
            store.entries("x", "test")

    def test_pick_covers_the_files_evenly_and_never_runs_off_the_end(self) -> None:
        root = make_collection(self.dir / "c", "noise", 10)
        store = SourceStore(MANIFEST, dirs={"x": root}, dev_percent=0)
        names = [store.pick("x", i / 1000.0).relpath for i in range(1000)]
        self.assertEqual(len(set(names)), 10)
        self.assertTrue(all(abs(names.count(n) - 100) <= 1 for n in set(names)))
        self.assertEqual(store.pick("x", 0.9999999999).relpath, "noise-009.wav")
        self.assertEqual(store.pick("x", 0.0).relpath, "noise-000.wav")

    def test_an_empty_partition_is_an_error_that_says_so(self) -> None:
        root = make_collection(self.dir / "c", "noise", 2)
        store = SourceStore(MANIFEST, dirs={"x": root}, dev_percent=0)
        with self.assertRaisesRegex(SourceError, "no dev files"):
            store.pick("x", 0.5, "dev")


class ReadingWithoutTrust(Case):
    def test_a_changed_file_a_replaced_one_and_a_missing_one_are_refused(self) -> None:
        root = make_collection(self.dir / "c", "noise", 3)
        store = SourceStore(MANIFEST, dirs={"x": root})
        entry = store.entries("x", "all")[0]
        self.assertEqual(store.path_of("x", entry), root / entry.relpath)
        (root / entry.relpath).write_bytes(b"changed")
        with self.assertRaisesRegex(SourceError, "does not match"):
            store.path_of("x", entry)
        store.path_of("x", entry, verify=False)
        (root / entry.relpath).unlink()
        (root / entry.relpath).symlink_to(root / "noise-001.wav")
        with self.assertRaisesRegex(SourceError, "missing or not a regular file"):
            store.path_of("x", entry, verify=False)
        (root / entry.relpath).unlink()
        with self.assertRaises(SourceError):
            store.path_of("x", entry)

    def test_an_index_entry_cannot_name_a_file_outside_the_directory(self) -> None:
        root = make_collection(self.dir / "c", "noise", 1)
        outside = self.dir / "outside.wav"
        write_noise(outside)
        store = SourceStore(MANIFEST, dirs={"x": root})
        evil = IndexEntry("../outside.wav", file_sha256(outside), outside.stat().st_size)
        with self.assertRaisesRegex(SourceError, "escapes"):
            store.path_of("x", evil)

    def test_a_written_index_is_what_the_store_reads_afterwards(self) -> None:
        root = make_collection(self.dir / "data" / "nsynth", "noise", 4)
        store = SourceStore(MANIFEST, data_root=self.dir / "data")
        path, digest, count = store.write_index("nsynth")
        self.assertEqual(count, 4)
        self.assertEqual(path, self.dir / "data" / "_index" / "nsynth.tsv")
        # Another process reads the file; a file added later does not enter the index until it is rewritten.
        write_noise(root / "extra.wav")
        fresh = SourceStore(MANIFEST, data_root=self.dir / "data")
        self.assertEqual(index_digest(fresh.index("nsynth")), digest)
        self.assertEqual(len(fresh.index("nsynth")), 4)

    def test_availability(self) -> None:
        root = make_collection(self.dir / "c", "noise", 2)
        store = make_store(self.dir, {"nsynth": root})
        self.assertTrue(store.available("nsynth"))
        self.assertFalse(store.available("musan/music"))
        self.assertFalse(store.available("no-such-source"))

    def test_the_data_root_follows_the_environment(self) -> None:
        old = os.environ.get("ENTH_DATA_DIR")
        try:
            os.environ["ENTH_DATA_DIR"] = str(self.dir / "elsewhere")
            self.assertEqual(default_data_root(MANIFEST), self.dir / "elsewhere")
            del os.environ["ENTH_DATA_DIR"]
            self.assertEqual(default_data_root(MANIFEST), pathlib.Path(os.path.expanduser(MANIFEST.local_root)))
        finally:
            if old is not None:
                os.environ["ENTH_DATA_DIR"] = old
            else:
                os.environ.pop("ENTH_DATA_DIR", None)


class Exclusions(Case):
    def test_the_real_lists_parse_and_know_the_gr3_block(self) -> None:
        ex = load_exclusions()
        self.assertRegex(ex.sha256, r"^[0-9a-f]{64}$")
        self.assertIn((770000000000, 770999999999), ex.seed_ranges())
        self.assertTrue(ex.values("degrade-variant"))
        self.assertTrue(ex.values("freesound-id"))

    def test_the_seed_constant_is_the_union_of_the_lists_seed_ranges(self) -> None:
        ex = load_exclusions()
        listed = sorted({(int(lo), int(hi)) for v in ex.values("seed-range") for lo, _, hi in [v.partition("..")]})
        self.assertEqual(tuple(listed), GR3_SEED_RANGES, "update GR3_SEED_RANGES with the list (and say so)")
        for lo, hi in ex.seed_ranges():  # the degradation lists' ranges lie inside the block
            self.assertTrue(seed_in_gr3_block(lo) and seed_in_gr3_block(hi), (lo, hi))
        self.assertFalse(seed_in_gr3_block(GR3_SEED_RANGES[0][0] - 1))
        self.assertFalse(seed_in_gr3_block(GR3_SEED_RANGES[0][1] + 1))

    def _lists(self, **files: str) -> pathlib.Path:
        d = self.dir / "gr3"
        d.mkdir(exist_ok=True)
        for name in ("exclude-degraded.txt", "exclude-real.txt", "exclude-synth.txt"):
            (d / name).write_text(files.get(name, ""))
        return d

    def test_an_unknown_entry_type_or_a_missing_file_is_refused(self) -> None:
        with self.assertRaisesRegex(SourceError, "unknown entry type"):
            load_exclusions(self._lists(**{"exclude-real.txt": "teleport 7\n"}))
        with self.assertRaisesRegex(SourceError, "missing"):
            load_exclusions(self.dir / "empty")

    def test_audio_rules(self) -> None:
        d = self._lists(**{"exclude-real.txt": (
            "freesound-id 42  # a comment\nfreesound-pack 7\naudio-sha256 " + "ab" * 32 + "\npath /data/secret/**\npath ~/relative-to-home/**\n"
            "path content/gr3/**\n")})
        ex = load_exclusions(d)
        self.assertEqual(ex.audio_excluded(freesound_id=42), "freesound-id 42")
        self.assertIsNone(ex.audio_excluded(freesound_id=43))
        self.assertEqual(ex.audio_excluded(freesound_pack=7), "freesound-pack 7")
        self.assertTrue(ex.audio_excluded(sha256="ab" * 32).startswith("audio-sha256"))
        self.assertIsNone(ex.audio_excluded(sha256="cd" * 32))
        self.assertEqual(ex.audio_excluded(path="/data/secret/deep/file.wav"), "path /data/secret/**")
        self.assertIsNone(ex.audio_excluded(path="/data/public/file.wav"))
        self.assertTrue(ex.audio_excluded(path=os.path.expanduser("~/relative-to-home/x.wav")))
        self.assertIsNone(ex.audio_excluded(path="content/gr3/x.wav"), "repo-relative patterns concern synth files")

    def test_an_excluded_file_is_never_picked(self) -> None:
        root = make_collection(self.dir / "c", "noise", 6)
        entries = build_index(root)
        keep = entries[3]
        lines = "".join(f"audio-sha256 {e.sha256}\n" for e in entries if e is not keep)
        ex = load_exclusions(self._lists(**{"exclude-real.txt": lines}))
        store = SourceStore(MANIFEST, dirs={"x": root}, dev_percent=0, exclusions=ex)
        self.assertEqual([e.relpath for e in store.entries("x", "all")], [keep.relpath])
        for i in range(50):
            self.assertEqual(store.pick("x", i / 50.0).relpath, keep.relpath)

    def test_an_excluded_directory_is_never_picked(self) -> None:
        root = make_collection(self.dir / "c", "noise", 4)
        ex = load_exclusions(self._lists(**{"exclude-real.txt": f"path {os.path.realpath(root)}/**\n"}))
        store = SourceStore(MANIFEST, dirs={"x": root}, exclusions=ex)
        self.assertEqual(store.entries("x", "all"), [])
        self.assertFalse(store.available("x") and bool(store.entries("x", "all")))


class LicenceAtThePick(Case):
    """pick_source is the last gate: even a config that got past the checks cannot read a source its purpose forbids."""

    def ctx(self, purpose: str, ref: str, manifest=MANIFEST) -> AugCtx:
        root = make_collection(self.dir / "c", "noise", 3)
        return AugCtx(self.dir, 16000, 1, 1000, self.dir / "x.wav", store=make_store(self.dir, {ref: root}, manifest),
                      manifest=manifest, purpose=purpose)

    def test_shipped_training_reads_only_shipped_sources(self) -> None:
        path, entry = pick_source(self.ctx("shipped-training", "openslr-28/pointsource-noises"), "openslr-28/pointsource-noises", 0.5)
        self.assertTrue(path.is_file())
        for ref in ("esc-50", "demand", "musan/music", "wham-noise", "openslr-28/real-air", "freesound/cc-by-nc"):
            with self.subTest(ref=ref), self.assertRaises(mf.LicenceError):
                pick_source(self.ctx("shipped-training", ref), ref, 0.5)

    def test_research_reads_research_only_but_never_excluded_sources(self) -> None:
        pick_source(self.ctx("research", "esc-50"), "esc-50", 0.5)
        for ref in ("wham-noise", "openair"):
            with self.subTest(ref=ref), self.assertRaisesRegex(mf.LicenceError, "excluded"):
                pick_source(self.ctx("research", ref), ref, 0.5)

    def test_eval_never_reads_excluded_sources_either(self) -> None:
        pick_source(self.ctx("eval", "demand"), "demand", 0.5)
        with self.assertRaises(mf.LicenceError):
            pick_source(self.ctx("eval", "wham-noise"), "wham-noise", 0.5)

    def test_a_manifest_that_lies_about_a_licence_still_cannot_ship_it(self) -> None:
        m = build_manifest(source_block(id="x", licence="CC-BY-NC-4.0", use="shipped-training"))
        with self.assertRaises(mf.LicenceError):
            pick_source(self.ctx("shipped-training", "x", m), "x", 0.5)

    def test_every_pick_is_listed_once(self) -> None:
        ctx = self.ctx("shipped-training", "openslr-28/pointsource-noises")
        for _ in range(3):
            pick_source(ctx, "openslr-28/pointsource-noises", 0.5)
        self.assertEqual(len(ctx.used), 1)
        self.assertEqual(ctx.used[0]["licence"], "CC0-1.0")


class Hermetic(Case):
    def test_the_synthetic_world_never_touches_the_real_data_root(self) -> None:
        world = make_world(self.dir / "w")
        store = make_store(self.dir, world)
        self.assertTrue(str(store.data_root).startswith(str(self.dir)))
        self.assertFalse(store.available("musan/noise-free-sound"))


if __name__ == "__main__":
    unittest.main()
