"""Lane AUG: the data manifest records a licence per source and refuses NC and SA data for shipped training.

Acceptance (docs/phase-3.md, AUG): "Licences recorded per source; no NC or SA data in shipped-model training".
Standard library only; no ffmpeg, no data files (checks against the fetched archive skip when it is absent).
"""

from __future__ import annotations

import hashlib
import os
import unittest

from augment_support import build_manifest, source_block
from enth_ml.augment import manifest as mf
from enth_ml.augment.sources import SourceStore, index_digest

# Every collection docs/r3.md ("Augmentation and licences") and the open check name.
NAMED = {
    "nsynth", "musan", "fsd50k", "but-reverbdb", "openslr-28", "tinysol", "freesound",  # shipped-capable
    "esc-50", "demand", "mit-ir-survey",  # evaluation-only and local
    "wham-noise", "openair",  # excluded
}
# Collections whose terms keep them out of shipped training, with the flag that does it.
KEPT_OUT = {
    "esc-50": "NC", "demand": "SA", "wham-noise": "NC", "mit-ir-survey": None, "openair": None,
}


class RealManifest(unittest.TestCase):
    def setUp(self) -> None:
        self.m = mf.load_manifest()

    def test_the_file_is_sound(self) -> None:
        self.assertEqual(mf.check_manifest(self.m), [])
        self.assertEqual(self.m.version, 1)
        self.assertEqual(self.m.shipped_licences, mf.DEFAULT_SHIPPED_LICENCES)

    def test_every_source_has_a_licence_read_at_its_source(self) -> None:
        for sid, s in self.m.sources.items():
            with self.subTest(source=sid):
                self.assertTrue(s.licence and s.licence_url and s.licence_evidence and s.licence_read_on, sid)
                self.assertRegex(s.licence_read_on, r"^2026-10-\d\d$")
                self.assertIn(s.use, mf.USES)

    def test_the_collections_the_design_names_are_all_recorded(self) -> None:
        self.assertTrue(NAMED <= set(self.m.sources), NAMED - set(self.m.sources))

    def test_no_nc_or_sa_data_is_allowed_for_shipped_training(self) -> None:
        for ref in self.m.all_refs():
            lic = self.m.licence_of(ref)
            if self.m.effective_use(ref) == "shipped-training":
                self.assertIsNone(mf.licence_refusal(lic), f"{ref}: {lic}")
                self.assertEqual(mf.licence_flags(lic), set(), f"{ref}: {lic}")
        # and the other direction: what carries NC or SA is kept out, wherever it sits
        flagged = [r for r in self.m.all_refs() if mf.licence_flags(self.m.licence_of(r))]
        self.assertGreaterEqual(len(flagged), 6, flagged)
        for ref in flagged:
            self.assertNotEqual(self.m.effective_use(ref), "shipped-training", ref)
            with self.assertRaises(mf.LicenceError):
                mf.require_shipped([ref], self.m)

    def test_evaluation_only_and_excluded_collections_stay_out(self) -> None:
        for sid, flag in KEPT_OUT.items():
            with self.subTest(source=sid):
                self.assertNotEqual(self.m.effective_use(sid), "shipped-training")
                if flag:
                    self.assertIn(flag, mf.licence_flags(self.m.sources[sid].licence))
                self.assertTrue(self.m.sources[sid].reason)
        self.assertEqual(self.m.effective_use("wham-noise"), "excluded")
        self.assertEqual(self.m.effective_use("openair"), "excluded")
        self.assertEqual(self.m.sources["demand"].licence, "CC-BY-SA-3.0")
        self.assertEqual(self.m.sources["esc-50"].licence, "CC-BY-NC-3.0")

    def test_fsd50k_and_freesound_split_by_licence(self) -> None:
        for sid in ("fsd50k", "freesound"):
            parts = self.m.sources[sid].parts
            self.assertEqual(self.m.effective_use(f"{sid}/{'clips-' if sid == 'fsd50k' else ''}cc0"), "shipped-training")
            for p in parts.values():
                if mf.licence_flags(p.licence):
                    self.assertEqual(p.use, "research-only", p.id)
            self.assertEqual(self.m.effective_use(sid), "research-only", "a source is only as free as its least free part")

    def test_the_open_check_licences_are_recorded(self) -> None:
        # docs/phase-3.md, open check: AMR, GSM, WHAM!, OpenSLR 28's sources, Freesound's terms on training.
        t = self.m.tools
        self.assertEqual(t["libopencore-amrnb"].licence, "Apache-2.0")
        self.assertIn("NOT A GRANT OF PATENT RIGHTS", t["libopencore-amrnb"].evidence)
        self.assertIn("TU-Berlin", t["libgsm"].licence)
        self.assertEqual(t["libvo-amrwbenc"].licence, "Apache-2.0")
        self.assertEqual(self.m.sources["wham-noise"].licence, "CC-BY-NC-4.0")
        osr = self.m.sources["openslr-28"]
        self.assertEqual(osr.licence, "Apache-2.0")
        self.assertEqual(osr.parts["real-rwcp"].use, "research-only")
        self.assertEqual(osr.parts["real-reverb2014"].use, "research-only")
        self.assertEqual(osr.parts["real-air"].licence, "MIT")
        self.assertEqual(osr.parts["pointsource-noises"].use, "shipped-training")
        self.assertIn("training", self.m.sources["freesound"].licence_evidence)
        self.assertIn("Data Packs", " ".join(self.m.sources["freesound"].obligations + (self.m.sources["freesound"].licence_evidence,)))

    def test_tools_run_as_subprocesses_or_not_at_all(self) -> None:
        for tid, tool in self.m.tools.items():
            self.assertIn(tool.usage, mf.TOOL_USAGE, tid)
        for tid in ("pedalboard", "gpurir", "fdk-aac"):
            self.assertEqual(self.m.tools[tid].usage, "excluded", tid)
        for tid in ("ffmpeg", "libmp3lame", "libshine", "libgsm", "libopencore-amrnb"):
            self.assertEqual(self.m.tools[tid].usage, "subprocess", tid)
        self.assertIn("GPL-3.0", self.m.tools["pedalboard"].licence)
        self.assertIn("AGPL-3.0", self.m.tools["gpurir"].licence)

    def test_held_out_collections_never_train(self) -> None:
        held = [sid for sid, s in self.m.sources.items() if s.role == "heldout-eval"]
        self.assertTrue({"demand", "mit-ir-survey", "synthetic-heldout"} <= set(held))
        for sid in held:
            self.assertNotEqual(self.m.effective_use(sid), "shipped-training", sid)
            with self.assertRaises(mf.LicenceError):
                mf.require_shipped([sid], self.m)

    def test_shipped_references_pass_the_gate(self) -> None:
        refs = self.m.shipped_refs()
        self.assertIn("openslr-28/sim-small", refs)
        self.assertIn("openslr-28/pointsource-noises", refs)
        self.assertNotIn("openslr-28", refs, "the whole archive includes research-only parts")
        mf.require_shipped(refs, self.m)
        with self.assertRaises(mf.ManifestError):
            mf.require_shipped(["no-such-source"], self.m)
        with self.assertRaises(mf.ManifestError):
            mf.require_shipped(["openslr-28/no-such-part"], self.m)

    def test_cc_by_sources_carry_a_credit_line_for_the_notices_file(self) -> None:
        for ref in self.m.shipped_refs():
            src, _ = self.m.split_ref(ref)
            if mf.canonical_licence(self.m.licence_of(ref)).startswith("CC-BY"):
                self.assertTrue(src.attribution, ref)

    def test_a_fetched_archive_is_pinned(self) -> None:
        osr = self.m.sources["openslr-28"].archive
        self.assertTrue(osr.fetched)
        self.assertEqual(osr.bytes, 1311166223)
        self.assertEqual(osr.sha256, "3b50cfde915b3984738169b4beb341e9f6b8062ae4c2076146c5db71c2c05dc7")
        for sid, s in self.m.sources.items():
            if s.archive and not s.archive.fetched:
                self.assertEqual(s.archive.sha256, "", f"{sid}: no hash before the first fetch")

    def test_the_fetched_archive_and_indexes_match_when_present(self) -> None:
        store = SourceStore(self.m)
        arch = self.m.sources["openslr-28"].archive
        zip_path = store.data_root / arch.local
        if not zip_path.is_file():
            self.skipTest("the OpenSLR 28 archive is not on this machine (see ml/data/fetch.py)")
        self.assertEqual(zip_path.stat().st_size, arch.bytes)
        if os.environ.get("ENTH_VERIFY_DATA"):  # hashing 1.3 GB takes about 13 s: opt in
            h = hashlib.sha256()
            with open(zip_path, "rb") as f:
                for block in iter(lambda: f.read(1 << 22), b""):
                    h.update(block)
            self.assertEqual(h.hexdigest(), arch.sha256)
        for pid, part in self.m.sources["openslr-28"].parts.items():
            ref = f"openslr-28/{pid}"
            if not store.directory(ref).is_dir():
                self.skipTest("the archive is not extracted")
            entries = store.index(ref)
            self.assertEqual(len(entries), part.files, ref)
            self.assertEqual(index_digest(entries), part.index_sha256, ref)


