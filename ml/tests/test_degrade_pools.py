"""Lane DEG: the pool definitions are valid data, the two pools are disjoint, and plans stay inside their pool.

Standard library only; no ffmpeg needed.
"""

from __future__ import annotations

import dataclasses
import unittest

from degrade_support import ML  # noqa: F401  (puts ml/ on sys.path)
from enth_ml.degrade import POOLS_PATH, check_disjoint, load_pools, make_plan
from enth_ml.degrade.pools import (
    ALL_PROFILE,
    STAGE_ORDER,
    PoolError,
    Variant,
    domain,
    domains_meet,
    variants_disjoint,
)
from enth_ml.degrade.rng import Rng, derive, instance_seed

FAMILY_PROFILES = ("mp3", "noise-hum", "reverb", "clip-limit", "eq", "mono", "trim", "band-bit")
# Compound damage in one chain, for lane DS's real-set degraded copies; also in both pools.
COMPOSITE_PROFILES = ("upload-noisy", "upload-room", "upload-loud", "phone-recording")


def point_in_domain(value, spec) -> bool:
    return domains_meet(domain(value), domain(spec))


class PoolFile(unittest.TestCase):
    def setUp(self) -> None:
        self.ps = load_pools()
        self.train, self.heldout = self.ps.pools["train"], self.ps.pools["heldout"]

    def test_the_file_loads_with_two_pools_and_every_stage(self) -> None:
        self.assertEqual(sorted(self.ps.pools), ["heldout", "train"])
        self.assertTrue(POOLS_PATH.is_file())
        for pool in self.ps.pools.values():
            self.assertEqual(tuple(pool.stages), STAGE_ORDER)
            for stage in pool.stages.values():
                for v in stage.variants.values():
                    self.assertIn("kind", v.params, f"{pool.name}/{stage.name}/{v.name}")

    def test_both_pools_normalise_to_the_same_target(self) -> None:
        self.assertEqual(self.train.target_lufs, -23.0)
        self.assertEqual(self.heldout.target_lufs, -23.0)

    def test_heldout_is_never_clean_and_train_keeps_some_clean(self) -> None:
        self.assertEqual(self.heldout.p_clean, 0.0)
        self.assertTrue(0.10 <= self.train.p_clean <= 0.20)

    def test_names_a_manifest_can_reference_exist(self) -> None:
        names = self.ps.names()
        self.assertIn("shine-cbr", names["heldout"]["stages"]["codec"]["variants"])
        self.assertEqual(self.heldout.ref("codec", "shine-cbr"), "heldout/codec/shine-cbr")
        for fam in FAMILY_PROFILES + COMPOSITE_PROFILES:
            self.assertIn(fam, self.train.profiles, f"train profile {fam}")
            self.assertIn(fam, self.heldout.profiles, f"heldout profile {fam}")
        for extra in ("eq-other", "codec-other", "phone-codec"):
            self.assertIn(extra, self.heldout.profiles)

    def test_the_gr3_families_are_in_the_heldout_pool(self) -> None:
        # Report 04 section 3.2: MP3, noise plus hum, reverb, clipping and limiting, phone-speaker EQ, stereo to mono.
        codec = self.heldout.stages["codec"].variants
        self.assertEqual(codec["shine-cbr"].params["encoder"], "libshine")
        self.assertEqual(self.heldout.profiles["mp3"].variants["codec"], ("shine-cbr", "lame-vbr", "lame-abr"))
        self.assertIn("phone-speaker", self.heldout.stages["eq"].variants)
        self.assertIn("amr-nb", codec)
        self.assertIn("flutter", self.heldout.stages["room"].variants)
        self.assertEqual(self.heldout.profiles["noise-hum"].stages, ("noise", "hum"))
        shine = self.heldout.stages["codec"].variants["shine-cbr"].params["bitrate_kbps"]["choice"]
        self.assertEqual((min(shine), max(shine)), (32, 96), "docs/r3.md: held-out MP3 at 32 to 96 kbps")
        rt60 = [v.params["rt60_s"] for v in self.heldout.stages["room"].variants.values()]
        self.assertTrue(all(0.4 <= lo and hi <= 2.5 for lo, hi in rt60), rt60)

    def test_composite_profiles_chain_several_stages_in_stage_order(self) -> None:
        want = {
            "upload-noisy": ("noise", "hum", "codec"),
            "upload-room": ("room", "codec"),
            "upload-loud": ("dynamics", "codec"),
            "phone-recording": ("room", "eq", "mono", "codec"),
        }
        for pool in (self.train, self.heldout):
            for name, stages in want.items():
                self.assertEqual(pool.profiles[name].stages, stages, f"{pool.name}/{name}")
        held = self.heldout.profiles
        self.assertEqual(held["phone-recording"].variants["codec"], ("amr-nb", "gsm"))
        self.assertEqual(held["phone-recording"].variants["room"], ("living-room",))
        self.assertEqual(held["upload-room"].variants["room"], ("living-room", "hall", "cathedral", "flutter"))

    def test_a_bad_file_is_refused(self) -> None:
        import tempfile
        import pathlib

        with tempfile.TemporaryDirectory() as td:
            bad = pathlib.Path(td) / "bad.toml"
            bad.write_text('schema = "wrong"\n', encoding="utf-8")
            with self.assertRaises(PoolError):
                load_pools(bad)


