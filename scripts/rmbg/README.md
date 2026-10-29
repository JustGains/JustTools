# justrmbg model files

`make_models.py` makes the three model files justrmbg prefers over BRIA's own
ONNX export (`rmbg-2.0.onnx`): see [docs/rmbg.md](../../docs/rmbg.md#model-files).

```sh
python -m pip install torch==2.11.0 torchvision==0.26.0 transformers==5.5.4 timm==1.0.26 kornia==0.8.2 \
    onnx==1.23.2 onnxsim==0.8.1 onnxconverter-common==1.16.0 onnxruntime==1.24.4 numpy pillow
python make_models.py <folder with BRIA's config.json, birefnet.py, BiRefNet_config.py, model.safetensors> out --check
```

BRIA's files come from https://huggingface.co/briaai/RMBG-2.0 after accepting
its license (CC BY-NC 4.0; commercial use needs an agreement with BRIA). With
the versions above, the script reproduces justrmbg's pinned SHA-256 hashes byte
for byte and says so for each file; copy the three files beside
`rmbg-2.0.onnx` (justrmbg's model cache, or `models` beside the executable).
A file with another hash is skipped, so a different toolchain cannot slip in a
different model.

`make_nvidia_runtime.py` assembles one folder with ONNX Runtime's GPU build,
CUDA 12, cuDNN 9 and TensorRT 10 for `ORT_DYLIB_PATH` on Windows, from the
`onnxruntime-gpu`, `torch` (or NVIDIA's `nvidia-*-cu12` wheels) and
`tensorrt-cu12-libs` packages.
