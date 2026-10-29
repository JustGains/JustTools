"""Makes justrmbg's model files from BRIA RMBG-2.0's PyTorch weights (see docs/rmbg.md, "Model files").

BRIA's own ONNX export computes each deformable convolution with gathers over every sampling position: thousands of
small operations and gigabytes of intermediate tensors at 1024 x 1024. These exports compute the same convolution
(to 1e-13 in float64) with bilinear resampling (GridSample) instead:

  rmbg-2.0-fast.onnx       every kernel point of a convolution in one resample and one 1x1 convolution; for GPUs
                           (TensorRT builds its half-precision engine from this file)
  rmbg-2.0-fast-fp16.onnx  the same in half precision, except the sampling grids and GridSample (float32): in half
                           precision a grid coordinate is only good to a quarter pixel
  rmbg-2.0-cpu.onnx        one kernel point at a time, so only one sampled map exists at once: half the memory and
                           twice the speed of BRIA's export on a CPU

All three take pixel_values [1, 3, 1024, 1024] and return alphas [1, 1, 1024, 1024] in [0, 1], like BRIA's export.

Requirements (these exact versions reproduce the hashes justrmbg pins, byte for byte):
  python -m pip install torch==2.11.0 torchvision==0.26.0 transformers==5.5.4 timm==1.0.26 kornia==0.8.2 \
      onnx==1.23.2 onnxsim==0.8.1 onnxconverter-common==1.16.0 onnxruntime==1.24.4 numpy pillow
  and BRIA's files (accept the license at https://huggingface.co/briaai/RMBG-2.0): config.json, birefnet.py,
  BiRefNet_config.py and model.safetensors in one folder.

usage: python make_models.py WEIGHTS_DIR OUT_DIR [--check]
  --check first compares the rewritten convolutions with torchvision's on random data.
Copy the three files beside rmbg-2.0.onnx in justrmbg's model folder; justrmbg uses each only if its SHA-256 matches.
"""
import argparse
import collections
import hashlib
import os
import sys
import time

import numpy as np
import torch
import torch.nn.functional as F

EXPECTED = {
    'rmbg-2.0-fast.onnx': 'd779938f65415623c01adff5016f1ab34cde6855d750b5a11610cd7f934536b2',
    'rmbg-2.0-fast-fp16.onnx': '28454a831e24573b60fbe2e135163a64d132f9b40a3405c550b014c661cd370d',
    'rmbg-2.0-cpu.onnx': '8ab5fa6664ca12cc8592ea4ed9a7af48d4f6782ab239b1776a69793cd40e27fb',
}
SIZE = 1024


def pair(value):
    return tuple(value) if isinstance(value, (tuple, list)) else (value, value)


def deform_conv2d_by_points(input, offset, weight, bias=None, stride=(1, 1), padding=(0, 0), dilation=(1, 1),
                            mask=None):
    """torchvision.ops.deform_conv2d (DCNv2, one offset group) as one bilinear resample and one 1x1 convolution per
    kernel point. Sampling at (y, x) = (out_y * stride - pad + i * dilation + dy, ...) with zeros outside the input
    is grid_sample with align_corners=False and zero padding."""
    _, _, height, width = input.shape
    _, _, kernel_h, kernel_w = weight.shape
    (stride_h, stride_w), (pad_h, pad_w), (dil_h, dil_w) = pair(stride), pair(padding), pair(dilation)
    out_h, out_w = offset.shape[-2:]
    ys = (torch.arange(out_h, dtype=input.dtype, device=input.device) * stride_h - pad_h).view(1, out_h, 1)
    xs = (torch.arange(out_w, dtype=input.dtype, device=input.device) * stride_w - pad_w).view(1, 1, out_w)
    out = None
    for i in range(kernel_h):
        for j in range(kernel_w):
            k = i * kernel_w + j
            y = ys + i * dil_h + offset[:, 2 * k]
            x = xs + j * dil_w + offset[:, 2 * k + 1]
            grid = torch.stack(((2 * x + 1) / width - 1, (2 * y + 1) / height - 1), dim=-1)
            sampled = F.grid_sample(input, grid, mode='bilinear', padding_mode='zeros', align_corners=False)
            if mask is not None:
                sampled = sampled * mask[:, k:k + 1]
            term = F.conv2d(sampled, weight[:, :, i:i + 1, j:j + 1])
            out = term if out is None else out + term
    if bias is not None:
        out = out + bias.view(1, -1, 1, 1)
    return out