class Disjointness(unittest.TestCase):
    """Acceptance (phase-3, DEG): the held-out pools are disjoint from training's."""

    def setUp(self) -> None:
        self.ps = load_pools()
        self.train, self.heldout = self.ps.pools["train"], self.ps.pools["heldout"]

    def test_train_and_heldout_are_disjoint(self) -> None:
        self.assertEqual(check_disjoint(self.train, self.heldout), [])
        self.assertEqual(check_disjoint(self.heldout, self.train), [])

    def test_every_heldout_variant_differs_from_every_train_variant_of_its_stage(self) -> None:
        for stage in STAGE_ORDER:
            for hn, hv in self.heldout.stages[stage].variants.items():
                for tn, tv in self.train.stages[stage].variants.items():
                    self.assertTrue(variants_disjoint(hv, tv), f"{stage}: heldout/{hn} meets train/{tn}")

    def test_the_encoders_and_noises_gr3_names_are_not_trained_on(self) -> None:
        # The held-out MP3 encoder, colours and phone codecs never appear in training.
        enc = lambda pool: {v.params.get("encoder") for v in pool.stages["codec"].variants.values()}  # noqa: E731
        self.assertIn("libshine", enc(self.heldout))
        self.assertNotIn("libshine", enc(self.train))
        self.assertTrue({"libopencore_amrnb", "libgsm"} <= enc(self.heldout))
        self.assertFalse({"libopencore_amrnb", "libgsm"} & enc(self.train))
        colours = lambda pool: {v.params["color"] for v in pool.stages["noise"].variants.values()}  # noqa: E731
        self.assertFalse(colours(self.train) & {"blue", "violet", "velvet"})
        self.assertTrue({"blue", "violet", "velvet"} <= colours(self.heldout))
        # An encoder both pools use (LAME, ffmpeg's AAC, libopus) is used in a different mode, coder or application.
        configs = lambda pool: {  # noqa: E731
            tuple(v.params.get(k) for k in ("encoder", "mode", "aac_coder", "application"))
            for v in pool.stages["codec"].variants.values()
        }
        self.assertFalse(configs(self.train) & configs(self.heldout))

    def test_the_checker_is_not_vacuous(self) -> None:
        # A pool is not disjoint from itself, from a copy, or from one that smuggles a training variant in.
        self.assertTrue(check_disjoint(self.train, self.train))
        codec = dict(self.heldout.stages["codec"].variants)
        codec["lame-cbr-again"] = Variant("lame-cbr-again", 1.0, self.train.stages["codec"].variants["lame-cbr"].params)
        spoiled = dataclasses.replace(
            self.heldout,
            stages={**self.heldout.stages, "codec": dataclasses.replace(self.heldout.stages["codec"], variants=codec)},
        )
        problems = check_disjoint(self.train, spoiled)
        self.assertTrue(any("lame-cbr-again" in p for p in problems), problems)
        same_parity = dataclasses.replace(self.heldout, instance_parity=self.train.instance_parity)
        self.assertTrue(any("instance_parity" in p for p in check_disjoint(self.train, same_parity)))
        # An overlapping range of the same kind is caught, a non-overlapping one is not.
        near = Variant("n", 1.0, {**self.train.stages["tail_trunc"].variants["truncate"].params, "keep_s": [1.4, 2.0]})
        far = Variant("f", 1.0, {**self.train.stages["tail_trunc"].variants["truncate"].params, "keep_s": [1.6, 2.0]})
        train_v = self.train.stages["tail_trunc"].variants["truncate"]
        self.assertFalse(variants_disjoint(train_v, near))
        self.assertTrue(variants_disjoint(train_v, far))

    def test_domains_meet(self) -> None:
        self.assertTrue(domains_meet(domain([1.0, 3.0]), domain([3.0, 4.0])))
        self.assertFalse(domains_meet(domain([1.0, 3.0]), domain([3.1, 4.0])))
        self.assertTrue(domains_meet(domain({"choice": [1, 2]}), domain({"choice": [2, 3]})))
        self.assertFalse(domains_meet(domain({"choice": [1, 2]}), domain({"choice": [3]})))
        self.assertTrue(domains_meet(domain({"choice": [5, 9]}), domain([4, 6])))
        self.assertFalse(domains_meet(domain({"choice": [5, 9]}), domain([6, 8])))
        self.assertFalse(domains_meet(domain("a"), domain("b")))
        self.assertFalse(domains_meet(domain(True), domain([0, 1])))  # a flag is not a number
        self.assertTrue(domains_meet(domain({"log": [80.0, 400.0]}), domain(400.0)))

    def test_instance_seeds_never_collide_between_pools(self) -> None:
        train_seeds, held_seeds = set(), set()
        for seed in range(400):
            for stage in ("room", "noise", "bit_depth"):
                for label in ("ir", "noise", "dither"):
                    t = instance_seed(seed, "train", self.train.instance_parity, stage, label)
                    h = instance_seed(seed, "heldout", self.heldout.instance_parity, stage, label)
                    self.assertEqual(t % 2, 0)
                    self.assertEqual(h % 2, 1)
                    self.assertLess(t + 2 * 8, 2**32)  # the per-channel offsets stay inside anoisesrc's range
                    self.assertLess(h + 2 * 8, 2**32)
                    train_seeds.add(t)
                    held_seeds.add(h)
        self.assertEqual(train_seeds & held_seeds, set())
        self.assertGreater(len(train_seeds), 1000)  # and the seeds really vary

    def test_plans_stay_inside_their_own_pool(self) -> None:
        for pool in (self.train, self.heldout):
            other = self.heldout if pool is self.train else self.train
            for seed in range(250):
                for profile in (None, ALL_PROFILE):
                    plan = make_plan(self.ps, pool.name, seed, profile)
                    for e in plan["stages"]:
                        if not e["active"]:
                            continue
                        self.assertTrue(e["variant"].startswith(pool.name + "/"), e["variant"])
                        variant = pool.stages[e["stage"]].variants[e["variant"].split("/")[2]]
                        if e["stage"] == "eq":
                            continue  # structured; its numbers are checked below
                        for key, spec in variant.params.items():
                            if key in e["params"]:
                                self.assertTrue(
                                    point_in_domain(e["params"][key], spec),
                                    f"{e['variant']}.{key} = {e['params'][key]!r} outside {spec!r}",
                                )
                        # The drawn point lies in no variant of the other pool (flat stages).
                        for ov in other.stages[e["stage"]].variants.values():
                            shared = [k for k in ov.params if k in e["params"] and k in variant.params]
                            inside = all(point_in_domain(e["params"][k], ov.params[k]) for k in shared)
                            self.assertFalse(inside, f"{e['variant']} drew a point inside {other.name}/{ov.name}")

    def test_eq_components_stay_inside_their_variant(self) -> None:
        for pool in (self.train, self.heldout):
            for seed in range(300):
                plan = make_plan(self.ps, pool.name, seed, "eq")
                e = [x for x in plan["stages"] if x["stage"] == "eq"][0]
                v = pool.stages["eq"].variants[e["variant"].split("/")[2]].params
                p = e["params"]
                if "hp" in p:
                    self.assertTrue(point_in_domain(p["hp"]["f_hz"], v["hp_hz"]))
                if "lp" in p:
                    self.assertTrue(point_in_domain(p["lp"]["f_hz"], v["lp_hz"]))
                for pk in p.get("peaks", []):
                    self.assertTrue(point_in_domain(pk["f_hz"], v["peak_hz"]))
                    self.assertTrue(point_in_domain(pk["gain_db"], v["peak_gain_db"]))
                    self.assertTrue(point_in_domain(pk["q"], v["peak_q"]))
                if "tilt_db_oct" in p:
                    self.assertTrue(point_in_domain(p["tilt_db_oct"], v["tilt_db_oct"]))
                self.assertTrue({"hp", "lp", "peaks", "tilt_db_oct"} & set(p), "an EQ stage draws at least a component")


