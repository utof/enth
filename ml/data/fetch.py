#!/usr/bin/env python3
"""Fetch one dataset archive that ``enth_ml/augment/manifest.toml`` names, by hand, politely, and check it.

This is the only network code in ``ml/``: the ``enth_ml`` package never imports it, no test run or training run starts it,
and it runs only when a person (or an agent the owner told to) types its name. Standard library only.

What it does, in order, for ``python3 ml/data/fetch.py REF --yes``:

1. **Gate.** Refuses a source the manifest excludes, a source none of whose parts may train shipped weights unless
   ``--research`` is given (it says why), an archive over 3 GB unless ``--allow-large``, a source whose archive is not a
   single file URL (FSD50K, Freesound: fetched by hand), and anything without ``--yes`` (which says "I read the licence
   and terms the manifest cites"; the script prints them first).
2. **robots.txt.** Fetches the host's ``/robots.txt`` first (RFC 9309 rules, own parser: wildcards, ``$``, longest match),
   obeys ``Disallow`` for the product token ``enth-research`` (or ``*``) and any ``Crawl-delay``, and re-checks every
   redirect target. A robots.txt that cannot be read (anything but 200, 404 or 410) stops the run.
3. **One polite request.** ``User-Agent: enth-research/0.1`` and no other identifying header (no email, name, cookie or
   referer); one connection; at least 2 s between requests to a host; resumes a ``.part`` file with ``Range``; no retry
   after 403, 404, 429 or a 5xx (it reports and stops).
4. **Check.** Size against the manifest, SHA-256 against the manifest. An empty hash (first fetch) prints the digest and stops
   before extracting, so that it is pinned in the manifest first (``--trust-first-fetch`` extracts anyway).
5. **Extract** into a fresh ``<data root>/<extract_to>.partial`` and rename it on success: no absolute path, no ``..``, no
   link or device is ever created, member names are checked before a byte is written, total size and file count are
   capped. Nothing is deleted by this script.
6. **Index** every part (``relpath<TAB>sha256<TAB>bytes``) and print each digest next to the one the manifest pins.

``--list`` shows what could be fetched; ``--verify REF`` re-hashes an archive and its indexes without any network access.
Downloaded and extracted files are untrusted data: this script never opens an extracted file; run anything that reads them
with ``python3 -I`` and from another directory.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import shutil
import socket
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

ML = Path(__file__).resolve().parents[1]
if str(ML) not in sys.path:
    sys.path.insert(0, str(ML))

from enth_ml.augment import manifest as mf  # noqa: E402
from enth_ml.augment.sources import SourceStore, default_data_root  # noqa: E402

USER_AGENT = "enth-research/0.1"
PRODUCT_TOKEN = "enth-research"
MIN_GAP_S = 2.0
LARGE_BYTES = 3_000_000_000
MAX_REDIRECTS = 3
CHUNK = 1 << 20
TIMEOUT_S = 60
RETRIES = 3
RETRY_WAIT_S = 30.0
MAX_EXPAND = 8  # extracted bytes may be at most this many times the archive's
MAX_FILES = 500_000


class FetchError(RuntimeError):
    """Refused or failed; the message says what to do."""


# --- robots.txt (RFC 9309) ------------------------------------------------------------------------------------------


class Robots:
    """One host's robots.txt: ``allowed(path)`` and ``crawl_delay`` for the product token."""

    def __init__(self, text: str, token: str = PRODUCT_TOKEN) -> None:
        groups: list[tuple[list[str], list[tuple[bool, str]], float | None]] = []
        agents: list[str] = []
        rules: list[tuple[bool, str]] = []
        delay: float | None = None
        in_rules = False
        for raw in text.splitlines():
            line = raw.split("#", 1)[0].strip()
            if ":" not in line:
                continue
            key, _, value = line.partition(":")
            key, value = key.strip().lower(), value.strip()
            if key == "user-agent":
                if in_rules:
                    groups.append((agents, rules, delay))
                    agents, rules, delay, in_rules = [], [], None, False
                agents.append(value.lower())
            elif key in ("allow", "disallow"):
                in_rules = True
                if value:  # an empty Disallow allows everything: no rule
                    rules.append((key == "allow", value))
            elif key == "crawl-delay":
                in_rules = True
                try:
                    delay = float(value)
                except ValueError:
                    pass
        if agents:
            groups.append((agents, rules, delay))
        mine = [g for g in groups if any(a != "*" and a in token.lower() for a in g[0])]
        chosen = mine or [g for g in groups if "*" in g[0]]
        self.rules = [r for g in chosen for r in g[1]]
        delays = [g[2] for g in chosen if g[2] is not None]
        self.crawl_delay = max(delays) if delays else None

    @staticmethod
    def _matches(pattern: str, path: str) -> bool:
        regex = "".join(".*" if c == "*" else "$" if c == "$" else re.escape(c) for c in pattern)
        if pattern.endswith("$"):
            regex = regex[:-1] + r"\Z"
        return re.match(regex, path) is not None

    def allowed(self, path: str) -> bool:
        best: tuple[int, bool] | None = None
        for allow, pattern in self.rules:
            if self._matches(pattern, path):
                cand = (len(pattern), allow)
                if best is None or cand[0] > best[0] or (cand[0] == best[0] and allow):
                    best = cand
        return True if best is None else best[1]


