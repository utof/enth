#!/usr/bin/env python3
"""Check GR3's frozen held-out sets (content/gr3, lane DS): hashes, sizes, rules, disjointness, exclusions.

Standard library only (Python 3.11+, for tomllib). Run from anywhere:

    python3 content/gr3/check.py                      # offline: hashes, sizes, every rule re-derived
    python3 content/gr3/check.py --enth path/to/enth  # also re-derive every code and fingerprint (no render)
    python3 content/gr3/check.py --enth path/to/enth --reselect   # and re-run the whole seed selection (6-10 min)
    python3 content/gr3/check.py --deg ml             # also recompute every degraded plan's hash (lane DEG's chain)
    python3 content/gr3/check.py --audio ~/code/enth-scratch/r3-data/freesound   # and the reference audio

Exit status 0 when every check passes, 1 otherwise. The README ("Checking the freeze") explains each check.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent

FROZEN = ["synth.toml", "real.toml", "degraded.toml", "exclude-synth.txt", "exclude-real.txt", "exclude-degraded.txt"]
SYNTH_PROFILES = ["mp3", "noise-hum", "reverb", "clip-limit"]
REAL_PROFILES = ["mp3", "upload-noisy", "upload-room", "upload-loud", "phone-recording"]
STRATA = ["sub", "fm", "ac", "owner"]
LICENCES = {"CC0-1.0", "CC-BY-3.0", "CC-BY-4.0"}
EXCLUDE_TYPES = {
    "exclude-synth.txt": {"seed-range", "stream", "code", "fingerprint", "preset-stem", "preset-name", "file-sha256", "path"},
    "exclude-real.txt": {"freesound-id", "freesound-pack", "freesound-uploader", "audio-sha256", "path"},
    "exclude-degraded.txt": {"degrade-pool", "degrade-variant", "degrade-profile", "degrade-seed-range", "pools-sha256"},
}

failures: list[str] = []


def check(ok: bool, what: str) -> bool:
    if not ok:
        failures.append(what)
    return ok


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    return sha256_bytes(p.read_bytes())


def hkey(salt: str, s: str) -> str:
    return sha256_bytes(f"{salt}:{s}".encode())


# --- code v1 (docs/challenge.md, "Codes v1"): Crockford base32, a GF(32) check symbol --------------------------
ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def _mul_alpha(x: int) -> int:
    x <<= 1
    if x & 0b100000:
        x ^= 0b100101  # x^5 + x^2 + 1
    return x & 0b11111


def short_code(preset: int, generator: int, seed: int) -> str:
    """The short form: form 0, version 1, preset (8 bits), generator (12), seed (40); 13 symbols and a check."""
    value = (0 << 64) | (1 << 60) | (preset << 52) | (generator << 40) | seed
    data = [(value >> (60 - 5 * i)) & 31 for i in range(13)]
    acc = 0
    for s in data:
        acc = _mul_alpha(acc) ^ s
    syms = "".join(ALPHABET[s] for s in data) + ALPHABET[_mul_alpha(acc)]
    return "-".join(syms[i : i + 4] for i in range(0, len(syms), 4))


# --- loading ---------------------------------------------------------------------------------------------------
def load_toml(name: str) -> dict:
    with open(HERE / name, "rb") as f:
        return tomllib.load(f)


def load_exclusions(name: str) -> dict[str, set[str]]:
    out: dict[str, set[str]] = {}
    for n, line in enumerate((HERE / name).read_text(encoding="utf-8").splitlines(), start=1):
        body = line.split("#", 1)[0].strip()
        if not body:
            continue
        kind, _, value = body.partition(" ")
        value = value.strip()
        check(kind in EXCLUDE_TYPES[name] and value != "", f"{name}:{n}: unknown entry type or empty value: {line!r}")
        out.setdefault(kind, set()).add(value)
    return out


# --- checks ----------------------------------------------------------------------------------------------------
def check_hashes() -> None:
    listed: dict[str, str] = {}
    for line in (HERE / "hashes.txt").read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        digest, _, name = line.partition("  ")
        listed[name] = digest
    for name, digest in listed.items():
        p = HERE / name
        if check(p.is_file(), f"hashes.txt names a missing file: {name}"):
            check(sha256_file(p) == digest, f"hash mismatch: {name} (the freeze was broken, or hashes.txt is stale)")
    expected = set(FROZEN) | {f"presets/{p.name}" for p in (HERE / "presets").glob("*.patch.json")}
    check(set(listed) == expected, f"hashes.txt must list exactly the frozen files; missing {sorted(expected - set(listed))}, extra {sorted(set(listed) - expected)}")
    for name in FROZEN + ["hashes.txt"]:
        check(b"\r" not in (HERE / name).read_bytes(), f"{name} has CR bytes (LF only, so hashes match everywhere)")


def check_synth(synth: dict, excl: dict[str, set[str]]) -> list[dict]:
    check(synth.get("schema") == "enth.gr3-synth/1", "synth.toml: schema")
    targets = synth["target"]
    ids = [t["id"] for t in targets]
    check(len(ids) == len(set(ids)), "synth.toml: duplicate target ids")
    test = [t for t in targets if t["split"] == "test"]
    dev = [t for t in targets if t["split"] == "dev"]
    gen_test = [t for t in test if t["kind"] == "generator"]
    gen_dev = [t for t in dev if t["kind"] == "generator"]
    sizes = synth["sizes"]
    check(len(gen_test) == sizes["test_generator"] == 92, "synth.toml: 92 generator test targets")
    check([sum(1 for t in gen_test if t["level"] == lv) for lv in range(1, 6)] == [12, 20, 20, 20, 20], "synth.toml: test targets per level 12/20/20/20/20")
    check(sum(1 for t in test if t["kind"] == "preset") == sizes["test_hand_made"] == 22, "synth.toml: 22 hand-made test renders")
    check(sum(1 for t in test if t["kind"] == "owner-g1") == sizes["test_owner_g1"] == 8, "synth.toml: 8 owner G1 placeholders")
    check(len(test) == sizes["test_total"] == 122, "synth.toml: 122 test targets")
    check(len(dev) == sizes["dev"] == 30, "synth.toml: 30 dev targets")
    check(test == targets[: len(test)], "synth.toml: test targets come first (degraded seeds follow their order)")

    # seeds and codes
    lo, hi = synth["generator"]["block"]
    for t in gen_test + gen_dev:
        off = 0 if t["split"] == "test" else 500_000
        rel = t["seed"] - lo - off
        check(lo <= t["seed"] <= hi and rel // 10_000 == t["level"] and 1 <= rel % 10_000 <= 500, f"{t['id']}: seed outside its formula")
        check(short_code(t["level"], 1, t["seed"]) == t["code"], f"{t['id']}: code does not encode level {t['level']}, generator 1, seed {t['seed']}")
        check(len(t["fingerprint"]) == 64, f"{t['id']}: fingerprint")
    for field in ("seed", "code", "fingerprint"):
        a = {t[field] for t in gen_test}
        b = {t[field] for t in gen_dev}
        check(len(a) == len(gen_test) and len(b) == len(gen_dev) and not (a & b), f"synth.toml: {field}s not unique or test and dev overlap")
    # ids follow the selection order (k increasing within a level and split)
    for split, group in (("t", gen_test), ("d", gen_dev)):
        for lv in range(1, 6):
            row = [t for t in group if t["level"] == lv]
            check([t["id"] for t in row] == [f"gen-{split}-l{lv}-{i:02d}" for i in range(1, len(row) + 1)], f"generator ids, split {split} level {lv}")
            check([t["seed"] for t in row] == sorted(t["seed"] for t in row), f"generator seeds increase, split {split} level {lv}")

    # the preset split, re-derived from content/presets-candidates
    cands = []
    for p in sorted((REPO / "content/presets-candidates").glob("*/*.patch.json")):
        stem = p.name[: -len(".patch.json")]
        cands.append((p.parent.name, hkey("gr3/preset-split/v1", stem), stem, p))
    cands.sort()
    order = [c[2] for c in cands]
    ps = synth["preset_split"]
    check(len(order) == 22, "content/presets-candidates: 22 candidates")
    check(ps["order"] == order and ps["test"] == order[0::2] and ps["anchor"] == order[1::2], "synth.toml: the preset split does not follow its rule")
    check(not set(ps["test"]) & set(ps["anchor"]), "synth.toml: preset halves overlap")
    check(ps["dev_from_anchor"] == [order[i] for i in (1, 5, 9, 13, 17)], "synth.toml: dev presets are anchors at positions 1, 5, 9, 13, 17")
    cand_path = {c[2]: c[3] for c in cands}
    hand_test = [t for t in test if t["kind"] == "preset"]
    hand_dev = [t for t in dev if t["kind"] == "preset"]
    check([t["stem"] for t in hand_test][0::2] == ps["test"], "synth.toml: hand-made test targets are the test half, two notes each")
    check([t["stem"] for t in hand_dev][0::2] == ps["dev_from_anchor"], "synth.toml: hand-made dev targets are the dev anchors, two notes each")
    for t in hand_test:
        copy = HERE / "presets" / f"{t['stem']}.patch.json"
        check(t["file"] == f"content/gr3/presets/{t['stem']}.patch.json" and copy.is_file() and sha256_file(copy) == t["sha256"], f"{t['id']}: frozen preset copy")
        if sha256_file(cand_path[t["stem"]]) != t["sha256"]:
            print(f"note: {cand_path[t['stem']].relative_to(REPO)} changed since the freeze; GR3 uses its frozen copy")
    for t in hand_dev:
        if sha256_file(REPO / t["file"]) != t["sha256"]:
            print(f"note: dev preset {t['file']} changed since the freeze; dev runs use the frozen bytes (git show)")
    for a, b in zip(targets[0::1], targets[1::1]):
        if a["kind"] in ("preset", "owner-g1") and a["id"].endswith("-a") and b["id"] == a["id"][:-2] + "-b":
            pa, pb = a["perf"], b["perf"]
            check(pb == dict(pa, key=pa["key"] + 7, velocity=80) and pa["velocity"] == 100, f"{a['id']}: note b is not a fifth up at velocity 80")
    check({t["file"] for t in test if t["kind"] == "owner-g1"} == {f"content/presets/{g}.patch.json" for g in ("bass", "pad", "pluck", "lead")}, "synth.toml: G1 placeholders")

    # the core: ceil(n/3) per generator level and ceil(11/3) test presets (both notes), by hash
    core = set()
    for lv in range(1, 6):
        row = sorted((t["id"] for t in gen_test if t["level"] == lv), key=lambda i: hkey("gr3/core/v1", i))
        core.update(row[: math.ceil(len(row) / 3)])
    core_presets = set(sorted(ps["test"], key=lambda s: hkey("gr3/core/v1", s))[: math.ceil(len(ps["test"]) / 3)])
    core.update(t["id"] for t in hand_test if t["stem"] in core_presets)
    check({t["id"] for t in test if t["core"]} == core and len(core) == sizes["test_core"] == 40, "synth.toml: the core does not follow its rule")

    # exclusions
    check(f"{lo}..{hi}" in excl.get("seed-range", set()), "exclude-synth.txt: the GR3 seed block")
    check("r3/test" in excl.get("stream", set()) and "content/gr3/**" in excl.get("path", set()), "exclude-synth.txt: r3/test and content/gr3/**")
    for t in gen_test + gen_dev:
        check(t["code"] in excl.get("code", set()) and t["fingerprint"] in excl.get("fingerprint", set()), f"exclude-synth.txt misses {t['id']}")
    for t in hand_test:
        check(t["stem"] in excl.get("preset-stem", set()) and t["sha256"] in excl.get("file-sha256", set()), f"exclude-synth.txt misses preset {t['stem']}")
        check(t["preset"] in excl.get("preset-name", set()), f"exclude-synth.txt misses preset name {t['preset']}")
    for stem in ps["anchor"]:
        check(stem not in excl.get("preset-stem", set()), f"exclude-synth.txt excludes anchor {stem}")
    return test


def check_real(real: dict, excl: dict[str, set[str]]) -> dict[str, dict]:
    check(real.get("schema") == "enth.gr3-real/1", "real.toml: schema")
    refs = real["reference"]
    by_id = {r["id"]: r for r in refs}
    check(len(by_id) == len(refs), "real.toml: duplicate ids")
    test = [r for r in refs if r["role"] == "test"]
    for s in STRATA:
        row = [r for r in test if r["stratum"] == s]
        expect = [f"owner-{i:02d}" for i in range(1, 11)] if s == "owner" else [f"real-{s}-{i:02d}" for i in range(1, 11)]
        check([r["id"] for r in row] == expect, f"real.toml: 10 test references in stratum {s}")
    defaults = [r for r in refs if r["role"] == "default"]
    reserves = [r for r in refs if r["role"] == "reserve"]
    check(len(test) == real["sizes"]["clean"] == 40, "real.toml: 40 clean references")
    check(len(defaults) == real["sizes"]["defaults"] == 10, "real.toml: 10 owner defaults")
    check(len(reserves) == real["sizes"]["reserves"] == 8, "real.toml: 8 reserves")
    for i in range(1, 11):
        slot = by_id[f"owner-{i:02d}"]
        check(slot["default"] == f"owner-default-{i:02d}" and by_id[slot["default"]]["fills"] == slot["id"], f"owner-{i:02d}: default")
    fs = [r for r in refs if r["source"] == "freesound"]
    check(len(fs) == 48, "real.toml: 48 Freesound sounds")
    ids = [r["freesound_id"] for r in fs]
    check(len(ids) == len(set(ids)), "real.toml: a Freesound id appears twice")
    packs = [r["pack"]["id"] for r in fs if "pack" in r]
    check(len(packs) == len(set(packs)), "real.toml: two sounds from one pack")
    up: dict[str, int] = {}
    for r in fs:
        up[r["uploader"]] = up.get(r["uploader"], 0) + 1
        check(r["licence"] in LICENCES, f"{r['id']}: licence {r['licence']} is not CC0 or CC BY")
        check((r["licence"] == "CC0-1.0") == ("publicdomain/zero/1.0" in r["licence_url"]), f"{r['id']}: licence and its URL disagree")
        check(str(r["freesound_id"]) in r["page"] and str(r["freesound_id"]) in r["audio"]["url"], f"{r['id']}: page or audio URL names another sound")
        check(len(r["audio"]["sha256"]) == 64 and r["audio"]["bytes"] > 0, f"{r['id']}: audio hash")
        check(str(r["freesound_id"]) in excl.get("freesound-id", set()), f"exclude-real.txt misses {r['id']}")
        check(r["audio"]["sha256"] in excl.get("audio-sha256", set()), f"exclude-real.txt misses the audio of {r['id']}")
        if "pack" in r:
            check(str(r["pack"]["id"]) in excl.get("freesound-pack", set()), f"exclude-real.txt misses the pack of {r['id']}")
    check(max(up.values()) <= 2, "real.toml: more than 2 sounds from one uploader")
    check("~/code/enth-scratch/owner/r3-picks/**" in excl.get("path", set()), "exclude-real.txt: the owner's picks")
    return by_id


def check_degraded(deg: dict, test: list[dict], refs: dict[str, dict], excl: dict[str, set[str]]) -> None:
    check(deg.get("schema") == "enth.gr3-degraded/1", "degraded.toml: schema")
    synth = deg["items"]["synth"]
    expect = []
    for t_idx, t in enumerate(test, start=1):
        for p_idx, prof in enumerate(SYNTH_PROFILES, start=1):
            expect.append((f"deg-{t['id']}-{prof}", t["id"], prof, 770_100_000_000 + 10 * t_idx + p_idx, t["core"]))
    got = [(i["id"], i["target"], i["profile"], i["seed"], i["core"]) for i in synth]
    check(got == expect, "degraded.toml: synth items are not every test target x 4 profiles with their seeds")
    check(len(synth) == deg["sizes"]["synth"] == 488 and sum(i["core"] for i in synth) == deg["sizes"]["synth_core"] == 160, "degraded.toml: synth sizes")
    real = deg["items"]["real"]
    expect = []
    k = 0
    for s in STRATA:
        row = sorted((r["id"] for r in refs.values() if r["role"] == "test" and r["stratum"] == s), key=lambda i: hkey("gr3/real-degraded/v1", i))[:5]
        for prof, src in zip(REAL_PROFILES, row):
            k += 1
            expect.append((f"real-deg-{k:02d}", src, prof, 770_200_000_000 + k))
    check([(i["id"], i["source"], i["profile"], i["seed"]) for i in real] == expect, "degraded.toml: real items do not follow their rule")
    check(len(real) == deg["sizes"]["real"] == 20, "degraded.toml: 20 real items")
    linked = {r["id"]: r.get("degraded_copy") for r in refs.values() if r.get("degraded_copy")}
    check(linked == {i["source"]: i["id"] for i in real}, "real.toml's degraded_copy fields disagree with degraded.toml")
    for i in synth + real:
        check(len(i["plan_sha256"]) == 64, f"{i['id']}: plan hash")
    check(deg["chain"]["pool"] == "heldout" and "heldout" in excl.get("degrade-pool", set()), "exclude-degraded.txt: the heldout pool")
    check(deg["chain"]["pools_sha256"] in excl.get("pools-sha256", set()), "exclude-degraded.txt: the pools file hash")
    lo, hi = (int(x) for x in next(iter(excl.get("degrade-seed-range", {"0..0"}))).split(".."))
    check(all(lo <= i["seed"] <= hi for i in synth + real), "exclude-degraded.txt: the degradation seed range misses an item")
    for prof in set(SYNTH_PROFILES) | set(REAL_PROFILES):
        check(f"heldout/profile/{prof}" in excl.get("degrade-profile", set()), f"exclude-degraded.txt misses heldout/profile/{prof}")


# --- optional checks -------------------------------------------------------------------------------------------
def enth_json(enth: str, *args: str) -> dict:
    out = subprocess.run([enth, *args, "--format", "json"], capture_output=True, text=True)
    return json.loads(out.stdout)


def check_enth(enth: str, synth: dict) -> None:
    for t in synth["target"]:
        if t["kind"] != "generator":
            continue
        d = enth_json(enth, "challenge", "new", "--level", str(t["level"]), "--seed", str(t["seed"]))
        check(d.get("code") == t["code"] and d.get("fingerprint") == t["fingerprint"], f"{t['id']}: enth gives {d.get('code')} {d.get('fingerprint')}")
    print(f"enth: {sum(1 for t in synth['target'] if t['kind'] == 'generator')} generator targets re-derived")


def check_reselect(enth: str, synth: dict) -> None:
    """Re-run synth.toml's [generator] selection rule and compare the accepted seeds."""
    units = {}
    for line in (REPO / "crates/enth-challenge/src/random/catalogue-v1.txt").read_text().splitlines():
        parts = line.split()
        units[parts[0]] = parts[2].split("=")[1]

    def same(addr: str, a: float, b: float) -> bool:
        u = units[addr]
        if u == "None":
            return a == b
        if u == "Hz":
            return max(a, b) / min(a, b) < 1.5 if addr.startswith("lfo") else abs(12 * math.log2(a / b)) < 2
        if u == "Ms":
            return max(a + 1, b + 1) / min(a + 1, b + 1) < 1.5
        if u == "Percent":
            if addr.endswith(".level") or addr.endswith(".sustain"):
                if a < 0.1 or b < 0.1:
                    return a < 0.1 and b < 0.1
                return abs(20 * math.log10(a / b)) < 3
            return abs(a - b) < 10
        if u == "Db":
            return abs(a - b) < 3
        if u == "Semitones":
            return abs(a - b) < 1
        if u == "Cents":
            return abs(a - b) < 3
        if u == "Ratio":
            return max(a, b) / min(a, b) < 1.5
        raise ValueError(u)

    def summary(code: str) -> dict:
        d = enth_json(enth, "challenge", "show", code, "--reveal")
        t = d["reveal"]["target"]
        enabled = sorted(m["id"] for g in ("generators", "fx", "voice_fx") for m in t[g] if m.get("enabled", True))
        return {"phrase": d["phrase"]["id"], "enabled": enabled, "routes": sorted((r["src"], r["dst"], r["depth"]) for r in t["routes"]),
                "values": {v["addr"]: v["target"] for v in d["reveal"]["values"]}}

    def near(a: dict, b: dict) -> bool:
        if a["phrase"] != b["phrase"] or a["enabled"] != b["enabled"] or set(a["values"]) != set(b["values"]):
            return False
        if [r[:2] for r in a["routes"]] != [r[:2] for r in b["routes"]] or any(abs(x[2] - y[2]) >= 5 for x, y in zip(a["routes"], b["routes"])):
            return False
        return all(same(k, a["values"][k], b["values"][k]) for k in a["values"])

    taken = set()
    for line in (HERE / "exclude-synth.txt").read_text().splitlines():  # GR1's and the golden challenges' fingerprints
        if line.startswith("code ") and ("gr1" in line or "golden" in line):
            d = enth_json(enth, "challenge", "new", "--code", line.split()[1])
            if "fingerprint" in d:
                taken.add(d["fingerprint"])
    counts = {"test": [12, 20, 20, 20, 20], "dev": [4, 4, 5, 4, 5]}
    accepted = {lv: [] for lv in range(1, 6)}
    seeds = []
    for split, off in (("test", 0), ("dev", 500_000)):
        for lv in range(1, 6):
            got = 0
            for k in range(1, 501):
                if got == counts[split][lv - 1]:
                    break
                seed = synth["generator"]["block"][0] + off + 10_000 * lv + k
                d = enth_json(enth, "challenge", "new", "--level", str(lv), "--seed", str(seed))
                if d.get("error") == "silent_target" or d["fingerprint"] in taken:
                    continue
                s = summary(d["code"])
                if any(near(s, o) for o in accepted[lv]):
                    continue
                accepted[lv].append(s)
                taken.add(d["fingerprint"])
                seeds.append(seed)
                got += 1
    want = [t["seed"] for t in synth["target"] if t["kind"] == "generator"]
    check(sorted(seeds) == sorted(want), "reselect: the selection rule gives other seeds")
    print(f"reselect: {len(seeds)} seeds re-selected")


