"""Running the system ``ffmpeg`` binary as a subprocess (it is LGPL or GPL; it is never imported or linked).

Every call is built from an argument template (``{in}``, ``{out}``, ``{aux0}``... stand for paths), so the JSON record
can say exactly what was run without naming the temporary files. The fixed part (one thread, no stdin, bit-exact
output headers with no encoder version string) is added here.
"""

from __future__ import annotations

import functools
import os
import re
import subprocess
from collections.abc import Mapping, Sequence

FFMPEG = os.environ.get("ENTH_FFMPEG", "ffmpeg")

# Fixed global options. One thread and one filter thread: slice threading must not change the order of a float sum.
BASE_ARGS = ["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-threads", "1", "-filter_threads", "1"]
# Output options for every file written: no metadata, and no `Lavf<version>` string in the WAV header, so the bytes
# of a working file do not depend on the libavformat version.
OUT_FLAGS = ["-map_metadata", "-1", "-fflags", "+bitexact", "-flags:a", "+bitexact"]


class FfmpegError(RuntimeError):
    """ffmpeg is missing or a call failed."""


@functools.cache
def version_line() -> str:
    """The first line of ``ffmpeg -version``, recorded in every JSON record."""
    try:
        done = subprocess.run([FFMPEG, "-version"], capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError) as e:
        raise FfmpegError(f"cannot run {FFMPEG!r} (install ffmpeg, or set ENTH_FFMPEG): {e}") from e
    return done.stdout.splitlines()[0].strip()


@functools.cache
def encoders() -> frozenset[str]:
    """The names of the audio encoders this ffmpeg was built with."""
    done = subprocess.run([FFMPEG, "-hide_banner", "-encoders"], capture_output=True, text=True, check=True)
    return frozenset(m.group(1) for m in re.finditer(r"^ A\S{5} (\S+)", done.stdout, re.MULTILINE))


def run(args: Sequence[str], files: Mapping[str, str]) -> str:
    """Run ``ffmpeg`` with the fixed options plus ``args``; ``{name}`` elements are replaced by ``files[name]``.

    Returns stderr (empty unless the template raises the log level, as the loudness measurement does).
    """
    argv = [FFMPEG, *BASE_ARGS]
    for a in args:
        argv.append(files[a[1:-1]] if a.startswith("{") and a.endswith("}") and a[1:-1] in files else a)
    env = dict(os.environ, AV_LOG_FORCE_NOCOLOR="1")
    try:
        done = subprocess.run(argv, capture_output=True, text=True, env=env, check=False)
    except OSError as e:
        raise FfmpegError(f"cannot run {FFMPEG!r}: {e}") from e
    if done.returncode != 0:
        tail = "\n".join(done.stderr.strip().splitlines()[-6:])
        raise FfmpegError(f"ffmpeg failed ({done.returncode}) on {' '.join(args)[:300]}\n{tail}")
    return done.stderr
