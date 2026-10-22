"""propose v0: a small 2D CNN with Kₘ winner-takes-all hypothesis heads and a confidence head (docs/r3.md, "The
model: propose v0 (lane TR)").

Built only from op set 1 (``crates/enth-solver/src/propose/pack.rs``), so the exporter can write it to ``ENTHMODL``:

* the input standardisation is ``Mul`` and ``Add`` constants inside the graph (the model contract: never Python
  preprocessing);
* the trunk is ``Conv2d`` (explicit symmetric padding, floor mode), ``BatchNorm2d`` (folded into the convolution at
  export: ``fold.fold_batchnorm``), ``Silu`` and residual ``Add``;
* the frequency axis is pooled by an ``AvgPool2d`` with a full-length kernel and no padding (so counting padding never
  arises), then ``Reshape``; the time axis is kept (8 steps of 250 ms) so envelope timing reaches the heads;
* the heads are ``Linear``, ``Concat`` (the context), ``Reshape``, ``Silu``, a per-hypothesis ``MatMul`` + ``Add``, and the
  output row is ``Slice`` + ``Sigmoid`` + ``Concat``: continuous values and route depths in 0-1, logits elsewhere,
  exactly what ``decode`` reads.

Shapes carry the batch axis here; the exporter drops it (op set 1 has none).
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field

import torch
from torch import nn

from . import layout as L


@dataclass
class ModelConfig:
    """The network's shape. The defaults are propose v0's (about 3.8 M parameters, about 0.48 GFLOP)."""

    stem: int = 24
    widths: tuple[int, ...] = (32, 64, 128, 256)
    blocks: tuple[int, ...] = (1, 1, 2, 1)
    embed: int = 384  # the embedding (exported later for retrieval)
    hyp_hidden: int = 128  # per-hypothesis hidden width
    conf_hidden: int = 256
    hypotheses: int = 8  # Kₘ
    mel_shape: tuple[int, int, int] = L.MEL_SHAPE
    context_len: int = L.CONTEXT_LEN
    width: int = L.WIDTH

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_dict(cls, d: dict) -> "ModelConfig":
        d = dict(d)
        for k in ("widths", "blocks", "mel_shape"):
            if k in d:
                d[k] = tuple(d[k])
        return cls(**d)


class ConvBN(nn.Module):
    """``Conv2d`` (no bias; 3x3, padding 1 on every side) then ``BatchNorm2d``; the activation is the caller's."""

    def __init__(self, cin: int, cout: int, stride: int = 1) -> None:
        super().__init__()
        self.conv = nn.Conv2d(cin, cout, kernel_size=3, stride=stride, padding=1, bias=False)
        self.bn = nn.BatchNorm2d(cout)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.bn(self.conv(x))


class ResBlock(nn.Module):
    """``silu(x + bn(conv(silu(bn(conv(x))))))``: the basic residual block."""

    def __init__(self, c: int) -> None:
        super().__init__()
        self.a = ConvBN(c, c)
        self.b = ConvBN(c, c)
        self.act = nn.SiLU()

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.act(x + self.b(self.act(self.a(x))))


class Stage(nn.Module):
    """A stride-2 ``ConvBN`` + ``Silu``, then ``n`` residual blocks."""

    def __init__(self, cin: int, cout: int, n: int) -> None:
        super().__init__()
        self.down = ConvBN(cin, cout, stride=2)
        self.act = nn.SiLU()
        self.blocks = nn.Sequential(*[ResBlock(cout) for _ in range(n)])

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.blocks(self.act(self.down(x)))


