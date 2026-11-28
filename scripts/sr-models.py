#!/usr/bin/env python3
"""Build rehue's extra super-resolution models for realesrgan-ncnn-vulkan
from the official Real-ESRGAN weights.

    python3 -I scripts/sr-models.py WEIGHTS_DIR OUT_DIR

WEIGHTS_DIR holds the upstream releases, unchanged
(https://github.com/xinntao/Real-ESRGAN/releases):

    RealESRNet_x4plus.pth            v0.1.1
    realesr-general-x4v3.pth         v0.2.5.0
    realesr-general-wdn-x4v3.pth     v0.2.5.0

OUT_DIR gets one .param (the network) and .bin (the weights) per model,
named as rehue's --model asks for them:

    realesrnet-x4plus           --model soft
    realesr-general-x4v3-dn0    --model general --denoise 0
    realesr-general-x4v3-dn0.5  --model general --denoise 0.5
    realesr-general-x4v3-dn1    --model general --denoise 1

The denoise levels blend the general weights with their weak-denoise
twin, as upstream's --denoise_strength does.

The weights are written the way ncnn's own converter writes them:
convolution weights as fp16 behind ncnn's fp16 tag (fp32 truncated, as
ncnn's float32_to_float16 does, subnormals kept), biases and PReLU
slopes as fp32, layers in module order.  From the official
RealESRGAN_x4plus.pth this gives a .bin byte-identical to the
realesrgan-x4plus.bin the ncnn tool ships.

Needs torch and numpy.  The weights are read with weights_only=True, so
a .pth can't run code.  Real-ESRGAN (code and weights) is BSD-3-Clause,
Xintao Wang; its LICENSE is copied into OUT_DIR.
"""
import os
import struct
import sys

import numpy as np
import torch

LICENSE = """BSD 3-Clause License

Copyright (c) 2021, Xintao Wang
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"""

DENOISE = ["0", "0.5", "1"]


def state(path):
    d = torch.load(path, map_location="cpu", weights_only=True)
    for key in ("params_ema", "params"):
        if key in d:
            return d[key]
    return d


def fp16_ncnn(w):
    """ncnn's float32_to_float16: the significand truncated, subnormals kept."""
    u = np.ascontiguousarray(w, dtype="<f4").view("<u4").astype(np.int64)
    sign = (u >> 31) & 1
    exp = (u >> 23) & 0xFF
    sig = u & 0x7FFFFF
    newexp = exp - 127 + 15
    out = (sign << 15) | (newexp.clip(0, 31) << 10) | (sig >> 13)
    sub = (newexp <= 0) & (newexp >= -10)
    out = np.where(sub, (sign << 15) | ((sig | 0x800000) >> (14 - newexp).clip(0, 63)), out)
    out = np.where((newexp < -10) | (exp == 0), sign << 15, out)
    out = np.where(newexp >= 31, (sign << 15) | (0x1F << 10), out)
    nan = (sign << 15) | (0x1F << 10) | np.where(sig != 0, 0x200, 0)
    out = np.where(exp == 0xFF, nan, out)
    return out.astype("<u2")


def write_bin(sd, path):
    modules = []
    for key in sd:
        module = key.rsplit(".", 1)[0]
        if module not in modules:
            modules.append(module)
    with open(path, "wb") as f:
        for module in modules:
            w = sd[f"{module}.weight"].float().numpy().reshape(-1)
            if f"{module}.bias" in sd:  # convolution
                f.write(struct.pack("<I", 0x01306B47))
                half = fp16_ncnn(w).tobytes()
                f.write(half + b"\0" * (-len(half) % 4))
                f.write(sd[f"{module}.bias"].float().numpy().astype("<f4").tobytes())
            else:  # PReLU slopes
                f.write(w.astype("<f4").tobytes())


