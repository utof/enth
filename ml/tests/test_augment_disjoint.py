"""Lane AUG: augmentation never meets what GR3 holds out. Extends lane DEG's disjointness test to this package.

Acceptance (docs/phase-3.md, AUG): the augmentation sources and settings are disjoint from GR3's held-out set, as DEG's pools
are. Four layers, each tested here:

1. settings: every variant of ``augment.toml`` differs from every related held-out DEG variant (DEG's own rule), and the
   EQ-like and band-limit-like ones by a *number* in a parameter both define (the stricter rule below);
2. the held-out augmentation config (``augment-eval.toml``) differs from the training one in the same strict way;
3. sources: no training config reaches a collection the manifest holds out, and the held-out config reaches nothing else;
4. GR3's exclusion lists: no training plan shows a held-out pool, variant, profile or seed, and the instance seeds keep
   the pools' parity.

Standard library only; no ffmpeg, no data files.
"""

from __future__ import annotations

import unittest

from augment_support import config_text, recorded_stage
from degrade_support import ML  # noqa: F401  (puts ml/ on sys.path)
from enth_ml.augment import manifest as mf
from enth_ml.augment.config import AUG_STAGES, CONFIG_PATH, SLOTS, check_config, load_config, parse_config
from enth_ml.augment.plan import make_plan, sample_seed
from enth_ml.augment.sources import GR3_SEED_RANGES, load_exclusions
from enth_ml.degrade import check_disjoint, load_pools
from enth_ml.degrade import make_plan as deg_plan
from enth_ml.degrade.pools import Variant, domain, domains_meet, variants_disjoint

TRAIN = load_config(CONFIG_PATH)
EVAL = load_config(CONFIG_PATH.with_name("augment-eval.toml"))
POOLS = load_pools()
MANIFEST = mf.load_manifest()
EXCL = load_exclusions()
# The DEG stage each of ours is a relative of (what a held-out variant of it is meant to be unlike).
RELATED = {"rir": "room", "mic": "eq", "bg_noise": "noise", "rate": "band_limit"}
# Where the difference must be numeric, not just a different generator: both define the same kind of parameter.
STRICT = {"mic": "eq", "rate": "band_limit"}


def _numeric(d: tuple) -> bool:
    return d[0] == "range" or all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in d[1])


def numeric_shared(a: Variant, b: Variant) -> list[str]:
    """Parameters both variants define as numbers or sets of numbers (``kind`` and other names do not count)."""
    return [
        k for k in sorted(set(a.params) & set(b.params))
        if k != "kind" and _numeric(domain(a.params[k])) and _numeric(domain(b.params[k]))
    ]


def strictly_apart(a: Variant, b: Variant) -> bool:
    """True when some shared numeric parameter's domains do not meet (``kind`` is not enough)."""
    return any(not domains_meet(domain(a.params[k]), domain(b.params[k])) for k in numeric_shared(a, b))


