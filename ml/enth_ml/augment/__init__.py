"""Augmentation and the data manifest (lane AUG; pre-declared empty by lane S0s, R3).

The training-time chain over ``ds1``'s clean renders (docs/r3.md, "Augmentation and licences"), 10 to 20 % of samples
left clean, and the manifest that records each source's licence. Shipped weights train only on our renders plus CC0, CC
BY or Apache data; CC BY-NC, CC BY-SA and licence-less sets are evaluation-only and stay local.

It builds on lane DEG's degradation chain (``enth_ml.degrade``), whose pools it draws from untouched, and adds what needs
recordings or a second clip:

* ``manifest`` / ``manifest.toml``: the data manifest, one entry per source (licence, URL, version, hash, allowed use) and
  the check that refuses a non-allowlisted licence for shipped training.
* ``sources``: the hashed index of each collection on disk, the train/dev split and GR3's exclusion lists.
* ``config`` / ``augment.toml``: the versioned augmentation config (stages, variants, profiles, label policy) and its checks.
* ``plan``: the merged, seeded plan (``make_plan``) and the per-sample seed (``sample_seed``).
* ``rir`` / ``stages`` / ``chain``: recorded room impulse responses, recorded background noises, microphone colour, gain
  staging, sample rates, a second sound under the clip, and ``augment_file`` which runs it all and writes the record.

``python -m enth_ml.augment --help`` lists the command line. Standard library only, plus the system ``ffmpeg`` binary.
"""

from .chain import augment_file
from .config import AugmentConfig, ConfigError, check_config, load_config
from .manifest import LicenceError, Manifest, check_manifest, load_manifest, require_shipped
from .plan import Availability, PlanError, make_plan, sample_seed
from .sources import SourceStore, load_exclusions

__all__ = [
    "AugmentConfig",
    "Availability",
    "ConfigError",
    "LicenceError",
    "Manifest",
    "PlanError",
    "SourceStore",
    "augment_file",
    "check_config",
    "check_manifest",
    "load_config",
    "load_exclusions",
    "load_manifest",
    "make_plan",
    "require_shipped",
    "sample_seed",
]
