"""Lane AUG: a recorded impulse response becomes DEG's shape: a unit impulse at 0 plus a tail at ``wet_db`` below it.

Standard library only; no ffmpeg, no data files (the recordings are synthetic).
"""

from __future__ import annotations

import array
import math
import pathlib
import unittest

from augment_support import temp_dir, write_rir
from enth_ml.augment import rir
from enth_ml.degrade import wavio


class Pieces(unittest.TestCase):
    def test_the_onset_is_the_first_sample_reaching_a_quarter_of_the_peak(self) -> None:
        x = [0.0] * 50
        x[10], x[20], x[30] = 0.1, 0.3, 1.0
        self.assertEqual(rir.onset_index(x), 20)  # 0.1 is under a quarter of 1.0, 0.3 is over
        self.assertEqual(rir.onset_index([-1.0, 0.5]), 0)
        with self.assertRaises(rir.IrError):
            rir.onset_index([0.0] * 8)

    def test_the_decay_curve_starts_at_zero_db_and_never_rises(self) -> None:
        x = [1.0, 0.5, 0.25, 0.125, 0.0625] * 3
        edc = rir.decay_curve_db(x)
        self.assertAlmostEqual(edc[0], 0.0, places=9)
        self.assertTrue(all(b <= a + 1e-12 for a, b in zip(edc, edc[1:], strict=False)))

    def test_rt60_of_an_exponential_decay_is_recovered(self) -> None:
        sr = 16000
        for rt in (0.2, 0.5, 1.2):
            # Energy envelope 10^(-6 t / rt): amplitude 10^(-3 t / rt), the usual definition of an RT60.
            x = [10.0 ** (-3.0 * i / sr / rt) for i in range(int(sr * rt * 1.4))]
            got, method = rir.estimate_rt60(rir.decay_curve_db(x), sr)
            self.assertEqual(method, "T20")
            self.assertAlmostEqual(got, rt, delta=0.04 * rt)

    def test_a_recording_cut_off_while_loud_falls_back_to_t10_or_gives_none(self) -> None:
        # The decay curve of a finite signal ends at -inf only if its last samples are quiet; a signal cut off while
        # still loud never reaches -25 dB (T20 impossible), and a shorter one not even -15 dB (T10 impossible).
        got, method = rir.estimate_rt60(rir.decay_curve_db([1.0] * 40), 16000)
        self.assertEqual(method, "T10")
        self.assertGreater(got, 0.0)
        self.assertEqual(rir.estimate_rt60(rir.decay_curve_db([1.0] * 12), 16000), (None, None))

    def test_channel_choice(self) -> None:
        self.assertEqual(rir.pick_channels(1, 1, 0.9), [0])
        self.assertEqual(rir.pick_channels(1, 2, 0.9), [0, 0])
        self.assertEqual(rir.pick_channels(4, 1, 0.0), [0])
        self.assertEqual(rir.pick_channels(4, 1, 0.99), [3])
        self.assertEqual(rir.pick_channels(4, 2, 0.99), [3, 0])
        self.assertEqual(rir.pick_channels(2, 2, 0.0), [0, 1])

    def test_interleave_and_back(self) -> None:
        tails = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]
        self.assertEqual(list(rir.interleave(tails)), [1.0, 4.0, 2.0, 5.0, 3.0, 6.0])
        self.assertEqual(rir.deinterleave(rir.interleave(tails), 2), tails)