class Settings(unittest.TestCase):
    def test_deg_s_own_pools_are_still_disjoint(self) -> None:
        self.assertEqual(check_disjoint(POOLS.pool("train"), POOLS.pool("heldout")), [])

    def test_every_training_variant_differs_from_every_related_held_out_variant_by_deg_s_rule(self) -> None:
        held = POOLS.pool("heldout")
        for aug, deg in RELATED.items():
            for v in TRAIN.stages[aug].variants.values():
                for h in held.stages[deg].variants.values():
                    with self.subTest(variant=f"{aug}/{v.name}", held_out=f"{deg}/{h.name}"):
                        self.assertTrue(variants_disjoint(v, h))
                        self.assertNotEqual(v.name, h.name)
                        self.assertNotEqual(v.kind, h.kind)

    def test_microphones_and_rates_differ_from_the_held_out_eq_and_band_limit_by_a_number(self) -> None:
        held = POOLS.pool("heldout")
        compared = 0
        for aug, deg in STRICT.items():
            for v in TRAIN.stages[aug].variants.values():
                for h in held.stages[deg].variants.values():
                    shared = numeric_shared(v, h)
                    if not shared:
                        continue  # a tilt has nothing a microphone defines: different families, different generators
                    compared += 1
                    with self.subTest(variant=f"{aug}/{v.name}", held_out=f"{deg}/{h.name}", shared=shared):
                        self.assertTrue(strictly_apart(v, h))
        self.assertGreaterEqual(compared, 4 + 2, "four microphones against two EQ shapes, the rates against the narrow band")

    def test_the_microphone_high_passes_lie_below_the_held_out_speakers_and_the_rates_outside_the_narrow_band(self) -> None:
        held = POOLS.pool("heldout")
        speaker_hp = min(domain(h.params["hp_hz"])[1] for h in held.stages["eq"].variants.values() if "hp_hz" in h.params)
        for v in TRAIN.stages["mic"].variants.values():
            self.assertLess(domain(v.params["hp_hz"])[2], speaker_hp, v.name)
        narrow = held.stages["band_limit"].variants["resample-narrow"].params["sr_hz"]["choice"]
        for v in TRAIN.stages["rate"].variants.values():
            self.assertFalse(set(v.params["sr_hz"]["choice"]) & set(narrow))
            self.assertGreater(min(v.params["sr_hz"]["choice"]), max(narrow))

    def test_the_strict_rule_has_teeth(self) -> None:
        a = Variant("a", 1.0, {"kind": "x", "hp_hz": [100.0, 300.0]})
        same_kind_name_other_range = Variant("b", 1.0, {"kind": "y", "hp_hz": [250.0, 900.0]})
        self.assertTrue(variants_disjoint(a, same_kind_name_other_range), "DEG's rule accepts a different kind")
        self.assertFalse(strictly_apart(a, same_kind_name_other_range), "ours also wants the numbers apart")
        self.assertTrue(strictly_apart(a, Variant("c", 1.0, {"kind": "y", "hp_hz": [310.0, 900.0]})))
        self.assertEqual(numeric_shared(a, Variant("d", 1.0, {"kind": "z", "color": "pink"})), [])

    def test_the_held_out_config_differs_from_the_training_one_in_a_number_stage_by_stage(self) -> None:
        compared = 0
        for stage, edef in EVAL.stages.items():
            for tv in TRAIN.stages[stage].variants.values():
                for ev in edef.variants.values():
                    shared = numeric_shared(tv, ev)
                    self.assertTrue(shared, f"{stage}: nothing numeric to compare {tv.name} with {ev.name}")
                    compared += 1
                    with self.subTest(stage=stage, train=tv.name, held_out=ev.name, shared=shared):
                        self.assertTrue(strictly_apart(tv, ev))
        self.assertGreaterEqual(compared, len(EVAL.stages))

    def test_the_held_out_microphone_is_above_every_deg_training_eq_s_high_pass(self) -> None:
        # Training's device EQ high-passes 80-400 Hz; the held-out headset starts above it.
        device = POOLS.pool("train").stages["eq"].variants["device"].params["hp_hz"]
        for v in EVAL.stages["mic"].variants.values():
            self.assertGreater(domain(v.params["hp_hz"])[1], domain(device)[2])

    def test_no_stage_of_ours_has_a_synthetic_instance_so_no_instance_seed_can_collide(self) -> None:
        for cfg in (TRAIN, EVAL):
            for sdef in cfg.stages.values():
                for v in sdef.variants.values():
                    self.assertFalse(any("seed" in k for k in v.params), f"{v.name}")


class Sources(unittest.TestCase):
    def test_no_training_variant_reads_a_held_out_collection(self) -> None:
        held = {r for r in MANIFEST.all_refs() if MANIFEST.role_of(r) == "heldout-eval"}
        self.assertTrue({"demand", "mit-ir-survey", "esc-50", "synthetic-heldout"} <= held)
        self.assertFalse(set(TRAIN.sources()) & held)
        self.assertTrue(set(EVAL.sources()) <= held)
        self.assertFalse(set(TRAIN.sources()) & set(EVAL.sources()))

    def test_a_training_config_cannot_be_pointed_at_a_held_out_collection(self) -> None:
        pools = POOLS
        for purpose in ("shipped-training", "research"):
            for ref in sorted(r for r in MANIFEST.all_refs() if MANIFEST.role_of(r) == "heldout-eval"):
                with self.subTest(purpose=purpose, ref=ref):
                    stage = recorded_stage("bg_noise", ref) + '[profile.p]\nstages = ["bg_noise"]\n'
                    self.assertTrue(check_config(parse_config(config_text(purpose=purpose, stages=stage)), MANIFEST, pools))

    def test_no_held_out_collection_is_among_the_shipped_references(self) -> None:
        for ref in MANIFEST.shipped_refs():
            self.assertNotEqual(MANIFEST.role_of(ref), "heldout-eval", ref)

    def test_a_held_out_config_cannot_be_pointed_at_a_training_collection(self) -> None:
        for ref in sorted(MANIFEST.shipped_refs()):
            if MANIFEST.role_of(ref) == "heldout-eval":
                continue
            with self.subTest(ref=ref):
                stage = recorded_stage("bg_noise", ref) + '[profile.p]\nstages = ["bg_noise"]\n'
                self.assertTrue(check_config(parse_config(config_text(purpose="eval", stages=stage)), MANIFEST, POOLS))


