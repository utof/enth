"""Lane AUG: the merged plan is a pure function of its inputs, leaves DEG's draws alone, and keeps its rules.

Standard library only; no ffmpeg, no data files.
"""

from __future__ import annotations

import unittest
from unittest import mock

from augment_support import config_text
from degrade_support import ML  # noqa: F401  (puts ml/ on sys.path)
from enth_ml.augment.config import AUG_STAGES, CONFIG_PATH, MERGED_ORDER, SLOTS, load_config, parse_config
from enth_ml.augment.plan import (
    PARTITIONS,
    SEED_LIMIT,
    Availability,
    PlanError,
    make_plan,
    plan_digest,
    sample_seed,
)
from enth_ml.augment.sources import GR3_SEED_RANGES, seed_in_gr3_block
from enth_ml.degrade import load_pools
from enth_ml.degrade import make_plan as deg_plan
from enth_ml.degrade.pools import STAGE_ORDER, domain, domains_meet

CFG = load_config(CONFIG_PATH)
EVAL = load_config(CONFIG_PATH.with_name("augment-eval.toml"))
POOLS = load_pools()
SEEDS = range(3000)


def active(plan) -> set[str]:
    return {e["stage"] for e in plan["stages"] if e["active"]}


def inside(value, spec) -> bool:
    return domains_meet(domain(value), domain(spec))


class Purity(unittest.TestCase):
    def test_the_same_inputs_give_the_same_plan_and_other_seeds_other_plans(self) -> None:
        for seed in (0, 1, 7, 12345):
            self.assertEqual(make_plan(CFG, POOLS, seed), make_plan(CFG, POOLS, seed))
        digests = {plan_digest(make_plan(CFG, POOLS, s)) for s in range(300)}
        self.assertGreater(len(digests), 280)

    def test_a_golden_plan_pins_the_streams(self) -> None:
        # An inline config, so editing augment.toml never moves it: if this fails the draws themselves moved (this
        # package's streams or DEG's). Bump the config version and say so in the record of any dataset built before.
        cfg = parse_config(config_text(stages=GAIN + RATE + MIC))
        self.assertEqual(
            [plan_digest(make_plan(cfg, POOLS, s)) for s in (0, 1, 2)],
            ["5f36bf339fe848a5", "e330ba2d398bc7d7", "731e00c6c8c92d54"],
        )
        self.assertEqual(plan_digest(make_plan(cfg, POOLS, 0, "all")), "96f22fd8d98f98e3")

    def test_the_plan_is_plain_data_in_the_merged_order(self) -> None:
        import json

        for seed in range(40):
            plan = make_plan(CFG, POOLS, seed)
            json.dumps(plan)
            self.assertEqual([e["stage"] for e in plan["stages"]], list(MERGED_ORDER))

    def test_deg_s_draws_are_untouched(self) -> None:
        # Every degradation entry is exactly DEG's, except one that a recorded stage replaced (switched off, and said so).
        for seed in SEEDS:
            plan, ref = make_plan(CFG, POOLS, seed), deg_plan(POOLS, "train", seed)
            self.assertEqual(
                (plan["clean"], plan["forced_stage"], plan["target_lufs"]),
                (ref["clean"], ref["forced_stage"], ref["target_lufs"]),
            )
            ours = {e["stage"]: e for e in plan["stages"]}
            for e in ref["stages"]:
                got = ours[e["stage"]]
                if "replaced_by" in got:
                    self.assertTrue(e["active"])
                    self.assertEqual(got, {"stage": e["stage"], "active": False, "replaced_by": SLOTS_INV[e["stage"]]})
                else:
                    self.assertEqual(got, e, f"seed {seed} stage {e['stage']}")

    def test_deg_profiles_pass_through(self) -> None:
        for prof in ("mp3", "reverb", "phone-recording"):
            plan, ref = make_plan(CFG, POOLS, 5, prof), deg_plan(POOLS, "train", 5, prof)
            self.assertEqual([e for e in plan["stages"] if e["stage"] in STAGE_ORDER], ref["stages"])
            self.assertEqual(active(plan) & set(AUG_STAGES), set())

    def test_a_stage_s_numbers_do_not_move_when_another_stage_changes(self) -> None:
        base = parse_config(config_text(stages=GAIN + RATE + MIC))
        more = parse_config(config_text(stages=GAIN + RATE + MIC.replace("[stage.mic.variant.a]", "[stage.mic.variant.zz]\nkind = \"x\"\nhp_hz = [1.0, 2.0]\n[stage.mic.variant.a]")))
        for seed in range(200):
            a, b = make_plan(base, POOLS, seed, "all"), make_plan(more, POOLS, seed, "all")
            ea, eb = {e["stage"]: e for e in a["stages"]}, {e["stage"]: e for e in b["stages"]}
            self.assertEqual(ea["gain"], eb["gain"])
            self.assertEqual(ea["rate"], eb["rate"])