# --- one polite client ----------------------------------------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):  # type: ignore[override]
        return None


class Client:
    """Sequential requests with the fixed headers, a gap per host, robots checks and manual redirects."""

    def __init__(self, sleep=time.sleep, clock=time.monotonic, say=print) -> None:
        self.opener = urllib.request.build_opener(_NoRedirect)
        self.sleep, self.clock, self.say = sleep, clock, say
        self.last: dict[str, float] = {}
        self.robots: dict[str, Robots] = {}
        self.requests = 0

    def _wait(self, host: str, extra: float) -> None:
        gap = max(MIN_GAP_S, extra)
        now = self.clock()
        if host in self.last and now - self.last[host] < gap:
            self.sleep(gap - (now - self.last[host]))
        self.last[host] = self.clock()

    def _get(self, url: str, headers: dict[str, str] | None = None):
        host = urllib.parse.urlsplit(url).netloc
        extra = self.robots[host].crawl_delay or 0.0 if host in self.robots else 0.0
        self._wait(host, extra)
        self.requests += 1
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, **(headers or {})})
        return self.opener.open(req, timeout=TIMEOUT_S)

    def load_robots(self, url: str) -> Robots:
        parts = urllib.parse.urlsplit(url)
        host = parts.netloc
        if host in self.robots:
            return self.robots[host]
        robots_url = f"{parts.scheme}://{host}/robots.txt"
        text = ""
        try:
            for _ in range(MAX_REDIRECTS + 1):
                try:
                    with self._get(robots_url) as resp:
                        text = resp.read(1 << 20).decode("utf-8", "replace")
                    break
                except urllib.error.HTTPError as e:
                    if e.code in (301, 302, 303, 307, 308) and e.headers.get("Location"):
                        robots_url = urllib.parse.urljoin(robots_url, e.headers["Location"])
                        continue
                    if e.code in (404, 410):
                        text = ""
                        break
                    raise FetchError(
                        f"{robots_url}: HTTP {e.code}; robots.txt cannot be read, so nothing is fetched from {host}. "
                        "Check the site's terms by hand."
                    ) from e
        except (urllib.error.URLError, socket.timeout, OSError) as e:
            raise FetchError(f"{robots_url}: cannot be read ({e}); nothing is fetched from {host}") from e
        self.robots[host] = Robots(text)
        self.say(f"robots.txt of {host}: {len(self.robots[host].rules)} rule(s) for {PRODUCT_TOKEN} or *"
                 + (f", crawl-delay {self.robots[host].crawl_delay:g} s" if self.robots[host].crawl_delay else ""))
        return self.robots[host]

    def check_allowed(self, url: str) -> None:
        parts = urllib.parse.urlsplit(url)
        path = parts.path + (f"?{parts.query}" if parts.query else "")
        if not self.load_robots(url).allowed(path or "/"):
            raise FetchError(f"robots.txt of {parts.netloc} disallows {path} for {PRODUCT_TOKEN}: not fetched")

    def open(self, url: str, headers: dict[str, str] | None = None):
        """GET ``url`` (robots checked for it and for each redirect target) and return the open response."""
        for _ in range(MAX_REDIRECTS + 1):
            self.check_allowed(url)
            try:
                return self._get(url, headers)
            except urllib.error.HTTPError as e:
                if e.code in (301, 302, 303, 307, 308) and e.headers.get("Location"):
                    url = urllib.parse.urljoin(url, e.headers["Location"])
                    self.say(f"redirected to {urllib.parse.urlsplit(url).netloc}")
                    continue
                if e.code == 416:
                    raise
                retry = e.headers.get("Retry-After")
                raise FetchError(f"{url}: HTTP {e.code}" + (f" (Retry-After {retry})" if retry else "") + "; not retried") from e
        raise FetchError(f"{url}: more than {MAX_REDIRECTS} redirects")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(CHUNK), b""):
            h.update(block)
    return h.hexdigest()