def deform_conv2d_batched(input, offset, weight, bias=None, stride=(1, 1), padding=(0, 0), dilation=(1, 1),
                          mask=None):
    """The same convolution with every kernel point at once: one bilinear resample of all K sampling positions
    (laid side by side as one wide image), then one 1x1 convolution over the C*K sampled channels."""
    # plain ints, so the export holds constant shapes (and a convolution weight of known shape)
    batch, channels, height, width = (int(d) for d in input.shape)
    outs, _, kernel_h, kernel_w = (int(d) for d in weight.shape)
    points = kernel_h * kernel_w
    (stride_h, stride_w), (pad_h, pad_w), (dil_h, dil_w) = pair(stride), pair(padding), pair(dilation)
    out_h, out_w = (int(d) for d in offset.shape[-2:])
    dtype, device = input.dtype, input.device
    ys = (torch.arange(out_h, dtype=dtype, device=device) * stride_h - pad_h).view(1, 1, out_h, 1)
    xs = (torch.arange(out_w, dtype=dtype, device=device) * stride_w - pad_w).view(1, 1, 1, out_w)
    # kernel point k = i * kernel_w + j sits at (i * dil_h, j * dil_w)
    ki = (torch.arange(kernel_h, dtype=dtype, device=device) * dil_h).repeat_interleave(kernel_w).view(1, points, 1, 1)
    kj = (torch.arange(kernel_w, dtype=dtype, device=device) * dil_w).repeat(kernel_h).view(1, points, 1, 1)
    pairs = offset.view(batch, points, 2, out_h, out_w)
    y = ys + ki + pairs[:, :, 0]
    x = xs + kj + pairs[:, :, 1]
    grid = torch.stack(((2 * x + 1) / width - 1, (2 * y + 1) / height - 1), dim=-1)  # [N, K, oh, ow, 2]
    grid = grid.permute(0, 2, 1, 3, 4).reshape(batch, out_h, points * out_w, 2)  # [N, oh, K*ow, 2]
    sampled = F.grid_sample(input, grid, mode='bilinear', padding_mode='zeros', align_corners=False)
    sampled = sampled.view(batch, channels, out_h, points, out_w)  # [N, C, oh, K, ow]
    if mask is not None:
        sampled = sampled * mask.view(batch, 1, points, out_h, out_w).permute(0, 1, 3, 2, 4)
    columns = sampled.permute(0, 1, 3, 2, 4).reshape(batch, channels * points, out_h, out_w)  # channel c*K + k
    out = F.conv2d(columns, weight.reshape(outs, channels * points, 1, 1))
    if bias is not None:
        out = out + bias.view(1, -1, 1, 1)
    return out


def swin_layer_forward(self, x, H, W):
    """BasicLayer.forward with its shifted-window attention mask built in NumPy. The input size is fixed in an
    export, so the mask is a constant; built with torch, a stage exactly one window wide has an empty slice that
    the ONNX exporter cannot handle."""
    module = sys.modules[type(self).__module__]
    Hp = int(np.ceil(H / self.window_size)) * self.window_size
    Wp = int(np.ceil(W / self.window_size)) * self.window_size
    img_mask = np.zeros((1, Hp, Wp, 1), np.float32)
    slices = (slice(0, -self.window_size), slice(-self.window_size, -self.shift_size), slice(-self.shift_size, None))
    cnt = 0
    for h in slices:
        for w in slices:
            img_mask[:, h, w, :] = cnt
            cnt += 1
    mask_windows = module.window_partition(torch.from_numpy(img_mask).to(x.device), self.window_size)
    mask_windows = mask_windows.view(-1, self.window_size * self.window_size)
    attn_mask = mask_windows.unsqueeze(1) - mask_windows.unsqueeze(2)
    attn_mask = attn_mask.masked_fill(attn_mask != 0, float(-100.0)).masked_fill(attn_mask == 0, float(0.0)).to(x.dtype)
    for blk in self.blocks:
        blk.H, blk.W = H, W
        x = blk(x, attn_mask)
    if self.downsample is not None:
        x_down = self.downsample(x, H, W)
        Wh, Ww = (H + 1) // 2, (W + 1) // 2
        return x, H, W, x_down, Wh, Ww
    return x, H, W, x, H, W


class Alphas(torch.nn.Module):
    """The final mask in [0, 1], like BRIA's export (input pixel_values, output alphas)."""

    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixel_values):
        return self.model(pixel_values)[-1].sigmoid()