SLOTS_INV = {deg: aug for aug, deg in SLOTS.items()}
GAIN = '[stage.gain]\np = 0.5\n[stage.gain.variant.v]\nkind = "level"\npeak_dbfs = [-45.0, -3.0]\n'
RATE = '[stage.rate]\np = 0.5\n[stage.rate.variant.v]\nkind = "native-rate"\nsr_hz = { choice = [8000, 16000] }\n'
MIC = (
    '[stage.mic]\np = 0.5\n[stage.mic.variant.a]\nkind = "mic-a"\nhp_hz = [30.0, 90.0]\nprox_hz = [100.0, 200.0]\n'
    "prox_db = [0.0, 3.0]\npeak_hz = [3000.0, 8000.0]\npeak_gain_db = [1.0, 5.0]\npeak_q = [0.7, 1.5]\n"
    "air_hz = [8000.0, 12000.0]\nair_db = [0.0, 4.0]\nlp_hz = [14000.0, 20000.0]\n"
)


class Rates(unittest.TestCase):
    """The chances the config states hold in the plans (large samples, loose bounds: these catch a wrong wiring)."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.plans = [make_plan(CFG, POOLS, s) for s in range(6000)]
        cls.refs = [deg_plan(POOLS, "train", s) for s in range(6000)]

    def share(self, pred) -> float:
        return sum(1 for p in self.plans if pred(p)) / len(self.plans)

    def test_the_clean_share_is_deg_s(self) -> None:
        self.assertAlmostEqual(self.share(lambda p: p["clean"]), 0.15, delta=0.02)
        for p in self.plans:
            if p["clean"]:
                self.assertEqual(active(p), set())

    def test_the_overall_chance_of_a_room_or_noise_stays_deg_s(self) -> None:
        for aug, deg in SLOTS.items():
            ours = self.share(lambda p, a=aug, d=deg: bool({a, d} & active(p)))
            theirs = sum(1 for r in self.refs if any(e["stage"] == deg and e["active"] for e in r["stages"])) / len(self.refs)
            self.assertEqual(ours, theirs, deg)

    def test_the_recorded_share_of_those_is_the_stage_s_p(self) -> None:
        for aug, deg in SLOTS.items():
            n_deg = sum(1 for r in self.refs if any(e["stage"] == deg and e["active"] for e in r["stages"]))
            n_aug = sum(1 for p in self.plans if aug in active(p))
            self.assertAlmostEqual(n_aug / n_deg, CFG.stages[aug].p, delta=0.04, msg=aug)

    def test_a_recorded_stage_never_stacks_on_the_synthetic_one_in_a_random_plan(self) -> None:
        for p in self.plans:
            for aug, deg in SLOTS.items():
                self.assertFalse({aug, deg} <= active(p))

    def test_the_independent_stages_fire_at_their_p_among_the_non_clean(self) -> None:
        keep = [p for p in self.plans if not p["clean"]]
        for stage in ("mix_in", "mic", "gain", "rate"):
            share = sum(1 for p in keep if stage in active(p)) / len(keep)
            self.assertAlmostEqual(share, CFG.stages[stage].p, delta=0.03, msg=stage)

    def test_variants_follow_their_weights(self) -> None:
        names = [e["variant"].rsplit("/", 1)[1] for p in self.plans for e in p["stages"] if e["stage"] == "rir" and e["active"]]
        total = sum(v.weight for v in CFG.stages["rir"].variants.values())
        for n, v in CFG.stages["rir"].variants.items():
            self.assertAlmostEqual(names.count(n) / len(names), v.weight / total, delta=0.05, msg=n)

    def test_every_drawn_parameter_lies_in_its_declared_domain(self) -> None:
        for p in self.plans:
            for e in p["stages"]:
                if not e["active"] or e["stage"] not in AUG_STAGES:
                    continue
                _, stage, name = e["variant"].split("/")[1:]
                spec = CFG.stages[stage].variants[name].params
                for k, v in e["params"].items():
                    if k.startswith("u_"):
                        self.assertTrue(0.0 <= v < 1.0, f"{stage}.{k}={v}")
                    else:
                        self.assertTrue(inside(v, spec[k]), f"{stage}/{name}.{k}={v} outside {spec[k]}")
                self.assertEqual(set(e["params"]) - set(spec), {k for k in e["params"] if k.startswith("u_")})

    def test_the_label_names_exactly_the_engine_s_own_effects_that_ran(self) -> None:
        saw_wet = saw_damage = False
        for p in self.plans[:2000]:
            expect = [
                e["stage"] for e in p["stages"]
                if e["active"] and (e["stage"] in ("room", "rir") or (e["stage"] == "dynamics" and e["params"]["kind"] == "compressor"))
            ]
            self.assertEqual([w["stage"] for w in p["label"]["wet_candidates"]], expect)
            self.assertEqual(p["label"]["policy"], "dry-and-wet" if expect else "damage")
            saw_wet |= bool(expect)
            saw_damage |= not expect
        self.assertTrue(saw_wet and saw_damage)


class Availability_(unittest.TestCase):
    def test_a_variant_whose_source_is_missing_is_dropped_and_listed(self) -> None:
        only_small = Availability(lambda ref: ref == "openslr-28/sim-small", layers=False)
        seen = set()
        for seed in SEEDS:
            plan = make_plan(CFG, POOLS, seed, availability=only_small)
            for e in plan["stages"]:
                if e["active"] and e["stage"] == "rir":
                    seen.add(e["variant"])
            self.assertNotIn("bg_noise", active(plan))
            self.assertNotIn("mix_in", active(plan))
        self.assertEqual(seen, {"augment/train-v1/rir/sim-small"})
        plan = make_plan(CFG, POOLS, 0, availability=only_small)
        self.assertIn("bg_noise/pointsource", plan["unavailable"])
        self.assertIn("rir/but-reverbdb", plan["unavailable"])
        self.assertIn("mix_in/layer", plan["unavailable"])
        self.assertIn("mix_in/note-layer", plan["unavailable"])

    def test_with_the_recorded_stage_unavailable_deg_s_synthetic_one_stays(self) -> None:
        none = Availability(lambda ref: False)
        for seed in SEEDS:
            plan, ref = make_plan(CFG, POOLS, seed, availability=none), deg_plan(POOLS, "train", seed)
            self.assertEqual([e for e in plan["stages"] if e["stage"] in STAGE_ORDER], ref["stages"])

    def test_layers_make_the_layer_variant_available(self) -> None:
        with_layers = Availability(lambda ref: False, layers=True)
        kinds = set()
        for seed in SEEDS:
            for e in make_plan(CFG, POOLS, seed, availability=with_layers)["stages"]:
                if e["stage"] == "mix_in" and e["active"]:
                    kinds.add(e["variant"])
        self.assertEqual(kinds, {"augment/train-v1/mix_in/layer"})

    def test_the_plan_does_not_depend_on_the_disk_beyond_what_is_dropped(self) -> None:
        # A plan with everything available and one with part of it differ only where a dropped variant was drawn.
        full, part = Availability(), Availability(lambda ref: ref != "but-reverbdb/rirs")
        for seed in range(500):
            a, b = make_plan(CFG, POOLS, seed, availability=full), make_plan(CFG, POOLS, seed, availability=part)
            self.assertEqual(active(a), active(b))
            for ea, eb in zip(a["stages"], b["stages"], strict=True):
                if ea["stage"] != "rir":
                    self.assertEqual(ea, eb)


class Profiles(unittest.TestCase):
    def test_each_aug_profile_forces_its_stages_and_nothing_else(self) -> None:
        for name, prof in CFG.profiles.items():
            plan = make_plan(CFG, POOLS, 3, name)
            want = set(prof.stages)
            if prof.deg:
                want |= {e["stage"] for e in deg_plan(POOLS, "train", 3, prof.deg)["stages"] if e["active"]}
            self.assertEqual(active(plan), want, name)
            self.assertEqual(plan["profile"], name)
            self.assertFalse(plan["clean"])

    def test_all_runs_every_stage_of_both_chains_with_the_recorded_ones_stacked(self) -> None:
        plan = make_plan(CFG, POOLS, 9, "all")
        self.assertEqual(active(plan), set(MERGED_ORDER))

    def test_all_skips_what_cannot_run_but_a_named_profile_refuses(self) -> None:
        none = Availability(lambda ref: False, layers=False)
        plan = make_plan(CFG, POOLS, 9, "all", availability=none)
        self.assertEqual(active(plan), set(STAGE_ORDER) | {"mic", "gain", "rate"})
        for name in ("recorded-room", "recorded-noise", "layer"):
            with self.subTest(profile=name), self.assertRaisesRegex(PlanError, "none of its variants can run"):
                make_plan(CFG, POOLS, 9, name, availability=none)
        make_plan(CFG, POOLS, 9, "mic", availability=none)

    def test_an_unknown_profile_is_refused_naming_the_choices(self) -> None:
        with self.assertRaisesRegex(PlanError, "no profile 'nope'.*recorded-room"):
            make_plan(CFG, POOLS, 1, "nope")

    def test_the_eval_config_composes_with_the_heldout_pool(self) -> None:
        for seed in range(50):
            plan = make_plan(EVAL, POOLS, seed)
            ref = deg_plan(POOLS, "heldout", seed)
            self.assertEqual(plan["pool"], "heldout")
            self.assertEqual(plan["partition"], "all")
            self.assertFalse(plan["clean"])  # the heldout pool has p_clean 0
            for e in plan["stages"]:
                if e["stage"] in STAGE_ORDER and "replaced_by" not in e:
                    self.assertEqual(e, next(r for r in ref["stages"] if r["stage"] == e["stage"]))


class Seeds(unittest.TestCase):
    def test_a_seed_inside_gr3_s_block_is_refused(self) -> None:
        for lo, hi in GR3_SEED_RANGES:
            for s in (lo, (lo + hi) // 2, hi):
                with self.subTest(seed=s), self.assertRaisesRegex(PlanError, "GR3"):
                    make_plan(CFG, POOLS, s)
            make_plan(CFG, POOLS, lo - 1)
            make_plan(CFG, POOLS, hi + 1)

    def test_seeds_out_of_range_or_of_the_wrong_type_are_refused(self) -> None:
        for s in (-1, SEED_LIMIT, 1.5, "7", True, None):
            with self.subTest(seed=s), self.assertRaises(PlanError):
                make_plan(CFG, POOLS, s)  # type: ignore[arg-type]
        make_plan(CFG, POOLS, SEED_LIMIT - 1)

    def test_sample_seeds_are_stable_in_range_and_never_in_the_block(self) -> None:
        self.assertEqual(sample_seed(1, "a", 0), sample_seed(1, "a", 0))
        self.assertEqual(sample_seed(1, "a"), sample_seed(1, "a", 0))
        self.assertEqual(sample_seed(1, "a", 0), 6843454643477805539)  # pinned: a change moves every dataset
        seen = set()
        for i in range(20000):
            s = sample_seed(7, f"sample-{i}", i % 5)
            self.assertTrue(0 <= s < SEED_LIMIT)
            self.assertFalse(seed_in_gr3_block(s))
            seen.add(s)
        self.assertEqual(len(seen), 20000)
        self.assertNotEqual(sample_seed(1, "a", 0), sample_seed(1, "a", 1), "a new epoch is a new augmentation")
        self.assertNotEqual(sample_seed(1, "a", 0), sample_seed(2, "a", 0))

    def test_a_hash_that_lands_in_the_block_is_hashed_again(self) -> None:
        inside_block = GR3_SEED_RANGES[0][0] + 5
        calls = []

        def fake(*parts):
            calls.append(parts)
            return inside_block if len(calls) == 1 else 12345

        with mock.patch("enth_ml.augment.plan.derive", fake):
            self.assertEqual(sample_seed(1, "x", 0), 12345)
        self.assertEqual(len(calls), 2)

    def test_partitions_are_validated(self) -> None:
        for part in PARTITIONS:
            self.assertEqual(make_plan(CFG, POOLS, 1, partition=part)["partition"], part)
        with self.assertRaises(PlanError):
            make_plan(CFG, POOLS, 1, partition="test")


if __name__ == "__main__":
    unittest.main()
