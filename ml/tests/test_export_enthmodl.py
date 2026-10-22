"""Lane ONX's model-file writer, raw tensor files and synthetic inputs (``enth_ml.export``). Standard library only.

The writer must lay out exactly what ``pack.rs`` reads: it reproduces the pinned format-1 fixture of ``pack.rs``'s
tests byte for byte, its op table agrees with ``pack.rs``'s ``OPS``, and the synthetic inputs agree with
``propose::net::synthetic_input`` on the values its test pins.
"""

import hashlib
import pathlib
import re
import struct
import unittest

from enth_ml.export import enthmodl, synthetic, tensors
from enth_ml.export.enthmodl import CONTEXT, ModelWriter

ROOT = pathlib.Path(__file__).resolve().parents[2]
PACK_RS = ROOT / "crates/enth-solver/src/propose/pack.rs"


def f32(v: float) -> float:
    return struct.unpack("<f", struct.pack("<f", v))[0]


def pinned_fixture() -> bytes:
    """``pack.rs``'s ``Fixture::default()``: two ``Linear`` heads on ``context`` (``the_format_1_fixture_is_pinned``)."""
    w = ModelWriter("propose-v0-test", 8, engine="0.2.0", flops=10_000, ds1=1, manifest_sha256=b"\x5a" * 32)
    w1 = w.tensor("head.hyp.weight", [8 * 102, 5], [1.0] * (8 * 102 * 5), "f16")
    w2 = w.tensor("head.conf.weight", [8, 5], [i * 0.25 for i in range(40)])
    h = w.node("Linear", [CONTEXT, w1])
    r = w.node("Reshape", [h], [8, 102])
    c = w.node("Linear", [CONTEXT, w2])
    w.output("hypotheses", r, [8, 102])
    w.output("confidence", c, [8])
    w.add_info("commit", "0123abc")
    w.add_info("licence", "renders only")
    return w.build()


class Writer(unittest.TestCase):
    def test_it_writes_the_pinned_format_1_fixture(self) -> None:
        text = PACK_RS.read_text(encoding="utf-8")
        length = int(re.search(r"const FIXTURE_LEN: usize = (\d+);", text).group(1))
        sha = re.search(r'const FIXTURE_SHA256: &str = "([0-9a-f]{64})";', text).group(1)
        b = pinned_fixture()
        self.assertEqual((len(b), hashlib.sha256(b).hexdigest()), (length, sha))

    def test_the_header_hash_covers_the_file_with_its_field_zeroed(self) -> None:
        b = bytearray(pinned_fixture())
        stored = bytes(b[224:256])
        b[224:256] = bytes(32)
        self.assertEqual(hashlib.sha256(b).digest(), stored)
        self.assertEqual(b[:8], b"ENTHMODL")
        self.assertEqual(struct.unpack_from("<HHHHHH", b, 8), (1, 256, 1, 1, 1, 1))
        self.assertEqual(bytes(b[80:112]), enthmodl.layout_v1_sha256())

    def test_tensors_are_aligned_and_subnormal_weights_flushed(self) -> None:
        w = ModelWriter("t", 1)
        a = w.tensor("a", [3], [1.0, 1e-40, -2.0])
        b = w.tensor("b", [2], [0.5, 0.25], "f16")
        self.assertEqual((a.index, b.index), (0, 1))
        self.assertEqual(w.parameters(), 5)
        data = b"".join(t.data for t in w.tensors)
        self.assertEqual(struct.unpack("<3f", w.tensors[0].data), (1.0, 0.0, -2.0))
        self.assertEqual(struct.unpack("<2e", w.tensors[1].data), (0.5, 0.25))
        self.assertEqual(len(data), 12 + 4)
        built = w.build()
        data_offset, data_len = struct.unpack_from("<QQ", built, 144)
        self.assertEqual(data_offset % 64, 0)
        self.assertEqual(data_len, 64 + 4, "the second tensor starts at the next multiple of 64")

    def test_value_ids_run_inputs_tensors_then_nodes(self) -> None:
        w = ModelWriter("t", 1)
        n0 = w.node("Relu", [CONTEXT])
        t0 = w.tensor("w", [1], [1.0])
        self.assertEqual((w._id(CONTEXT), w._id(t0), w._id(n0)), (1, 2, 3))

    def test_bad_nodes_are_refused_by_the_writer(self) -> None:
        w = ModelWriter("t", 1)
        with self.assertRaises(ValueError):
            w.node("Relu", [CONTEXT, CONTEXT])
        with self.assertRaises(ValueError):
            w.node("Softmax", [CONTEXT], [])
        with self.assertRaises(ValueError):
            w.tensor("x", [2], [1.0, float("inf")])

    def test_the_op_table_agrees_with_pack_rs(self) -> None:
        text = PACK_RS.read_text(encoding="utf-8")
        rows = re.findall(
            r'info\(\s*OpCode::(\w+),\s*"(\w+)",\s*\((\d+), (\d+)\),\s*Arity::(Exactly|AtLeast)\((\d+)\),?\s*\)', text
        )
        self.assertEqual(len(rows), 24)
        for code, (variant, name, lo, hi, arity, n) in enumerate(rows, start=1):
            self.assertEqual(variant, name)
            self.assertEqual(enthmodl.OPS[name], (code, (int(lo), int(hi)), int(n), arity == "AtLeast"), name)

    def test_eps_attributes_are_f32_bit_patterns(self) -> None:
        self.assertEqual(enthmodl.eps_attr(1e-5), struct.unpack("<i", struct.pack("<f", 1e-5))[0])
        self.assertEqual(enthmodl.eps_attr(-1.0), struct.unpack("<i", struct.pack("<f", -1.0))[0])


class Tensors(unittest.TestCase):
    def test_round_trip(self) -> None:
        entries = [("a", (2, 3), [0.5 * i for i in range(6)]), ("hypotheses", (1,), [-1.25])]
        self.assertEqual(tensors.read(tensors.write(entries)), entries)
        with self.assertRaises(ValueError):
            tensors.write([("x", (2,), [1.0])])
        with self.assertRaises(ValueError):
            tensors.read(b"NOTATENS" + bytes(4))


class Synthetic(unittest.TestCase):
    def test_the_values_rust_pins(self) -> None:
        mel, context = synthetic.synthetic_input(0)
        self.assertEqual((len(mel), len(context)), (3 * 128 * 64, 5))
        # propose::net's synthetic_inputs_are_exact_and_pinned.
        want = [1.533_243_2, -0.273_888_1, -1.894_264_9, 1.883_527_8]
        self.assertEqual(mel[:4], [f32(v) for v in want])
        self.assertEqual(context[4], f32(0.750_684_74))

    def test_every_value_is_exact_in_f32_and_not_subnormal(self) -> None:
        mel, context = synthetic.synthetic_input(7)
        for v in mel + context:
            self.assertEqual(f32(v), v)
            self.assertTrue(-2.0 <= v < 2.0)
            self.assertTrue(v == 0.0 or abs(v) >= 2.0**-22)

    def test_weights_match_the_rust_test_support(self) -> None:
        # tests/propose_support's values(seed, n, scale) seeds the stream with seed ^ 0x5EED0000.
        self.assertEqual(synthetic.weights(3, 4, 0.5), synthetic.values(3 ^ 0x5EED0000, 4, 0.5))
        self.assertTrue(all(-0.5 <= v < 0.5 for v in synthetic.weights(3, 100, 0.5)))


if __name__ == "__main__":
    unittest.main()