def check_deg(ml: str, deg: dict) -> None:
    sys.path.insert(0, str(Path(ml).resolve()))
    from enth_ml.degrade import chain, pools  # noqa: PLC0415

    ps = pools.load_pools()
    check(ps.sha256 == deg["chain"]["pools_sha256"], f"--deg: pools.toml's sha256 is {ps.sha256}, the freeze has {deg['chain']['pools_sha256']}")
    for i in deg["items"]["synth"] + deg["items"]["real"]:
        plan = chain.make_plan(ps, "heldout", i["seed"], i["profile"])
        h = sha256_bytes(json.dumps(plan, sort_keys=True, separators=(",", ":")).encode())
        check(h == i["plan_sha256"], f"{i['id']}: the plan's hash differs (DEG's chain or pools moved)")
    print(f"deg: {len(deg['items']['synth']) + len(deg['items']['real'])} plans recomputed")


def check_audio(root: str, real: dict) -> None:
    n = 0
    for r in real["reference"]:
        if r["source"] != "freesound":
            continue
        p = Path(os.path.expanduser(root)) / r["audio"]["file"]
        if check(p.is_file(), f"--audio: missing {p}"):
            check(sha256_file(p) == r["audio"]["sha256"] and p.stat().st_size == r["audio"]["bytes"], f"--audio: {p.name} differs from the manifest")
            n += 1
    print(f"audio: {n} files checked")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--enth", help="an enth build: re-derive every generator code and fingerprint (no render)")
    ap.add_argument("--reselect", action="store_true", help="with --enth: re-run the whole seed selection (6 to 10 minutes)")
    ap.add_argument("--deg", help="the ml/ directory with lane DEG's chain: recompute every degraded plan's hash")
    ap.add_argument("--audio", help="the directory holding the reference audio: check every file's SHA-256")
    args = ap.parse_args()

    check_hashes()
    excl = {name: load_exclusions(name) for name in EXCLUDE_TYPES}
    synth, real, deg = load_toml("synth.toml"), load_toml("real.toml"), load_toml("degraded.toml")
    test = check_synth(synth, excl["exclude-synth.txt"])
    refs = check_real(real, excl["exclude-real.txt"])
    check_degraded(deg, test, refs, excl["exclude-degraded.txt"])
    if args.enth:
        check_enth(args.enth, synth)
        if args.reselect:
            check_reselect(args.enth, synth)
    if args.deg:
        check_deg(args.deg, deg)
    if args.audio:
        check_audio(args.audio, real)

    if failures:
        for f in failures:
            print(f"FAIL {f}")
        print(f"gr3 check: {len(failures)} failure(s)")
        return 1
    print("gr3 check: OK (hashes match; sizes, rules, disjointness and exclusions hold)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
