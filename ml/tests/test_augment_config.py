"""Lane AUG: the augmentation configs are valid data, and a config can name only the sources its purpose allows.

Standard library only; no ffmpeg, no data files.
"""

from __future__ import annotations

import unittest

from augment_support import build_manifest, config_text, recorded_stage, source_block
from enth_ml.augment import manifest as mf
from enth_ml.augment.config import (
    AUG_STAGES,
    CONFIG_PATH,
    MERGED_ORDER,
    SCHEMA,
    SLOTS,
    ConfigError,
    check_config,
    load_config,
    parse_config,
)
from enth_ml.degrade import load_pools
from enth_ml.degrade.pools import STAGE_ORDER, domain

EVAL_PATH = CONFIG_PATH.with_name("augment-eval.toml")


class PackagedConfigs(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = mf.load_manifest()
        self.pools = load_pools()
        self.train = load_config(CONFIG_PATH)
        self.eval = load_config(EVAL_PATH)

    def test_the_two_files_load_and_pass_every_check(self) -> None:
        for cfg in (self.train, self.eval):
            with self.subTest(config=cfg.name):
                self.assertEqual(check_config(cfg, self.manifest, self.pools), [])
                self.assertRegex(cfg.sha256, r"^[0-9a-f]{64}$")
                self.assertGreaterEqual(cfg.version, 1)

    def test_training_composes_with_the_train_pool_and_evaluation_with_the_heldout_pool(self) -> None:
        self.assertEqual((self.train.purpose, self.train.degrade_pool), ("shipped-training", "train"))
        self.assertEqual((self.eval.purpose, self.eval.degrade_pool), ("eval", "heldout"))

    def test_the_merged_order_keeps_deg_s_order_and_holds_every_stage_once(self) -> None:
        self.assertEqual([s for s in MERGED_ORDER if s in STAGE_ORDER], list(STAGE_ORDER))
        self.assertEqual(len(set(MERGED_ORDER)), len(MERGED_ORDER))
        self.assertEqual(set(MERGED_ORDER), set(STAGE_ORDER) | set(AUG_STAGES))
        # A recorded stage runs after the synthetic stage it stands in for.
        for aug, deg in SLOTS.items():
            self.assertGreater(MERGED_ORDER.index(aug), MERGED_ORDER.index(deg))

    def test_every_variant_has_a_kind_and_a_probability_in_range(self) -> None:
        for cfg in (self.train, self.eval):
            for sname, sdef in cfg.stages.items():
                self.assertIn(sname, AUG_STAGES)
                self.assertTrue(0.0 <= sdef.p <= 1.0)
                for v in sdef.variants.values():
                    self.assertIn("kind", v.params, f"{cfg.name}/{sname}/{v.name}")
                    self.assertGreater(v.weight, 0.0)

    def test_the_recorded_variants_name_their_source_and_the_others_do_not(self) -> None:
        for cfg in (self.train, self.eval):
            for sname, sdef in cfg.stages.items():
                for v in sdef.variants.values():
                    recorded = v.kind in ("recorded", "layer-recorded")
                    self.assertEqual("source" in v.params, recorded, f"{cfg.name}/{sname}/{v.name}")

    def test_profile_names_do_not_collide_with_the_pools_profiles_or_all(self) -> None:
        for cfg in (self.train, self.eval):
            pool = self.pools.pool(cfg.degrade_pool)
            for name in cfg.profiles:
                self.assertNotIn(name, pool.profiles)
                self.assertNotEqual(name, "all")

    def test_every_profile_forces_stages_the_config_defines(self) -> None:
        for cfg in (self.train, self.eval):
            for p in cfg.profiles.values():
                self.assertTrue(p.stages)
                for s in p.stages:
                    self.assertIn(s, cfg.stages)

    def test_the_policy_marks_only_the_engine_s_own_effects_as_ambiguous(self) -> None:
        for cfg in (self.train, self.eval):
            self.assertEqual(cfg.policy_of("room"), "dry-and-wet")
            self.assertEqual(cfg.policy_of("rir"), "dry-and-wet")
            self.assertEqual(cfg.policy_of("dynamics", "compressor"), "dry-and-wet")
            for stage, kind in (("dynamics", "limiter"), ("dynamics", "hard-clip"), ("codec", None), ("noise", None),
                                ("eq", None), ("hum", None), ("mic", None), ("gain", None), ("mix_in", "layer")):
                self.assertEqual(cfg.policy_of(stage, kind), "damage", f"{stage}/{kind}")

    def test_a_training_config_names_nothing_held_out_and_an_eval_config_nothing_trainable(self) -> None:
        for ref in self.train.sources():
            self.assertNotEqual(self.manifest.role_of(ref), "heldout-eval", ref)
            self.assertEqual(self.manifest.effective_use(ref), "shipped-training", ref)
        for ref in self.eval.sources():
            self.assertEqual(self.manifest.role_of(ref), "heldout-eval", ref)
        self.assertFalse(set(self.train.sources()) & set(self.eval.sources()))

    def test_the_file_s_digest_is_that_of_its_bytes(self) -> None:
        import hashlib

        self.assertEqual(self.train.sha256, hashlib.sha256(CONFIG_PATH.read_bytes()).hexdigest())
        self.assertEqual(self.eval.sha256, hashlib.sha256(EVAL_PATH.read_bytes()).hexdigest())

    def test_the_fraction_parameters_are_not_drawn_from_the_config(self) -> None:
        # u_file, u_chan, u_start, u_layer are added by the plan, so the file must not define them.
        for cfg in (self.train, self.eval):
            for sdef in cfg.stages.values():
                for v in sdef.variants.values():
                    for k in v.params:
                        self.assertFalse(k.startswith("u_"), f"{v.name}.{k}")

    def test_all_parameter_domains_are_finite_and_ordered(self) -> None:
        for cfg in (self.train, self.eval):
            for sdef in cfg.stages.values():
                for v in sdef.variants.values():
                    for k, spec in v.params.items():
                        kind, lo, hi = domain(spec)
                        if kind == "range":
                            self.assertLessEqual(lo, hi, f"{v.name}.{k}")


class Refusals(unittest.TestCase):
    """Bad structure raises ConfigError; bad sources are reported by check_config."""

    def parse(self, **kw) -> object:
        return parse_config(config_text(**kw))

    def test_a_wrong_schema_or_purpose_is_refused(self) -> None:
        with self.assertRaises(ConfigError):
            parse_config(config_text().replace(SCHEMA, "enth.augment-config/2"))
        with self.assertRaises(ConfigError):
            self.parse(purpose="deploy")

    def test_purpose_and_pool_must_agree(self) -> None:
        for purpose, pool in (("shipped-training", "heldout"), ("research", "heldout"), ("eval", "train")):
            with self.subTest(purpose=purpose, pool=pool), self.assertRaises(ConfigError):
                self.parse(purpose=purpose, pool=pool)

    def test_unknown_or_malformed_stages_are_refused(self) -> None:
        bad = {
            "unknown stage": "[stage.reverb]\np = 0.5\n",
            "p above one": '[stage.gain]\np = 1.5\n[stage.gain.variant.v]\nkind = "level"\n',
            "no variants": "[stage.gain]\np = 0.5\n",
            "no kind": "[stage.gain]\np = 0.5\n[stage.gain.variant.v]\npeak_dbfs = [-9.0, -3.0]\n",
            "bad range": '[stage.gain]\np = 0.5\n[stage.gain.variant.v]\nkind = "level"\npeak_dbfs = [-3.0, -9.0]\n',
            "bad weight": '[stage.gain]\np = 0.5\n[stage.gain.variant.v]\nkind = "level"\nweight = 0\n',
        }
        for why, text in bad.items():
            with self.subTest(why), self.assertRaises(ConfigError):
                self.parse(stages=text)

    def test_profiles_must_name_defined_stages_and_not_shadow_all(self) -> None:
        stage = '[stage.gain]\np = 0.5\n[stage.gain.variant.v]\nkind = "level"\npeak_dbfs = [-9.0, -3.0]\n'
        for why, prof in {
            "unknown stage": '[profile.p]\nstages = ["rate"]\n',
            "empty": "[profile.p]\nstages = []\n",
            "shadows all": '[profile.all]\nstages = ["gain"]\n',
        }.items():
            with self.subTest(why), self.assertRaises(ConfigError):
                self.parse(stages=stage + prof)
        self.parse(stages=stage + '[profile.p]\nstages = ["gain"]\n')

    def test_the_label_policy_must_name_real_stages(self) -> None:
        with self.assertRaises(ConfigError):
            self.parse(extra='[label_policy]\ndry_and_wet = ["reverb"]\n')

    def test_a_shipped_config_may_name_only_shipped_training_sources(self) -> None:
        # The real manifest: every source or part whose use is not shipped-training is refused, by name.
        manifest = mf.load_manifest()
        pools = load_pools()
        refused = [r for r in manifest.all_refs() if manifest.effective_use(r) != "shipped-training"]
        self.assertTrue(refused)
        for ref in refused:
            with self.subTest(ref=ref):
                cfg = parse_config(config_text(stages=recorded_stage("bg_noise", ref)))
                problems = check_config(cfg, manifest, pools)
                self.assertTrue(any(ref in p for p in problems), problems)

    def test_a_shipped_config_may_name_every_shipped_training_source(self) -> None:
        manifest = mf.load_manifest()
        pools = load_pools()
        for ref in manifest.shipped_refs():
            if manifest.role_of(ref) == "heldout-eval":
                continue
            with self.subTest(ref=ref):
                cfg = parse_config(config_text(stages=recorded_stage("bg_noise", ref)))
                self.assertEqual(check_config(cfg, manifest, pools), [])

    def test_nc_sa_and_licence_less_sources_are_refused_whatever_the_manifest_says_about_use(self) -> None:
        # A manifest entry that claims shipped-training for an NC licence is itself invalid, so the config check
        # never gets the chance to accept it: check_manifest names it.
        for lic in ("CC-BY-NC-4.0", "CC-BY-SA-4.0", "CC-BY-NC-SA-3.0", "unknown", "research-only"):
            with self.subTest(licence=lic):
                m = build_manifest(source_block(id="x", licence=lic, use="shipped-training"))
                self.assertTrue(mf.check_manifest(m))
                cfg = parse_config(config_text(stages=recorded_stage("bg_noise", "x")))
                self.assertTrue(check_config(cfg, m), "a config naming it is refused as well")

    def test_a_source_the_manifest_does_not_know_is_refused(self) -> None:
        cfg = parse_config(config_text(stages=recorded_stage("bg_noise", "nowhere/part")))
        problems = check_config(cfg, mf.load_manifest())
        self.assertTrue(any("nowhere" in p for p in problems), problems)

    def test_a_research_config_may_name_research_only_sources_but_not_excluded_or_held_out_ones(self) -> None:
        m = mf.load_manifest()
        for ref, ok in (("musan/music", True), ("esc-50", False), ("wham-noise", False), ("demand", False),
                        ("openslr-28/real-air", True), ("openslr-28/sim-small", True)):
            with self.subTest(ref=ref):
                cfg = parse_config(config_text(purpose="research", stages=recorded_stage("bg_noise", ref)))
                self.assertEqual(check_config(cfg, m) == [], ok, check_config(cfg, m))

    def test_an_eval_config_may_name_held_out_collections_only(self) -> None:
        m = mf.load_manifest()
        for ref, ok in (("demand", True), ("mit-ir-survey", True), ("esc-50", True), ("wham-noise", False),
                        ("openair", False), ("openslr-28/sim-small", False), ("tinysol", False)):
            with self.subTest(ref=ref):
                cfg = parse_config(config_text(purpose="eval", stages=recorded_stage("bg_noise", ref)))
                self.assertEqual(check_config(cfg, m) == [], ok, check_config(cfg, m))

    def test_a_profile_may_not_reuse_a_deg_profile_name(self) -> None:
        stage = '[stage.gain]\np = 0.5\n[stage.gain.variant.v]\nkind = "level"\npeak_dbfs = [-9.0, -3.0]\n'
        cfg = parse_config(config_text(stages=stage + '[profile.mp3]\nstages = ["gain"]\n'))
        self.assertTrue(any("mp3" in p for p in check_config(cfg, mf.load_manifest(), load_pools())))
        cfg = parse_config(config_text(stages=stage + '[profile.p]\nstages = ["gain"]\ndeg = "no-such"\n'))
        self.assertTrue(any("no-such" in p for p in check_config(cfg, mf.load_manifest(), load_pools())))


if __name__ == "__main__":
    unittest.main()