class Refusals(unittest.TestCase):
    """The check is not vacuous: it catches each way a manifest could let NC or SA data into shipped training."""

    def problems(self, **kw: str) -> list[str]:
        return mf.check_manifest(build_manifest(source_block(**kw)))

    def test_a_clean_shipped_source_passes(self) -> None:
        for lic in ("CC0-1.0", "CC-BY-4.0", "CC-BY-3.0", "Apache-2.0", "CC BY 4.0", "cc-by-4.0"):
            with self.subTest(licence=lic):
                self.assertEqual(self.problems(licence=lic), [])

    def test_nc_sa_nd_sampling_and_unknown_licences_are_refused(self) -> None:
        bad = (
            "CC-BY-NC-4.0", "CC-BY-NC-SA-4.0", "CC BY-NC 3.0", "CC-BY-SA-3.0", "CC-BY-SA-4.0", "CC-BY-ND-4.0",
            "CC-Sampling+-1.0", "CC0-1.0 AND CC-BY-SA-4.0", "CC-BY-4.0 OR CC-BY-NC-4.0", "", "none", "unknown",
            "none-stated", "GPL-3.0-only", "MIT", "all rights reserved", "research use only", "per-upload",
        )
        for lic in bad:
            with self.subTest(licence=lic):
                got = self.problems(licence=lic)
                self.assertTrue(any("shipped-training" in p for p in got), (lic, got))
                with self.assertRaises(mf.LicenceError):
                    mf.require_shipped(["x"], build_manifest(source_block(licence=lic)))

    def test_the_same_licences_are_fine_for_research_and_excluded_with_a_reason(self) -> None:
        for use in ("research-only", "excluded"):
            self.assertEqual(self.problems(licence="CC-BY-NC-4.0", use=use, reason="NonCommercial"), [])
            self.assertTrue(any("reason" in p for p in self.problems(licence="CC-BY-NC-4.0", use=use)))

    def test_missing_pieces_are_reported(self) -> None:
        self.assertTrue(any("licence_url" in p for p in self.problems(licence_url="")))
        self.assertTrue(any("licence_url" in p for p in self.problems(licence_url="file:///etc/passwd")))
        self.assertTrue(any("licence_read_on" in p for p in self.problems(licence_read_on="")))
        self.assertTrue(any("licence_read_on" in p for p in self.problems(licence_read_on="22/10/2026")))
        self.assertTrue(any("licence_evidence" in p for p in self.problems(licence_evidence="")))
        self.assertTrue(any("attribution" in p for p in self.problems(licence="CC-BY-4.0", attribution="")))
        self.assertTrue(any("use" in p for p in self.problems(use="sometimes")))
        self.assertTrue(any("role" in p for p in self.problems(role="mystery")))

    def test_a_held_out_collection_can_never_be_shipped_training(self) -> None:
        self.assertTrue(any("held-out" in p for p in self.problems(role="heldout-eval")))

    def test_a_part_can_tighten_a_source_but_not_loosen_it(self) -> None:
        tight = source_block(licence="Apache-2.0") + (
            '[[source.part]]\nid = "a"\npath = "a"\nuse = "research-only"\nreason = "unverified"\n'
        )
        self.assertEqual(mf.check_manifest(build_manifest(tight)), [])
        loose = source_block(use="research-only", reason="no licence") + (
            '[[source.part]]\nid = "a"\npath = "a"\nuse = "shipped-training"\n'
        )
        self.assertTrue(any("looser" in p for p in mf.check_manifest(build_manifest(loose))))
        m = build_manifest(loose)
        self.assertEqual(m.effective_use("x/a"), "research-only", "the effective use never exceeds the source's")
        part_nc = source_block(licence="Apache-2.0") + (
            '[[source.part]]\nid = "a"\npath = "a"\nlicence = "CC-BY-NC-4.0"\nuse = "shipped-training"\n'
        )
        self.assertTrue(any("shipped-training" in p for p in mf.check_manifest(build_manifest(part_nc))))

    def test_a_source_with_a_research_part_is_not_shipped_as_a_whole(self) -> None:
        text = source_block(licence="Apache-2.0") + (
            '[[source.part]]\nid = "ok"\npath = "ok"\nuse = "shipped-training"\n'
            '[[source.part]]\nid = "no"\npath = "no"\nuse = "research-only"\nreason = "unverified"\n'
        )
        m = build_manifest(text)
        self.assertEqual(m.effective_use("x"), "research-only")
        mf.require_shipped(["x/ok"], m)
        with self.assertRaises(mf.LicenceError):
            mf.require_shipped(["x"], m)
        with self.assertRaises(mf.LicenceError):
            mf.require_shipped(["x/no"], m)

    def test_a_fetched_archive_needs_its_hash(self) -> None:
        arch = '[source.archive]\nurl = "https://example.test/a.zip"\nbytes = 10\nsha256 = "%s"\nfetched = true\nfetched_on = "2026-10-22"\nlocal = "dl/a.zip"\n'
        self.assertEqual(mf.check_manifest(build_manifest(source_block() + arch % ("a" * 64))), [])
        self.assertTrue(any("sha256" in p for p in mf.check_manifest(build_manifest(source_block() + arch % ""))))
        self.assertTrue(any("sha256" in p for p in mf.check_manifest(build_manifest(source_block() + arch % ("A" * 64)))))
        unfetched = arch.replace("fetched = true", "fetched = false") % ""
        self.assertEqual(mf.check_manifest(build_manifest(source_block() + unfetched)), [])

    def test_tools_are_never_imported(self) -> None:
        tool = '[[tool]]\nid = "t"\nname = "t"\nlicence = "GPL-3.0"\nversion = "1"\nusage = "%s"\nurl = "https://example.test"\nevidence = "read"\n'
        self.assertEqual(mf.check_manifest(build_manifest("", tool % "subprocess")), [])
        self.assertEqual(mf.check_manifest(build_manifest("", tool % "excluded")), [])
        self.assertTrue(any("usage" in p for p in mf.check_manifest(build_manifest("", tool % "import"))))

    def test_structure_errors_raise(self) -> None:
        with self.assertRaises(mf.ManifestError):
            mf.parse_manifest('schema = "other"\n')
        with self.assertRaises(mf.ManifestError):
            build_manifest(source_block() + source_block())  # the same id twice

    def test_licence_flags_and_canonical_names(self) -> None:
        self.assertEqual(mf.licence_flags("CC-BY-NC-SA-4.0"), {"NC", "SA"})
        self.assertEqual(mf.licence_flags("CC BY-ND 4.0"), {"ND"})
        self.assertEqual(mf.licence_flags("CC-Sampling+-1.0"), {"SAMPLING"})
        self.assertEqual(mf.licence_flags("CC-BY-4.0"), set())
        self.assertEqual(mf.licence_flags("Apache-2.0"), set())
        self.assertEqual(mf.canonical_licence(" cc by 4.0 "), "CC-BY-4.0")


if __name__ == "__main__":
    unittest.main()