def load(weights, deform):
    from transformers import AutoModelForImageSegmentation
    model = AutoModelForImageSegmentation.from_pretrained(weights, trust_remote_code=True).eval()
    module = sys.modules[type(model).__module__]
    module.deform_conv2d = deform
    module.BasicLayer.forward = swin_layer_forward
    return model


def export(weights, deform, path):
    started = time.time()
    with torch.no_grad():
        torch.onnx.export(Alphas(load(weights, deform)), (torch.zeros(1, 3, SIZE, SIZE),), path,
                          input_names=['pixel_values'], output_names=['alphas'], opset_version=17,
                          do_constant_folding=True, dynamo=False)
    print(f'exported {os.path.basename(path)} in {time.time() - started:.0f}s')


def simplify(source, target):
    import onnx
    import onnxsim
    model = onnx.load(source)
    simplified, ok = onnxsim.simplify(model)
    assert ok, 'onnxsim could not check the simplified model'
    onnx.save(simplified, target)
    ops = collections.Counter(node.op_type for node in simplified.graph.node)
    print(f'{os.path.basename(target)}: {len(model.graph.node)} -> {len(simplified.graph.node)} nodes; '
          f'{ops["GridSample"]} GridSample')


def half_with_float_grids(source, target):
    """Half precision except every node between a GridSample's grid input and the convolutions it comes from,
    and GridSample itself."""
    import onnx
    from onnxconverter_common import float16
    model = onnx.load(source)
    producers = {out: node for node in model.graph.node for out in node.output}
    initializers = {i.name for i in model.graph.initializer}
    keep = set()

    def walk(name):
        node = producers.get(name)
        if node is None or node.name in keep or node.op_type in ('Conv', 'Sigmoid'):
            return
        keep.add(node.name)
        for parent in node.input:
            if parent and parent not in initializers:
                walk(parent)

    for node in model.graph.node:
        if node.op_type == 'GridSample':
            keep.add(node.name)
            walk(node.input[1])
    converted = float16.convert_float_to_float16(model, keep_io_types=True, node_block_list=sorted(keep))
    onnx.save(converted, target)
    print(f'{os.path.basename(target)}: {len(keep)} nodes kept in float32')


def check():
    import torchvision
    torch.manual_seed(0)
    for kernel in (1, 3, 7):
        x = torch.randn(2, 8, 37, 41, dtype=torch.float64)
        offset = torch.randn(2, 2 * kernel * kernel, 19, 21, dtype=torch.float64) * 3
        mask = torch.rand(2, kernel * kernel, 19, 21, dtype=torch.float64) * 2
        weight = torch.randn(5, 8, kernel, kernel, dtype=torch.float64)
        bias = torch.randn(5, dtype=torch.float64)
        args = dict(stride=2, padding=kernel // 2 * 2, dilation=2, mask=mask)
        expected = torchvision.ops.deform_conv2d(x, offset, weight, bias, **args)
        for rewrite in (deform_conv2d_by_points, deform_conv2d_batched):
            difference = (expected - rewrite(x, offset, weight, bias, **args)).abs().max().item()
            assert difference < 1e-9, (rewrite.__name__, kernel, difference)
        print(f'kernel {kernel}: both rewrites match torchvision (float64)')


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as file:
        for block in iter(lambda: file.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('weights')
    parser.add_argument('out')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.check:
        check()
    os.makedirs(args.out, exist_ok=True)
    work = os.path.join(args.out, 'work')
    os.makedirs(work, exist_ok=True)
    fast = os.path.join(args.out, 'rmbg-2.0-fast.onnx')
    export(args.weights, deform_conv2d_batched, os.path.join(work, 'fast-raw.onnx'))
    simplify(os.path.join(work, 'fast-raw.onnx'), fast)
    half_with_float_grids(fast, os.path.join(args.out, 'rmbg-2.0-fast-fp16.onnx'))
    export(args.weights, deform_conv2d_by_points, os.path.join(work, 'cpu-raw.onnx'))
    simplify(os.path.join(work, 'cpu-raw.onnx'), os.path.join(args.out, 'rmbg-2.0-cpu.onnx'))
    for name in ('fast-raw.onnx', 'cpu-raw.onnx'):
        os.remove(os.path.join(work, name))
    os.rmdir(work)
    mismatched = 0
    for name, expected in EXPECTED.items():
        actual = sha256(os.path.join(args.out, name))
        same = actual == expected
        mismatched += not same
        print(f'{name}: {actual} {"(pinned)" if same else "DIFFERENT from the pinned hash; justrmbg will not use it"}')
    return 1 if mismatched else 0


if __name__ == '__main__':
    sys.exit(main())
