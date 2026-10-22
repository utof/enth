"""Lane AUG: ml/data/fetch.py is polite, checks what it gets, and extracts nothing it should not.

Standard library only. Every request goes to a server on 127.0.0.1 started by the test: nothing here touches the internet.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import os
import pathlib
import tarfile
import threading
import unittest
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from augment_support import ML, temp_dir
from enth_ml.augment import manifest as mf

spec = importlib.util.spec_from_file_location("enth_fetch", ML / "data" / "fetch.py")
fetch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fetch)


class Site:
    """A tiny web server: /robots.txt, /data/<name> with Range support, /go (redirect), and a log of what it was asked."""

    def __init__(self, files: dict[str, bytes], robots: str | None, redirects: dict[str, str] | None = None) -> None:
        self.files, self.robots, self.redirects = files, robots, redirects or {}
        self.log: list[tuple[str, dict[str, str]]] = []
        site = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a) -> None:  # silence
                pass

            def do_GET(self) -> None:
                site.log.append((self.path, {k.lower(): v for k, v in self.headers.items()}))
                if self.path == "/robots.txt":
                    if site.robots is None:
                        self.send_error(404)
                        return
                    body = site.robots.encode()
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                if self.path in site.redirects:
                    self.send_response(302)
                    self.send_header("Location", site.redirects[self.path])
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                name = self.path.removeprefix("/data/")
                if not self.path.startswith("/data/") or name not in site.files:
                    self.send_error(404)
                    return
                data = site.files[name]
                rng = self.headers.get("Range")
                if rng and rng.startswith("bytes="):
                    start = int(rng[6:].split("-")[0])
                    if start >= len(data):
                        self.send_error(416)
                        return
                    self.send_response(206)
                    self.send_header("Content-Range", f"bytes {start}-{len(data) - 1}/{len(data)}")
                    data = data[start:]
                else:
                    self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def __enter__(self) -> "Site":
        self.thread.start()
        return self

    def __exit__(self, *exc) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()

    def url(self, path: str) -> str:
        return f"http://127.0.0.1:{self.port}{path}"

    def asked(self, path: str) -> int:
        return sum(1 for p, _ in self.log if p == path)


def zip_bytes(members: dict[str, bytes], extra=None) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in members.items():
            z.writestr(name, data)
        if extra:
            extra(z)
    return buf.getvalue()


def manifest_for(url: str, data: bytes, *, sha: str | None = None, use: str = "shipped-training", parts: str = "",
                 bytes_: int | None = None, extra_src: str = "") -> mf.Manifest:
    digest = hashlib.sha256(data).hexdigest() if sha is None else sha
    text = f"""