class GR3Lists(unittest.TestCase):
    """What content/gr3/exclude-*.txt promise about training, held against the plans this package makes."""

    def test_the_lists_name_the_heldout_pool_and_the_gr3_block(self) -> None:
        self.assertIn("heldout", EXCL.values("degrade-pool"))
        self.assertTrue(EXCL.values("degrade-variant"))
        self.assertEqual(max(hi for _, hi in EXCL.seed_ranges()), 770999999999)

    def test_no_training_plan_shows_a_held_out_pool_variant_or_profile(self) -> None:
        banned_variants = set(EXCL.values("degrade-variant"))
        banned_profiles = set(EXCL.values("degrade-profile"))
        banned_pools = set(EXCL.values("degrade-pool"))
        names = [None, "all", *TRAIN.profiles, *POOLS.pool("train").profiles]
        seen_variants = 0
        for profile in names:
            for seed in range(0 if profile is None else 40, 1200 if profile is None else 41):
                plan = make_plan(TRAIN, POOLS, seed, profile)
                self.assertNotIn(plan["pool"], banned_pools)
                self.assertNotIn(f"{plan['pool']}/profile/{profile}", banned_profiles)
                for e in plan["stages"]:
                    if e["active"]:
                        seen_variants += 1
                        self.assertNotIn(e["variant"], banned_variants, f"seed {seed} profile {profile}")
                        self.assertNotIn(e["variant"].split("/")[0], banned_pools)
        self.assertGreater(seen_variants, 3000)

    def test_the_held_out_config_does_show_them(self) -> None:
        # The check above is not vacuous: the held-out composition draws exactly what the lists name.
        banned = set(EXCL.values("degrade-variant"))
        shown = {e["variant"] for s in range(300) for e in make_plan(EVAL, POOLS, s)["stages"] if e["active"] and e["variant"] in banned}
        self.assertGreater(len(shown), 10)

    def test_no_training_plan_uses_a_seed_in_the_reserved_ranges(self) -> None:
        for lo, hi in EXCL.seed_ranges():
            self.assertFalse(any(lo <= sample_seed(3, f"s{i}", i % 7) <= hi for i in range(5000)))
        for lo, hi in GR3_SEED_RANGES:
            with self.assertRaises(ValueError):
                make_plan(TRAIN, POOLS, lo + 12345)

    def test_instance_seeds_keep_each_pool_s_parity(self) -> None:
        for cfg, parity in ((TRAIN, 0), (EVAL, 1)):
            seen = 0
            for seed in range(400):
                for e in make_plan(cfg, POOLS, seed, "all")["stages"]:
                    if e["active"] and "seed" in e.get("params", {}):
                        seen += 1
                        self.assertEqual(e["params"]["seed"] % 2, parity, f"{cfg.name} seed {seed} {e['stage']}")
            self.assertGreater(seen, 100)

    def test_deg_s_instances_come_from_deg_s_streams_untouched(self) -> None:
        for seed in range(100):
            ours = {e["stage"]: e for e in make_plan(TRAIN, POOLS, seed, "all")["stages"]}
            for e in deg_plan(POOLS, "train", seed, "all")["stages"]:
                self.assertEqual(ours[e["stage"]], e)

    def test_the_slots_are_the_stages_the_pools_define(self) -> None:
        for aug, deg in SLOTS.items():
            self.assertIn(aug, AUG_STAGES)
            self.assertIn(deg, POOLS.pool("train").stages)
            self.assertIn(deg, POOLS.pool("heldout").stages)


if __name__ == "__main__":
    unittest.main()