class Plans(unittest.TestCase):
    def setUp(self) -> None:
        self.ps = load_pools()

    def test_a_plan_is_a_pure_function_of_its_inputs(self) -> None:
        for pool in ("train", "heldout"):
            for profile in (None, "all", "reverb"):
                self.assertEqual(make_plan(self.ps, pool, 5, profile), make_plan(self.ps, pool, 5, profile))
        self.assertNotEqual(make_plan(self.ps, "train", 5, "all"), make_plan(self.ps, "train", 6, "all"))
        self.assertNotEqual(make_plan(self.ps, "train", 5, "all"), make_plan(self.ps, "heldout", 5, "all"))

    def test_profiles_force_exactly_their_stages_and_restrict_variants(self) -> None:
        for pool in ("train", "heldout"):
            p = self.ps.pool(pool)
            for name, prof in p.profiles.items():
                plan = make_plan(self.ps, pool, 3, name)
                on = [e["stage"] for e in plan["stages"] if e["active"]]
                self.assertEqual(on, [s for s in STAGE_ORDER if s in prof.stages], name)
                for e in plan["stages"]:
                    if e["active"] and e["stage"] in prof.variants:
                        self.assertIn(e["variant"].split("/")[2], prof.variants[e["stage"]])
            allplan = make_plan(self.ps, pool, 3, ALL_PROFILE)
            self.assertTrue(all(e["active"] for e in allplan["stages"]))
        with self.assertRaises(PoolError):
            make_plan(self.ps, "heldout", 1, "no-such-profile")
        with self.assertRaises(PoolError):
            make_plan(self.ps, "validation", 1)

    def test_mixed_plans_follow_the_stage_probabilities(self) -> None:
        n = 3000
        for pool_name in ("train", "heldout"):
            pool = self.ps.pool(pool_name)
            clean = 0
            active = dict.fromkeys(STAGE_ORDER, 0)
            for seed in range(n):
                plan = make_plan(self.ps, pool_name, seed)
                clean += plan["clean"]
                self.assertTrue(plan["clean"] or any(e["active"] for e in plan["stages"]))
                for e in plan["stages"]:
                    active[e["stage"]] += e["active"]
            self.assertAlmostEqual(clean / n, pool.p_clean, delta=0.025)
            for stage, count in active.items():
                expected = (1 - pool.p_clean) * pool.stages[stage].p
                self.assertAlmostEqual(count / n, expected, delta=0.04, msg=f"{pool_name}/{stage}")

    def test_the_rng_and_the_seed_derivation_are_stable(self) -> None:
        # These constants pin the streams; changing them changes every seeded result (bump CHAIN_VERSION if you must).
        self.assertNotEqual(derive("plan", 1, "train"), derive("plan", 1, "heldout"))
        r = Rng(1)
        self.assertEqual([r._next() for _ in range(2)], [10451216379200822465, 13757245211066428519])
        self.assertAlmostEqual(Rng(7).random(), 0.3898297483912715, places=15)
        self.assertEqual(derive("enth"), 7251154863657285297)  # first 8 bytes of SHA-256("enth")


if __name__ == "__main__":
    unittest.main()