schema = "enth.data-manifest/1"
version = 1
local_root = "/nonexistent"
[policy]
shipped_licences = ["CC0-1.0", "CC-BY-3.0", "CC-BY-4.0", "Apache-2.0"]
[[source]]
id = "test-set"
name = "a test set"
kind = "noise"
role = "augmentation"
url = "https://example.test/x"
version = "1"
licence = "CC0-1.0"
licence_url = "https://example.test/licence"
licence_read_on = "2026-10-22"
licence_evidence = "read on the page"
use = "{use}"
attribution = "x"
{extra_src}
[source.archive]
url = "{url}"
bytes = {len(data) if bytes_ is None else bytes_}
sha256 = "{digest}"
fetched = false
local = "dl/test.zip"
extract_to = "test-set"
{parts}
"""
    return mf.parse_manifest(text)


class FakeClock:
    def __init__(self) -> None:
        self.t = 1000.0
        self.slept: list[float] = []

    def sleep(self, s: float) -> None:
        self.slept.append(s)
        self.t += s

    def now(self) -> float:
        self.t += 0.001
        return self.t


class Case(unittest.TestCase):
    def setUp(self) -> None:
        self._td = temp_dir()
        self.dir = pathlib.Path(self._td.name)
        self.root = self.dir / "data"
        self.clock = FakeClock()
        self.out: list[str] = []

    def tearDown(self) -> None:
        self._td.cleanup()

    def client(self) -> fetch.Client:
        return fetch.Client(sleep=self.clock.sleep, clock=self.clock.now, say=self.out.append)

    def run_fetch(self, m: mf.Manifest, site: Site | None = None, *, client=None, **kw) -> int:
        kw.setdefault("yes", True)
        return fetch.fetch(m, "test-set", self.root, client=client or self.client(), say=self.out.append, **kw)


class Robots(unittest.TestCase):
    def test_longest_match_wildcards_and_groups(self) -> None:
        r = fetch.Robots("User-agent: *\nDisallow: /search/\nDisallow: /*/ci/\nAllow: /search/public/\nDisallow: /exact$\n"
                         "User-agent: GPTBot\nDisallow: /\n")
        self.assertTrue(r.allowed("/resources/28/x.zip"))
        self.assertFalse(r.allowed("/search/q"))
        self.assertTrue(r.allowed("/search/public/page"))
        self.assertFalse(r.allowed("/projects/ci/file"))
        self.assertFalse(r.allowed("/exact"))
        self.assertTrue(r.allowed("/exact/more"))
        self.assertTrue(r.allowed("/"))

    def test_a_group_for_our_token_replaces_the_star_group(self) -> None:
        r = fetch.Robots("User-agent: *\nDisallow: /\n\nUser-agent: enth-research\nCrawl-delay: 7\nDisallow: /private\n")
        self.assertTrue(r.allowed("/public"))
        self.assertFalse(r.allowed("/private/x"))
        self.assertEqual(r.crawl_delay, 7.0)

    def test_empty_disallow_allows_all_and_no_robots_allows_all(self) -> None:
        self.assertTrue(fetch.Robots("User-agent: *\nDisallow:\n").allowed("/anything"))
        self.assertTrue(fetch.Robots("").allowed("/anything"))
        self.assertTrue(fetch.Robots("User-agent: *\nDisallow: /a\n# comment\nCrawl-delay: x\n").allowed("/b"))

    def test_the_other_bots_disallow_does_not_bind_us(self) -> None:
        r = fetch.Robots("User-agent: ClaudeBot\nDisallow: /\n")
        self.assertTrue(r.allowed("/x"))


class Download(Case):
    def test_a_fetch_checks_robots_first_sends_only_our_user_agent_and_extracts(self) -> None:
        data = zip_bytes({"RIRS/a/x.wav": b"RIFFxxxx" * 50, "RIRS/a/y.wav": b"RIFFyyyy" * 50})
        parts = '[[source.part]]\nid = "a"\npath = "RIRS/a"\nkind = "rir"\nlicence = "CC0-1.0"\nuse = "shipped-training"\n'
        with Site({"test.zip": data}, "User-agent: *\nCrawl-delay: 5\nDisallow: /search/\n") as site:
            m = manifest_for(site.url("/data/test.zip"), data, parts=parts)
            self.assertEqual(self.run_fetch(m, site), 0, "\n".join(self.out))
        self.assertEqual(site.log[0][0], "/robots.txt", "robots.txt is the first request")
        self.assertEqual([p for p, _ in site.log], ["/robots.txt", "/data/test.zip"])
        for _, headers in site.log:
            self.assertEqual(headers["user-agent"], "enth-research/0.1")
            self.assertFalse({"from", "referer", "cookie", "authorization", "x-forwarded-for"} & set(headers), headers)
        self.assertTrue(any(s >= 4.9 for s in self.clock.slept), "the crawl-delay was waited")
        self.assertEqual((self.root / "test-set" / "RIRS" / "a" / "x.wav").read_bytes(), b"RIFFxxxx" * 50)
        self.assertTrue((self.root / "dl" / "test.zip").is_file())
        self.assertFalse((self.root / "test-set.partial").exists())
        self.assertTrue(any("index test-set/a: 2 files" in line for line in self.out), self.out)

    def test_a_robots_disallow_stops_the_run_before_the_file_is_asked_for(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, "User-agent: *\nDisallow: /data/\n") as site:
            m = manifest_for(site.url("/data/test.zip"), data)
            with self.assertRaisesRegex(fetch.FetchError, "robots.txt .* disallows"):
                self.run_fetch(m, site)
        self.assertEqual(site.asked("/data/test.zip"), 0)
        self.assertFalse((self.root / "dl" / "test.zip").exists())

    def test_a_missing_robots_txt_allows_but_an_unreadable_one_stops(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, None) as site:
            self.assertEqual(self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site), 0)

        self.root = self.dir / "second"
        with Site({"test.zip": data}, None) as site:
            site.httpd.RequestHandlerClass.do_GET = lambda self: self.send_error(503)  # type: ignore[method-assign]
            with self.assertRaisesRegex(fetch.FetchError, "robots.txt cannot be read"):
                self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site)
        self.assertFalse((self.root / "dl" / "test.zip").exists())

    def test_a_redirect_is_followed_only_where_robots_allows_it(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, "User-agent: *\nDisallow: /secret/\n", redirects={"/go": "/secret/test.zip"}) as site:
            m = manifest_for(site.url("/go"), data)
            with self.assertRaisesRegex(fetch.FetchError, "disallows /secret/test.zip"):
                self.run_fetch(m, site)
        with Site({"test.zip": data}, "User-agent: *\nDisallow:\n", redirects={"/go": "/data/test.zip"}) as site:
            self.assertEqual(self.run_fetch(manifest_for(site.url("/go"), data), site), 0)

    def test_requests_to_one_host_are_spaced_at_least_two_seconds(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, "") as site:
            self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site)
        self.assertGreaterEqual(sum(self.clock.slept), 1.9)

    def test_an_interrupted_download_resumes_from_the_part_file(self) -> None:
        data = zip_bytes({"a.wav": os.urandom(40000)})
        (self.root / "dl").mkdir(parents=True)
        (self.root / "dl" / "test.zip.part").write_bytes(data[:15000])
        with Site({"test.zip": data}, "") as site:
            self.assertEqual(self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site), 0)
        self.assertEqual((self.root / "dl" / "test.zip").read_bytes(), data)
        ranged = [h for p, h in site.log if p == "/data/test.zip"]
        self.assertEqual(ranged[0]["range"], "bytes=15000-")

    def test_a_wrong_hash_or_size_is_kept_for_inspection_and_never_extracted(self) -> None:
        data = zip_bytes({"a.wav": b"x" * 100})
        with Site({"test.zip": data}, "") as site:
            with self.assertRaisesRegex(fetch.FetchError, "does not match the SHA-256"):
                self.run_fetch(manifest_for(site.url("/data/test.zip"), data, sha="0" * 64), site)
            self.assertTrue((self.root / "dl" / "test.zip").is_file())
            self.assertFalse((self.root / "test-set").exists())
        self.clock = FakeClock()
        with Site({"test.zip": data}, "") as site:
            with self.assertRaisesRegex(fetch.FetchError, "the manifest says"):
                self.run_fetch(manifest_for(site.url("/data/test.zip"), data, bytes_=len(data) + 5), site)
            self.assertTrue((self.root / "dl" / "test.zip.part").is_file(), "kept for inspection, not promoted")

    def test_a_first_fetch_without_a_pinned_hash_prints_the_digest_and_does_not_extract(self) -> None:
        data = zip_bytes({"a.wav": b"x" * 100})
        with Site({"test.zip": data}, "") as site:
            m = manifest_for(site.url("/data/test.zip"), data, sha="")
            self.assertEqual(self.run_fetch(m, site), 0)
            self.assertTrue(any(hashlib.sha256(data).hexdigest() in line for line in self.out))
            self.assertFalse((self.root / "test-set").exists())
            self.assertEqual(self.run_fetch(m, site, trust_first=True), 0)
            self.assertTrue((self.root / "test-set" / "a.wav").is_file())

    def test_a_404_or_a_429_is_reported_once_and_not_retried(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({}, "") as site:
            with self.assertRaisesRegex(fetch.FetchError, "HTTP 404"):
                self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site)
            self.assertEqual(site.asked("/data/test.zip"), 1)


class Gate(Case):
    def test_excluded_sources_are_never_fetched_and_research_only_ones_need_the_flag(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, "") as site:
            for use, flag in (("excluded", True), ("research-only", False)):
                with self.subTest(use=use):
                    m = manifest_for(site.url("/data/test.zip"), data, use=use, extra_src='reason = "NonCommercial."')
                    with self.assertRaises(fetch.FetchError):
                        self.run_fetch(m, site, research=flag)
            m = manifest_for(site.url("/data/test.zip"), data, use="research-only")
            self.assertEqual(self.run_fetch(m, site, research=True), 0)
            self.assertEqual(site.asked("/data/test.zip"), 1, "only the allowed run fetched")

    def test_it_will_not_go_on_without_yes_and_says_what_the_licence_is(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        with Site({"test.zip": data}, "") as site:
            with self.assertRaisesRegex(fetch.FetchError, "--yes"):
                self.run_fetch(manifest_for(site.url("/data/test.zip"), data), site, yes=False)
            self.assertEqual(site.log, [], "nothing was requested")
        self.assertTrue(any("licence CC0-1.0" in line for line in self.out))

    def test_big_archives_and_by_hand_sources_are_refused(self) -> None:
        data = zip_bytes({"a.wav": b"x"})
        m = manifest_for("http://127.0.0.1:9/x", data, bytes_=3_500_000_000)
        with self.assertRaisesRegex(fetch.FetchError, "--allow-large"):
            self.run_fetch(m)
        m = manifest_for("https://example.test/landing-page", data, bytes_=0)
        with self.assertRaisesRegex(fetch.FetchError, "by hand"):
            self.run_fetch(m)

    def test_the_real_manifest_is_gated_the_same_way(self) -> None:
        m = mf.load_manifest()
        for ref, message in (("wham-noise", "excluded"), ("openair", "excluded"), ("esc-50", "--research"),
                             ("demand", "--research"), ("fsd50k", "by hand"), ("musan", "--allow-large"),
                             ("but-reverbdb", "--allow-large"), ("no-such", "not a source")):
            with self.subTest(ref=ref), self.assertRaisesRegex(fetch.FetchError, message):
                fetch.gate(m, ref, research=False, allow_large=False, yes=True, say=lambda *_: None)

    def test_the_script_sends_nothing_but_a_user_agent(self) -> None:
        text = (ML / "data" / "fetch.py").read_text()
        self.assertIn('USER_AGENT = "enth-research/0.1"', text)
        self.assertIn('headers={"User-Agent": USER_AGENT, **(headers or {})}', text)
        for header in ("Referer", "From", "Cookie", "Authorization", "X-Forwarded-For"):
            self.assertFalse(f'"{header}"' in text or f"'{header}'" in text, header)


class Extraction(Case):
    def extract(self, archive: bytes, name: str = "a.zip", **kw) -> dict:
        path = self.dir / name
        path.write_bytes(archive)
        return fetch.safe_extract(path, self.dir / "out", **{"max_bytes": 10_000_000, **kw})

    def test_a_plain_archive_extracts_with_safe_permissions(self) -> None:
        stats = self.extract(zip_bytes({"d/x.wav": b"1" * 10, "d/e/y.wav": b"2" * 20}))
        self.assertEqual((stats["files"], stats["bytes"]), (2, 30))
        self.assertEqual((self.dir / "out" / "d" / "e" / "y.wav").stat().st_mode & 0o777, 0o644)

    def test_names_that_climb_or_are_absolute_stop_everything_before_a_byte_is_written(self) -> None:
        for bad in ("../evil.wav", "a/../../evil.wav", "/etc/evil.wav", "a\\..\\evil.wav", "C:/evil.wav"):
            with self.subTest(name=bad):
                data = zip_bytes({"fine.wav": b"x"}, extra=lambda z, b=bad: z.writestr(zipfile.ZipInfo(b), b"evil"))
                with self.assertRaisesRegex(fetch.FetchError, "unsafe name|climbs"):
                    self.extract(data, name=f"{abs(hash(bad))}.zip")
                self.assertFalse((self.dir / "evil.wav").exists())
                self.assertEqual(list((self.dir / "out").rglob("*.wav")), [], "not even the harmless member")
                for leftover in (self.dir / "out",):
                    import shutil

                    shutil.rmtree(leftover)

    def test_symlinks_in_a_zip_and_a_tar_are_skipped_not_created(self) -> None:
        def link(z: zipfile.ZipFile) -> None:
            info = zipfile.ZipInfo("lnk")
            info.external_attr = (0o120777 << 16)
            z.writestr(info, "/etc/passwd")

        stats = self.extract(zip_bytes({"ok.wav": b"x"}, extra=link))
        self.assertEqual((stats["files"], stats["skipped_members"]), (1, 1))
        self.assertFalse((self.dir / "out" / "lnk").exists())
        import shutil

        shutil.rmtree(self.dir / "out")
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as t:
            ti = tarfile.TarInfo("ok.wav")
            ti.size = 1
            t.addfile(ti, io.BytesIO(b"x"))
            sym = tarfile.TarInfo("lnk")
            sym.type, sym.linkname = tarfile.SYMTYPE, "/etc/passwd"
            t.addfile(sym)
            hard = tarfile.TarInfo("hard")
            hard.type, hard.linkname = tarfile.LNKTYPE, "ok.wav"
            t.addfile(hard)
        stats = self.extract(buf.getvalue(), name="a.tgz")
        self.assertEqual((stats["files"], stats["skipped_members"]), (1, 2))
        self.assertEqual(sorted(p.name for p in (self.dir / "out").iterdir()), ["ok.wav"])

    def test_a_bomb_and_a_flood_are_stopped(self) -> None:
        with self.assertRaisesRegex(fetch.FetchError, "expands past"):
            self.extract(zip_bytes({"zeros.bin": bytes(2_000_000)}), max_bytes=1_000_000)
        import shutil

        shutil.rmtree(self.dir / "out")
        with self.assertRaisesRegex(fetch.FetchError, "more than 3 files"):
            self.extract(zip_bytes({f"f{i}": b"x" for i in range(5)}), name="b.zip", max_files=3)

    def test_an_existing_target_and_a_non_archive_are_refused(self) -> None:
        (self.dir / "out").mkdir()
        with self.assertRaisesRegex(fetch.FetchError, "exists"):
            self.extract(zip_bytes({"a": b"x"}))
        (self.dir / "out").rmdir()
        with self.assertRaisesRegex(fetch.FetchError, "neither a zip nor a tar"):
            self.extract(b"plain text, not an archive", name="c.bin")

    def test_two_members_with_one_name_do_not_overwrite_each_other(self) -> None:
        import warnings

        buf = io.BytesIO()
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            with zipfile.ZipFile(buf, "w") as z:
                z.writestr("same.wav", b"first")
                z.writestr("same.wav", b"second")
        with self.assertRaises(FileExistsError):
            self.extract(buf.getvalue())


class Verify(Case):
    def test_verify_rehashes_the_archive_and_the_indexes_without_the_network(self) -> None:
        data = zip_bytes({"a/x.wav": b"RIFF" * 10})
        parts = '[[source.part]]\nid = "a"\npath = "a"\nkind = "rir"\nlicence = "CC0-1.0"\nuse = "shipped-training"\n'
        with Site({"test.zip": data}, "") as site:
            m = manifest_for(site.url("/data/test.zip"), data, parts=parts)
            self.run_fetch(m, site)
            asked = len(site.log)
            self.assertEqual(fetch.verify(m, "test-set", self.root, say=self.out.append), 0)
            self.assertEqual(len(site.log), asked, "no request")
        (self.root / "dl" / "test.zip").write_bytes(b"tampered")
        self.assertEqual(fetch.verify(m, "test-set", self.root, say=self.out.append), 1)


if __name__ == "__main__":
    unittest.main()