class Graph:
    """An ncnn graph; a blob read more than once gets a Split."""

    def __init__(self):
        self.layers = []  # [type, name, inputs, outputs, params]
        self.n = 0

    def blob(self):
        self.n += 1
        return f"b{self.n}"

    def add(self, kind, inputs, params=""):
        out = "data" if kind == "Input" else self.blob()
        self.layers.append([kind, f"{kind.lower()}{len(self.layers)}", list(inputs), [out], params])
        return out

    def conv(self, x, cin, cout, leaky=False):
        p = f"0={cout} 1=3 4=1 5=1 6={cout * cin * 9}"
        if leaky:
            p += " 9=2 -23310=1,2.000000e-01"
        return self.add("Convolution", [x], p)

    def text(self, output):
        self.layers[-1][3] = [output]
        uses = {}
        for layer in self.layers:
            for b in layer[2]:
                uses[b] = uses.get(b, 0) + 1
        lines, blobs, free = [], 0, {}
        for kind, name, inputs, outputs, params in self.layers:
            ins = [f"{b}_{free[b].pop()}" if uses[b] > 1 else b for b in inputs]
            lines.append(f"{kind} {name} {len(ins)} {len(outputs)} {' '.join(ins + outputs)} {params}".rstrip())
            blobs += len(outputs)
            for b in outputs:
                k = uses.get(b, 0)
                if k > 1:
                    free[b] = list(range(k))
                    lines.append(f"Split split_{b} 1 {k} {b} " + " ".join(f"{b}_{i}" for i in range(k)))
                    blobs += k
        return f"7767517\n{len(lines)} {blobs}\n" + "\n".join(lines) + "\n"


def rrdbnet_param(blocks=23):
    """RRDBNet x4 (RealESRGAN_x4plus / RealESRNet_x4plus)."""
    g = Graph()
    x = g.add("Input", [])
    feat = g.conv(x, 3, 64)
    body = feat
    for _ in range(blocks):
        rrdb_in = body
        for _ in range(3):
            dense = [body]
            for i in range(4):
                c = g.add("Concat", dense) if len(dense) > 1 else dense[0]
                dense.append(g.conv(c, 64 + 32 * i, 32, leaky=True))
            x5 = g.conv(g.add("Concat", dense), 192, 64)
            body = g.add("Eltwise", [x5, body], "0=1 -23301=2,2.000000e-01,1.000000e+00")
        body = g.add("Eltwise", [body, rrdb_in], "0=1 -23301=2,2.000000e-01,1.000000e+00")
    body = g.conv(body, 64, 64)
    f = g.add("BinaryOp", [feat, body])
    for _ in range(2):
        f = g.add("Interp", [f], "0=1 1=2.000000e+00 2=2.000000e+00")
        f = g.conv(f, 64, 64, leaky=True)
    f = g.conv(f, 64, 64, leaky=True)
    g.conv(f, 64, 3)
    return g.text("output")


def compact_param(num_conv=32, scale=4):
    """SRVGGNetCompact (realesr-general-x4v3, realesr-animevideov3)."""
    g = Graph()
    x = g.add("Input", [])
    a = g.add("PReLU", [g.conv(x, 3, 64)], "0=64")
    for _ in range(num_conv):
        a = g.add("PReLU", [g.conv(a, 64, 64)], "0=64")
    shuffled = g.add("PixelShuffle", [g.conv(a, 64, 3 * scale * scale)], f"0={scale}")
    up = g.add("Interp", [x], f"0=1 1={scale:.6e} 2={scale:.6e}")
    g.add("BinaryOp", [shuffled, up])
    return g.text("output")


def main(weights, out):
    os.makedirs(out, exist_ok=True)
    open(f"{out}/realesrnet-x4plus.param", "w").write(rrdbnet_param())
    write_bin(state(f"{weights}/RealESRNet_x4plus.pth"), f"{out}/realesrnet-x4plus.bin")
    general = state(f"{weights}/realesr-general-x4v3.pth")
    wdn = state(f"{weights}/realesr-general-wdn-x4v3.pth")
    for dn in DENOISE:
        name = f"{out}/realesr-general-x4v3-dn{dn}"
        open(f"{name}.param", "w").write(compact_param())
        s = float(dn)
        write_bin({k: s * general[k] + (1 - s) * wdn[k] for k in general}, f"{name}.bin")
    open(f"{out}/LICENSE", "w").write(LICENSE)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
