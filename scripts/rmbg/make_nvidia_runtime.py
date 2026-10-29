# Assembles one folder with everything ONNX Runtime needs for CUDA and TensorRT on Windows, for justrmbg's
# ORT_DYLIB_PATH (see docs/rmbg.md): onnxruntime-gpu's runtime and providers, the CUDA 12 and cuDNN 9 libraries
# (from PyTorch's wheel; NVIDIA's nvidia-*-cu12 wheels have the same files) and TensorRT 10 (tensorrt-cu12-libs).
#   python -m pip install torch==2.11.0 --index-url https://download.pytorch.org/whl/cu128
#   python -m pip install onnxruntime-gpu==1.24.4 tensorrt-cu12-libs==10.16.1.11
# usage: python make_nvidia_runtime.py OUTDIR      (run with the Python that has those packages)
import os
import shutil
import sys

import onnxruntime
import tensorrt_libs
import torch

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
capi = os.path.join(os.path.dirname(onnxruntime.__file__), 'capi')
torch_lib = os.path.join(os.path.dirname(torch.__file__), 'lib')
trt = os.path.dirname(tensorrt_libs.__file__)
wanted = {
    capi: ['onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'onnxruntime_providers_cuda.dll',
           'onnxruntime_providers_tensorrt.dll'],
    torch_lib: ['cudart64_12.dll', 'cublas64_12.dll', 'cublasLt64_12.dll', 'cufft64_11.dll', 'cudnn64_9.dll',
                'cudnn_adv64_9.dll', 'cudnn_cnn64_9.dll', 'cudnn_engines_precompiled64_9.dll',
                'cudnn_engines_runtime_compiled64_9.dll', 'cudnn_graph64_9.dll', 'cudnn_heuristic64_9.dll',
                'cudnn_ops64_9.dll', 'nvrtc64_120_0.dll', 'nvrtc-builtins64_128.dll', 'nvJitLink_120_0.dll'],
    trt: sorted(f for f in os.listdir(trt) if f.endswith('.dll')),
}
total = 0
for folder, names in wanted.items():
    for name in names:
        source = os.path.join(folder, name)
        target = os.path.join(out, name)
        if not os.path.exists(target) or os.path.getsize(target) != os.path.getsize(source):
            shutil.copy2(source, target)
        total += os.path.getsize(target)
print(f'{out}: {sum(len(v) for v in wanted.values())} files, {total / 2 ** 30:.2f} GiB; onnxruntime {onnxruntime.__version__}, '
      f'torch {torch.__version__} (CUDA {torch.version.cuda}, cuDNN {torch.backends.cudnn.version()})')