class FromARecording(unittest.TestCase):
    def setUp(self) -> None:
        self._td = temp_dir()
        self.dir = pathlib.Path(self._td.name)

    def tearDown(self) -> None:
        self._td.cleanup()

    def test_the_analysis_finds_the_onset_the_rt60_and_a_short_tail(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.5, delay_ms=3.0)
        a = rir.analyse(self.dir / "a.wav", 1, 0.0)
        self.assertEqual(a.sr, 16000)
        self.assertAlmostEqual(a.onset_ms[0], 3.0, delta=0.1)
        self.assertAlmostEqual(a.rt60_s, 0.5, delta=0.1)
        self.assertEqual(a.rt60_method, "T20")
        self.assertEqual(a.offset_samples, int(round(0.002 * 16000)) + 1)
        # Cut where the decay curve passes -45 dB: well under the 0.65 s that was recorded, and above the rt60 / 2.
        self.assertTrue(0.25 < a.tail_s < 0.5, a.tail_s)
        self.assertEqual(len(a.tails), 1)

    def test_the_kernel_is_a_unit_impulse_then_silence_then_the_tail_at_wet_db(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.4, delay_ms=5.0)
        a = rir.analyse(self.dir / "a.wav", 1, 0.0)
        for wet_db in (-20.0, -6.0, 0.0):
            h, meta = rir.assemble(a.tails, 16000, a.offset_samples / a.sr, wet_db)
            self.assertEqual(h[0], 1.0)
            d = meta["offset_samples"]
            self.assertTrue(all(v == 0.0 for v in h[1:d]), "nothing but the direct impulse in the direct window")
            tail_energy = math.fsum(v * v for v in h[1:])
            self.assertAlmostEqual(10.0 * math.log10(tail_energy), wet_db, places=3)
            self.assertAlmostEqual(meta["wet_db_actual"], wet_db, delta=0.001)

    def test_the_direct_sound_s_colour_is_dropped_whatever_it_is(self) -> None:
        # A recorded direct part spread over 1 ms with mixed signs would filter the clip; it must be gone.
        sr, n = 16000, 8000
        x = array.array("f", [0.0] * n)
        for i, v in enumerate((0.4, 1.0, -0.8, 0.5, -0.3)):
            x[40 + i] = v
        for i in range(60, n):
            x[i] = 0.05 * math.sin(i * 0.37) * 10.0 ** (-3.0 * (i - 60) / sr / 0.3)
        wavio.write_wav_f32(self.dir / "c.wav", sr, 1, x)
        a = rir.analyse(self.dir / "c.wav", 1, 0.0)
        h, meta = rir.assemble(a.tails, sr, a.offset_samples / sr, -10.0)
        self.assertEqual(h[0], 1.0)
        self.assertTrue(all(v == 0.0 for v in h[1 : meta["offset_samples"]]))

    def test_a_clip_s_rate_other_than_the_recording_s_moves_the_tail_not_the_direct_part(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.3)
        a = rir.analyse(self.dir / "a.wav", 1, 0.0)
        _, m16 = rir.assemble(a.tails, 16000, a.offset_samples / a.sr, -6.0)
        _, m48 = rir.assemble([t for t in a.tails], 48000, a.offset_samples / a.sr, -6.0)
        self.assertEqual(m16["offset_samples"], 33)
        self.assertEqual(m48["offset_samples"], 99)

    def test_two_channels_get_equal_length_tails_and_their_own_onsets(self) -> None:
        write_rir(self.dir / "m.wav", rt60=0.4, channels=3, seed=4)
        a = rir.analyse(self.dir / "m.wav", 2, 0.5)
        self.assertEqual(a.channels_picked, [1, 2])
        self.assertEqual(len(a.tails), 2)
        self.assertEqual(len(a.tails[0]), len(a.tails[1]))
        self.assertEqual(a.source_channels, 3)

    def test_a_mono_recording_serves_both_sides_of_a_stereo_clip(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.3)
        a = rir.analyse(self.dir / "a.wav", 2, 0.7)
        self.assertEqual(a.channels_picked, [0, 0])
        self.assertEqual(a.tails[0], a.tails[1])

    def test_the_tail_is_cut_at_the_cap_for_a_very_long_recording(self) -> None:
        write_rir(self.dir / "long.wav", rt60=8.0, dur=7.0, sr=8000)
        a = rir.analyse(self.dir / "long.wav", 1, 0.0)
        self.assertLessEqual(a.tail_s, rir.CAP_S + 0.01)
        self.assertGreater(a.tail_s, 3.0)

    def test_the_tail_fades_out_to_silence(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.4)
        a = rir.analyse(self.dir / "a.wav", 1, 0.0)
        t = a.tails[0]
        self.assertLess(abs(t[-1]), 0.05 * max(abs(v) for v in t))

    def test_analysis_is_deterministic(self) -> None:
        write_rir(self.dir / "a.wav", rt60=0.4, channels=2)
        self.assertEqual(rir.analyse(self.dir / "a.wav", 2, 0.3), rir.analyse(self.dir / "a.wav", 2, 0.3))

    def test_a_silent_recording_and_a_bare_spike_are_refused(self) -> None:
        wavio.write_wav_f32(self.dir / "silent.wav", 16000, 1, array.array("f", [0.0] * 4000))
        with self.assertRaises(rir.IrError):
            rir.analyse(self.dir / "silent.wav", 1, 0.0)
        spike = array.array("f", [0.0] * 4000)
        spike[100] = 1.0
        wavio.write_wav_f32(self.dir / "spike.wav", 16000, 1, spike)
        with self.assertRaisesRegex(rir.IrError, "no tail"):
            rir.analyse(self.dir / "spike.wav", 1, 0.0)

    def test_a_silent_tail_cannot_be_scaled(self) -> None:
        with self.assertRaises(rir.IrError):
            rir.assemble([[0.0] * 100], 16000, 0.002, -6.0)


if __name__ == "__main__":
    unittest.main()
