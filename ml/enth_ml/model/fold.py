"""BatchNorm folding (docs/r3.md: "``BatchNorm`` is folded at export").

``fold_batchnorm`` returns an inference copy of the model in which every ``ConvBN``'s ``BatchNorm2d`` is folded into its
``Conv2d`` (weight scaled per output channel, a bias added) and replaced by an identity, so the graph holds only op-set-1
ops. Lane ONX's exporter can walk the folded copy; the test holds the folded and unfolded outputs within float tolerance.
"""

from __future__ import annotations

import copy

import torch
from torch import nn

from .net import ConvBN, ProposeV0


@torch.no_grad()
def fold_conv_bn(conv: nn.Conv2d, bn: nn.BatchNorm2d) -> nn.Conv2d:
    """A ``Conv2d`` with a bias that computes ``bn(conv(x))`` in eval mode."""
    w = conv.weight
    std = torch.sqrt(bn.running_var + bn.eps)
    gamma = bn.weight if bn.affine else torch.ones_like(std)
    beta = bn.bias if bn.affine else torch.zeros_like(std)
    scale = gamma / std
    fused = nn.Conv2d(
        conv.in_channels,
        conv.out_channels,
        conv.kernel_size,
        stride=conv.stride,
        padding=conv.padding,
        dilation=conv.dilation,
        groups=conv.groups,
        bias=True,
    ).to(w.device, w.dtype)
    fused.weight.copy_(w * scale.reshape(-1, 1, 1, 1))
    b0 = conv.bias if conv.bias is not None else torch.zeros_like(bn.running_mean)
    fused.bias.copy_((b0 - bn.running_mean) * scale + beta)
    return fused


def fold_batchnorm(model: ProposeV0) -> ProposeV0:
    """An eval-mode copy with every BatchNorm folded into its convolution."""
    m = copy.deepcopy(model).eval()
    for mod in m.modules():
        if isinstance(mod, ConvBN):
            mod.conv = fold_conv_bn(mod.conv, mod.bn)
            mod.bn = nn.Identity()
    return m