def download(client: Client, url: str, dest: Path, expected_bytes: int, say=print) -> None:
    """Fetch ``url`` into ``dest`` through ``dest.part`` (resumed if it exists); the size must match ``expected_bytes``."""
    part = dest.with_name(dest.name + ".part")
    dest.parent.mkdir(parents=True, exist_ok=True)
    for attempt in range(1, RETRIES + 1):
        have = part.stat().st_size if part.exists() else 0
        if have > expected_bytes > 0:
            raise FetchError(f"{part} is larger than the archive should be; move it away and run again")
        try:
            headers = {"Range": f"bytes={have}-"} if have else None
            try:
                resp = client.open(url, headers)
            except urllib.error.HTTPError as e:  # 416: the .part may already be whole
                if e.code == 416 and have == expected_bytes:
                    break
                raise FetchError(f"{url}: HTTP 416 for a resume from {have}; move {part.name} away and run again") from e
            with resp:
                status = resp.status
                start = have
                if have and status != 206:
                    say("the server does not resume: starting again")
                    start = 0
                elif have:
                    m = re.match(r"bytes (\d+)-", resp.headers.get("Content-Range", ""))
                    if not m or int(m.group(1)) != have:
                        raise FetchError("the server resumed at another offset: move the .part away and run again")
                total = expected_bytes
                done, mark = start, start
                with open(part, "ab" if start else "wb") as f:
                    while True:
                        block = resp.read(CHUNK)
                        if not block:
                            break
                        f.write(block)
                        done += len(block)
                        if total and done - mark >= max(total // 20, CHUNK):
                            mark = done
                            say(f"  {done / 1e6:,.0f} of {total / 1e6:,.0f} MB")
            break
        except (urllib.error.URLError, socket.timeout, ConnectionError, TimeoutError) as e:
            if attempt == RETRIES:
                raise FetchError(f"{url}: network error after {RETRIES} tries ({e}); the .part is kept for a resume") from e
            say(f"network error ({e}); waiting {RETRY_WAIT_S:g} s and resuming")
            client.sleep(RETRY_WAIT_S * attempt)
    size = part.stat().st_size
    if expected_bytes and size != expected_bytes:
        raise FetchError(f"{part.name}: {size} bytes, the manifest says {expected_bytes}; kept for inspection, not used")
    os.replace(part, dest)


# --- extraction ----------------------------------------------------------------------------------------------------


def _safe_relpath(name: str) -> PurePosixPath:
    if not name or "\x00" in name or "\\" in name or name.startswith("/") or re.match(r"^[A-Za-z]:", name):
        raise FetchError(f"archive member {name!r} has an unsafe name: nothing is extracted")
    p = PurePosixPath(name)
    parts = [x for x in p.parts if x not in ("", ".")]
    if not parts or ".." in parts:
        raise FetchError(f"archive member {name!r} climbs out of its directory: nothing is extracted")
    return PurePosixPath(*parts)


def _is_root_dir(name: str) -> bool:
    """A directory entry naming the archive's own root (``.``, ``./``): BUT ReverbDB's tarball carries one; it names nothing to extract."""
    return bool(name) and not name.startswith("/") and PurePosixPath(name).parts == ()


def safe_extract(archive: Path, dest: Path, max_bytes: int, max_files: int = MAX_FILES) -> dict[str, int]:
    """Extract ``archive`` (zip or tar) into the new directory ``dest``; returns counts of files, bytes and skipped members."""
    if dest.exists():
        raise FetchError(f"{dest} exists: move it away first (this script deletes nothing)")
    dest.mkdir(parents=True)
    files = total = skipped = 0

    def put(rel: PurePosixPath, reader) -> None:
        nonlocal files, total
        target = dest.joinpath(*rel.parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        files += 1
        if files > max_files:
            raise FetchError(f"more than {max_files} files in the archive: stopped")
        with open(target, "xb") as out:  # exclusive: never overwrites, never follows a link
            for block in iter(lambda: reader.read(CHUNK), b""):
                total += len(block)
                if total > max_bytes:
                    raise FetchError(f"the archive expands past {max_bytes / 1e9:.1f} GB: stopped (a bomb, or raise the cap)")
                out.write(block)
        os.chmod(target, 0o644)

    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as z:
            infos = z.infolist()
            infos = [i for i in infos if not (i.is_dir() and _is_root_dir(i.filename))]
            plan = [(i, _safe_relpath(i.filename.rstrip("/"))) for i in infos]  # every name checked before any byte is written
            for info, rel in plan:
                mode = (info.external_attr >> 16) & 0o170000
                if info.is_dir():
                    dest.joinpath(*rel.parts).mkdir(parents=True, exist_ok=True)
                elif mode not in (0, 0o100000):
                    skipped += 1  # a link, device or fifo
                else:
                    with z.open(info) as r:
                        put(rel, r)
    elif tarfile.is_tarfile(archive):
        with tarfile.open(archive, "r:*") as t:
            members = [m for m in t.getmembers() if not (m.isdir() and _is_root_dir(m.name))]
            plan = [(m, _safe_relpath(m.name.rstrip("/"))) for m in members]
            for m, rel in plan:
                if m.isdir():
                    dest.joinpath(*rel.parts).mkdir(parents=True, exist_ok=True)
                elif m.isreg():
                    r = t.extractfile(m)
                    if r is None:
                        skipped += 1
                        continue
                    with r:
                        put(rel, r)
                else:
                    skipped += 1
    else:
        raise FetchError(f"{archive.name} is neither a zip nor a tar archive")
    return {"files": files, "bytes": total, "skipped_members": skipped}


# --- the manifest side ---------------------------------------------------------------------------------------------


def source_uses(m: mf.Manifest, src: mf.Source) -> set[str]:
    return {m.effective_use(f"{src.id}/{p}") for p in src.parts} if src.parts else {m.effective_use(src.id)}


def gate(m: mf.Manifest, ref: str, *, research: bool, allow_large: bool, yes: bool, say=print) -> mf.Source:
    if "/" in ref or ref not in m.sources:
        raise FetchError(f"{ref!r} is not a source of the manifest; the sources are {sorted(m.sources)}")
    src = m.sources[ref]
    arc = src.archive
    uses = source_uses(m, src)
    if uses == {"excluded"}:
        raise FetchError(f"{ref} is excluded in the manifest ({(src.reason or src.licence).rstrip('. ')}); it is never fetched")
    if "shipped-training" not in uses and not research:
        raise FetchError(
            f"{ref} is {sorted(uses)} ({src.licence}): {(src.reason or 'see the manifest').rstrip('. ')}. It may be fetched for research "
            "or evaluation only; add --research if that is what is wanted"
        )
    if arc is None or arc.bytes <= 0 or not arc.local or not re.match(r"^https?://", arc.url):
        raise FetchError(f"{ref} has no single archive URL in the manifest: fetch it by hand from {src.url} (see its note)")
    if arc.bytes > LARGE_BYTES and not allow_large:
        raise FetchError(f"{ref}: {arc.bytes / 1e9:.1f} GB is over the {LARGE_BYTES / 1e9:.0f} GB budget; add --allow-large once it is agreed")
    say(f"{ref}: {src.name}")
    say(f"  licence {src.licence} (read on {src.licence_read_on} at {src.licence_url})")
    say(f"  archive {arc.url}, {arc.bytes:,} bytes, {'sha256 pinned' if arc.sha256 else 'sha256 NOT pinned yet'}")
    if not yes:
        raise FetchError("say --yes to confirm that the licence and the site's terms were read (they are cited above)")
    return src


def check_space(root: Path, need: int) -> None:
    root.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(root).free
    if free < need:
        raise FetchError(f"{root}: {free / 1e9:.1f} GB free, about {need / 1e9:.1f} GB needed")


def index_parts(m: mf.Manifest, store: SourceStore, src: mf.Source, say=print) -> int:
    bad = 0
    for pid, part in src.parts.items():
        ref = f"{src.id}/{pid}"
        path, digest, count = store.write_index(ref)
        pinned = part.index_sha256
        status = "not pinned yet" if not pinned else "matches the manifest" if pinned == digest else "DIFFERS from the manifest"
        bad += bool(pinned and pinned != digest)
        say(f"  index {ref}: {count} files, {digest} ({status}) -> {path}")
    return bad


def fetch(m: mf.Manifest, ref: str, root: Path, *, research=False, allow_large=False, yes=False, trust_first=False,
          client: Client | None = None, say=print) -> int:
    src = gate(m, ref, research=research, allow_large=allow_large, yes=yes, say=say)
    arc = src.archive
    assert arc is not None
    check_space(root, int(arc.bytes * 3.5))
    dest = root / arc.local
    client = client or Client(say=say)
    if dest.exists() and dest.stat().st_size == arc.bytes:
        say(f"{dest.name} is already there")
    else:
        download(client, arc.url, dest, arc.bytes, say)
    digest = sha256_file(dest)
    say(f"  sha256 {digest}")
    if arc.sha256 and digest != arc.sha256:
        raise FetchError(f"{dest.name} does not match the SHA-256 in the manifest; it is kept for inspection and not extracted")
    if not arc.sha256 and not trust_first:
        say("The manifest has no hash for this archive yet: put the digest above in manifest.toml (`sha256 = ...`, "
            "`fetched = true`) and run again to extract it (or add --trust-first-fetch).")
        return 0
    if not arc.extract_to:
        return 0
    target = root / arc.extract_to
    if target.exists() and any(target.iterdir()):
        say(f"{target} already holds files: not extracting again")
    else:
        partial = target.with_name(target.name + ".partial")
        stats = safe_extract(dest, partial, max_bytes=min(MAX_EXPAND * arc.bytes, 60_000_000_000))
        if target.exists():
            target.rmdir()  # only an empty directory can be here
        os.replace(partial, target)
        say(f"  extracted {stats['files']} files, {stats['bytes'] / 1e6:,.0f} MB to {target}" + (
            f" ({stats['skipped_members']} link/device members skipped)" if stats["skipped_members"] else ""))
    store = SourceStore(m, data_root=root)
    return 1 if index_parts(m, store, src, say) else 0


def verify(m: mf.Manifest, ref: str, root: Path, say=print) -> int:
    if ref not in m.sources or m.sources[ref].archive is None:
        raise FetchError(f"{ref!r} has no archive in the manifest")
    src = m.sources[ref]
    arc = src.archive
    assert arc is not None
    status = 0
    path = root / arc.local if arc.local else None
    if path is not None and path.is_file():
        digest = sha256_file(path)
        ok = path.stat().st_size == arc.bytes and (not arc.sha256 or digest == arc.sha256)
        say(f"{path}: {path.stat().st_size:,} bytes, sha256 {digest} ({'ok' if ok else 'DIFFERS from the manifest'})")
        status |= 0 if ok else 1
    else:
        say(f"archive {arc.local or '(none)'} is not in {root} (fine once it is extracted and indexed)")
    store = SourceStore(m, data_root=root)
    for pid, part in src.parts.items():
        entries = store.index(f"{src.id}/{pid}")
        from enth_ml.augment.sources import index_digest

        digest = index_digest(entries)
        ok = bool(entries) and (not part.index_sha256 or digest == part.index_sha256) and (not part.files or len(entries) == part.files)
        say(f"index {src.id}/{pid}: {len(entries)} files, {digest} ({'ok' if ok else 'DIFFERS or empty'})")
        status |= 0 if ok else 1
    return status


def listing(m: mf.Manifest, root: Path, say=print) -> None:
    say(f"data root {root}")
    for sid, src in m.sources.items():
        arc = src.archive
        if arc is None:
            continue
        uses = ",".join(sorted(source_uses(m, src)))
        scripted = arc.bytes > 0 and bool(arc.local) and arc.url.startswith("http")
        say(f"  {sid:14s} {uses:34s} {src.licence:18s} {arc.bytes / 1e9:7.2f} GB  "
            f"{'fetched' if arc.fetched else 'not fetched'}  {'scripted' if scripted else 'by hand'}  {arc.url}")


def main(argv: list[str] | None = None, manifest: mf.Manifest | None = None, client: Client | None = None) -> int:
    ap = argparse.ArgumentParser(description="Fetch one dataset archive named in the augmentation data manifest.")
    ap.add_argument("ref", nargs="?", help="a source id of manifest.toml (e.g. openslr-28)")
    ap.add_argument("--data-dir", type=Path, help="where the data goes (default: $ENTH_DATA_DIR or the manifest's local_root)")
    ap.add_argument("--list", action="store_true", help="list the sources that have an archive and exit")
    ap.add_argument("--verify", action="store_true", help="re-hash REF's archive and indexes (no network) and exit")
    ap.add_argument("--yes", action="store_true", help="confirm that the licence and the site's terms were read")
    ap.add_argument("--research", action="store_true", help="allow a source that may not train shipped weights (research or evaluation use)")
    ap.add_argument("--allow-large", action="store_true", help=f"allow an archive over {LARGE_BYTES / 1e9:.0f} GB")
    ap.add_argument("--trust-first-fetch", action="store_true", help="extract an archive the manifest has no hash for")
    args = ap.parse_args(argv)
    m = manifest or mf.load_manifest()
    root = args.data_dir or default_data_root(m)
    try:
        if args.list:
            listing(m, root)
            return 0
        if not args.ref:
            ap.error("name a source (or use --list)")
        if args.verify:
            return verify(m, args.ref, root)
        return fetch(m, args.ref, root, research=args.research, allow_large=args.allow_large, yes=args.yes,
                     trust_first=args.trust_first_fetch, client=client)
    except (FetchError, mf.ManifestError) as e:
        print(f"fetch: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