class ProposeV0(nn.Module):
    """``forward(mel [B, 3, 128, 64], context [B, 5]) -> (hypotheses [B, K, 102], confidence [B, K])``.

    ``hypotheses`` holds layout 1's rows as decoding reads them (0-1 values where the layout has values, logits
    elsewhere); ``confidence`` holds K logits (create orders proposals by them, descending). ``embed`` returns the
    embedding the heads read (for retrieval later).
    """

    def __init__(self, cfg: ModelConfig | None = None) -> None:
        super().__init__()
        cfg = cfg or ModelConfig()
        self.cfg = cfg
        res, t, f = cfg.mel_shape
        if len(cfg.widths) != len(cfg.blocks):
            raise ValueError("widths and blocks must have the same length")
        # Input standardisation: y = x * scale + shift (graph constants; `set_standardisation` fills them from data).
        self.register_buffer("mel_scale", torch.ones(1, res, 1, f))
        self.register_buffer("mel_shift", torch.zeros(1, res, 1, f))
        self.register_buffer("ctx_scale", torch.ones(1, cfg.context_len))
        self.register_buffer("ctx_shift", torch.zeros(1, cfg.context_len))

        self.stem = ConvBN(res, cfg.stem)
        self.stem_act = nn.SiLU()
        stages = []
        c = cfg.stem
        for w, n in zip(cfg.widths, cfg.blocks):
            stages.append(Stage(c, w, n))
            c = w
            t = (t - 1) // 2 + 1  # floor mode, 3x3, stride 2, padding 1
            f = (f - 1) // 2 + 1
        self.stages = nn.Sequential(*stages)
        self.trunk_shape = (c, t, f)
        # Pool the frequency axis only: an AvgPool2d whose kernel is the whole axis (no padding).
        self.freq_pool = nn.AvgPool2d(kernel_size=(1, f), stride=(1, f), padding=0, ceil_mode=False, count_include_pad=False)
        self.flat = c * t
        self.proj = nn.Linear(self.flat, cfg.embed)
        self.mix = nn.Linear(cfg.embed + cfg.context_len, cfg.embed)
        self.act = nn.SiLU()
        k, h1 = cfg.hypotheses, cfg.hyp_hidden
        self.hyp_in = nn.Linear(cfg.embed, k * h1)
        # Per-hypothesis output layer: [B, K, 1, H1] @ [K, H1, W] + [K, 1, W] (MatMul broadcasting the leading axes).
        self.hyp_w = nn.Parameter(torch.empty(k, h1, cfg.width))
        self.hyp_b = nn.Parameter(torch.zeros(k, 1, cfg.width))
        with torch.no_grad():
            bound = 1.0 / h1**0.5  # nn.Linear's default range for a fan-in of H1
            self.hyp_w.uniform_(-bound, bound)
            # Break the symmetry between the K heads at the start: each head gets its own bias draw.
            self.hyp_b.uniform_(-0.5, 0.5)
        self.conf_in = nn.Linear(cfg.embed, cfg.conf_hidden)
        self.conf_out = nn.Linear(cfg.conf_hidden, k)

    # -- standardisation ---------------------------------------------------------------------------------------
    @torch.no_grad()
    def set_standardisation(
        self, mel_mean: torch.Tensor, mel_std: torch.Tensor, ctx_mean: torch.Tensor, ctx_std: torch.Tensor
    ) -> None:
        """Set ``x * scale + shift`` from per-(resolution, band) and per-context-entry means and deviations."""
        res, _, f = self.cfg.mel_shape
        # Floors keep a (nearly) constant input from getting a huge gain that an unusual upload would then blow up.
        mel_std = mel_std.clamp_min(0.05)
        ctx_std = ctx_std.clamp_min(0.1)
        self.mel_scale.copy_((1.0 / mel_std).reshape(1, res, 1, f))
        self.mel_shift.copy_((-mel_mean / mel_std).reshape(1, res, 1, f))
        self.ctx_scale.copy_((1.0 / ctx_std).reshape(1, -1))
        self.ctx_shift.copy_((-ctx_mean / ctx_std).reshape(1, -1))

    # -- forward ---------------------------------------------------------------------------------------------------
    def embed(self, mel: torch.Tensor, context: torch.Tensor) -> torch.Tensor:
        x = mel * self.mel_scale + self.mel_shift
        c = context * self.ctx_scale + self.ctx_shift
        x = self.stem_act(self.stem(x))
        x = self.stages(x)
        x = self.freq_pool(x)  # [B, C, T, 1]
        x = x.reshape(x.shape[0], self.flat)
        e = self.act(self.proj(x))
        e = self.act(self.mix(torch.cat([e, c.to(e.dtype)], dim=1)))
        return e

    def heads(self, e: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        k, h1, w = self.cfg.hypotheses, self.cfg.hyp_hidden, self.cfg.width
        b = e.shape[0]
        h = self.act(self.hyp_in(e)).reshape(b, k, 1, h1)
        raw = (torch.matmul(h, self.hyp_w) + self.hyp_b).reshape(b, k, w)
        c0, c1 = L.CONTINUOUS
        d0, d1 = L.DEPTH
        hyp = torch.cat([torch.sigmoid(raw[..., c0:c1]), raw[..., c1:d0], torch.sigmoid(raw[..., d0:d1])], dim=-1)
        conf = self.conf_out(self.act(self.conf_in(e)))
        return hyp, conf

    def forward(self, mel: torch.Tensor, context: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        e = self.embed(mel, context)
        # The heads run in f32 under autocast: the 0-1 values are compared with steps as fine as 0.0026 (a quarter
        # semitone in `osc1.semi`), which f16's 2^-11 spacing near 1 would blur.
        with torch.autocast(device_type=e.device.type, enabled=False):
            return self.heads(e.float())


def count_parameters(model: nn.Module) -> int:
    """Trainable parameters (BatchNorm's affine vectors included)."""
    return sum(p.numel() for p in model.parameters())


def exported_parameter_count(model: ProposeV0) -> int:
    """Parameters as ``ENTHMODL`` would count them: BatchNorm folded (each BN adds its conv a bias of C, drops 2C),
    standardisation constants included."""
    n = 0
    for m in model.modules():
        if isinstance(m, nn.Conv2d):
            n += m.weight.numel() + m.out_channels  # folded bias
        elif isinstance(m, nn.Linear):
            n += m.weight.numel() + (m.bias.numel() if m.bias is not None else 0)
    n += model.hyp_w.numel() + model.hyp_b.numel()
    n += model.mel_scale.numel() + model.mel_shift.numel() + model.ctx_scale.numel() + model.ctx_shift.numel()
    return n


def count_flops(model: ProposeV0) -> int:
    """FLOPs per inference as pack.rs counts them: 2 x the multiply-adds of Conv2d, Linear and MatMul, plus 1 per output
    element of every other op (Reshape, Concat and Slice count 0; BatchNorm is folded, so it counts 0)."""
    cfg = model.cfg
    flops = 0
    res, t, f = cfg.mel_shape
    flops += 2 * res * t * f  # Mul, Add (standardisation)
    flops += 2 * cfg.context_len

    def conv(cin: int, cout: int, t: int, f: int, stride: int) -> tuple[int, int, int]:
        to, fo = (t - 1) // stride + 1, (f - 1) // stride + 1
        return 2 * cout * to * fo * cin * 9, to, fo

    c = res
    x, t, f = conv(c, cfg.stem, t, f, 1)
    flops += x + cfg.stem * t * f  # conv, silu
    c = cfg.stem
    for w, n in zip(cfg.widths, cfg.blocks):
        x, t, f = conv(c, w, t, f, 2)
        flops += x + w * t * f
        c = w
        for _ in range(n):
            x, _, _ = conv(c, c, t, f, 1)
            flops += 2 * x + 3 * c * t * f  # two convs; silu, add, silu
    flops += c * t  # the frequency pool's outputs
    flat = c * t
    k, h1, wd = cfg.hypotheses, cfg.hyp_hidden, cfg.width
    for a, b in ((flat, cfg.embed), (cfg.embed + cfg.context_len, cfg.embed), (cfg.embed, k * h1), (cfg.embed, cfg.conf_hidden), (cfg.conf_hidden, k)):
        flops += 2 * a * b
    flops += 2 * cfg.embed + k * h1 + cfg.conf_hidden  # silu outputs
    flops += 2 * k * h1 * wd + k * wd  # MatMul, Add
    flops += (L.CONTINUOUS[1] - L.CONTINUOUS[0] + L.DEPTH[1] - L.DEPTH[0]) * k  # sigmoids
    return flops


ALLOWED_MODULES = (nn.Conv2d, nn.BatchNorm2d, nn.Linear, nn.SiLU, nn.AvgPool2d, nn.Sequential, nn.ModuleList)


def audit_ops(model: ProposeV0) -> list[str]:
    """Problems that would keep the model out of op set 1 (empty when it is exportable as built)."""
    problems: list[str] = []
    own = (ProposeV0, ConvBN, ResBlock, Stage)
    for name, m in model.named_modules():
        if isinstance(m, own):
            continue
        if not isinstance(m, ALLOWED_MODULES):
            problems.append(f"{name}: {type(m).__name__} is not in op set 1")
        if isinstance(m, nn.Conv2d):
            if m.padding_mode != "zeros" or isinstance(m.padding, str):
                problems.append(f"{name}: padding must be explicit zeros")
            if m.kernel_size[0] % 2 == 0 or m.kernel_size[1] % 2 == 0:
                problems.append(f"{name}: an even kernel needs asymmetric padding")
        if isinstance(m, nn.AvgPool2d):
            if m.ceil_mode:
                problems.append(f"{name}: ceil mode")
            pad = m.padding if isinstance(m.padding, tuple) else (m.padding, m.padding)
            if any(pad) and m.count_include_pad:
                problems.append(f"{name}: AvgPool counts padding (op set 1 does not)")
    return problems
