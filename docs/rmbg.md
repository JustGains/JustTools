# JustRMBG

Remove image backgrounds locally with BRIA RMBG-2.0. Every result is a PNG or
WebP with a transparent background, and inputs are always kept.

```sh
justrmbg                                # console launcher
justrmbg portrait.jpg                   # portrait-nobg.png beside the input
justrmbg a.jpg b.png --output cutouts   # several images into a folder
justrmbg photos --output cutouts        # every still image in a folder
justrmbg pose.jpg --picture             # pose-nobg.webp: cropped, padded, at most 400 px tall
justrmbg --check                        # test the runtime without the model
justrmbg --serve                        # keep the model loaded for a program
```

`justrmbg --help` lists providers and output rules. The pinned ONNX Runtime and
model download only with `--download`, from the launcher, or after confirmation
in a terminal. BRIA's weights require a separate license for commercial use.

## Output options

The same options exist as command-line flags and as serve-mode request fields
(camelCase), with the same defaults, so a batch and a server make the same
files:

| Flag | Request field | Default | Effect |
| --- | --- | --- | --- |
| `--format png\|webp` | `format` | the output's extension, else PNG | File format |
| `--crop` | `crop` | off | Crop to every pixel with any alpha |
| `--padding PX` | `padding` | 0 | Transparent room around the subject |
| `--max-width PX`, `--max-height PX` | `maxWidth`, `maxHeight` | none | Scale down (never up) to fit, colors weighted by alpha |
| `--quality N` | `quality` | 80 | WebP quality, 0 to 100 |
| `--alpha-quality N` | `alphaQuality` | 80 | WebP alpha quality: 80 moves alpha by at most a level and encodes about five times faster than lossless alpha |
| `--method N` | `method` | 4 | WebP effort, 0 to 6 |
| `--compression fast\|small` | `compression` | fast | PNG: about 10 ms for a 1 MP cut-out, or about a quarter second for a file a fifth smaller |
| `--picture` | `picture` | | A finished picture: `--crop --padding 10 --max-height 400 --format webp` |

Options given beside `--picture` win; in a request, `"picture": {"maxHeight":
300}` starts from the picture settings and changes one. Default output names
end in `.webp` when the format is WebP.

## Resident serve mode

Loading the model takes seconds. A program that removes many backgrounds can
start one serve process and send it images one at a time:

```sh
justrmbg --serve [--provider EP | --cpu | --gpu] [--model PATH] [--download] [output options]
```

`--serve` cannot be combined with image inputs, `--output`, or `--check`.
Output options given with `--serve` are every request's defaults. Startup
resolves the runtime and model and loads the engine exactly like a normal run,
with the same download rules. It then runs the model once on a blank image,
because the first inference prepares the GPU. After that, stdout receives one
line:

```json
{"ready":true,"provider":"DirectML","model":"rmbg-2.0-fast-fp16","load_seconds":4.4,"pictures":true}
```

`provider` is the provider in use, `model` the model file it runs (see Model
files), `load_seconds` covers all of startup, and `pictures` says requests may
ask for finished pictures. If startup fails, the process exits nonzero with the
error on stderr and never prints the ready line.

Then write one JSON object per line to stdin, with any output options:

```json
{"id":"42","input":"C:/frames/frame-0.webp","output":"C:/cutouts/frame-0.png"}
{"id":"43","input":"C:/frames/frame-1.webp","output":"C:/cutouts/frame-1.webp","picture":true}
```

Each request gets exactly one stdout line, in request order, flushed at once:

```json
{"id":"42","ok":true,"output":"C:\\cutouts\\frame-0.png","seconds":0.117,"stages":{"prepare":0.019,"wait":0.0,"infer":0.073,"write":0.025},"width":1184,"height":864,"bytes":1500428}
{"id":"44","ok":false,"error":"input not found: C:/frames/frame-9.webp"}
```

- The model runs one image at a time. Meanwhile the server reads and decodes
  the next request and saves the previous cut-out, so send requests as they
  come instead of waiting for each answer. `seconds` runs from reading a
  request to its answer, including time behind earlier requests; `stages`
  gives the seconds spent decoding and scaling (`prepare`), waiting for the
  model (`wait`), in the model (`infer`), and making and saving the cut-out
  (`write`). `width`, `height` and `bytes` describe the saved file.
- Command-line batches run through the same three stages.
- Relative paths resolve against the server's working directory, and a
  successful `output` is absolute. An existing output file is replaced
  atomically.
- A request fails, and serving continues, when its input is missing, not a
  file, animated, multi-page, or a GIF; when its output folder does not exist;
  when its output is a directory, the input itself, or a model file; or when an
  output option is out of range.
- A line that is not a JSON object with string `id`, `input`, and `output`
  fields gets `{"id":null,"ok":false,"error":"..."}`, with the `id` echoed when
  it could be read. Blank lines are ignored.
- If Auto's GPU inference fails, the engine switches to CPU for that request
  and every later one, and says so on stderr.
- Stdout carries only protocol lines. Status goes to stderr, almost all of it
  during startup; read or discard stderr so it cannot fill a pipe.
- Closing stdin ends the server with exit code 0.

One process at a time per GPU is right: the model runs one image at a time
anyway, and each process keeps its own copy of the model in GPU memory.

## Model files

BRIA's own ONNX export (`rmbg-2.0.onnx`, the managed download) computes each
deformable convolution with gathers over every sampling position: thousands of
small operations and gigabytes of intermediate tensors. Exports of the same
weights that compute the same convolutions (to 1e-13 in float64) with bilinear
resampling run two to five times faster. justrmbg uses each of these files when
it sits beside the managed model (in the model cache, or `models` beside the
executable) and matches its pinned SHA-256:

| File | Made for | Runs on |
| --- | --- | --- |
| `rmbg-2.0-fast-fp16.onnx` | every kernel point of a convolution in one resample; half precision except the sampling grids | DirectML and CUDA (default) |
| `rmbg-2.0-fast.onnx` | the same in float32 | TensorRT (which builds its own half-precision engine); DirectML and CUDA with `--full-precision` |
| `rmbg-2.0-cpu.onnx` | one kernel point at a time, so only one sampled map exists at once | CPU |
| `rmbg-2.0-fp16.onnx` | BRIA's export in half precision | DirectML and CUDA when the fast files are missing |

Each provider tries its files in that order and ends with BRIA's export, so a
missing or mismatched file only costs speed. CoreML keeps BRIA's files (the new
ones are not measured there). `--model` and `RMBG_MODEL` always use the named
file alone. A file verified once is not hashed again until its size or
modification time changes (`<file>.verified` beside it records the check).

The files are never downloaded. Make them with `scripts/rmbg/make_models.py`
from BRIA's PyTorch weights; with the versions in `scripts/rmbg/README.md` it
reproduces the pinned hashes byte for byte. `rmbg-2.0-fp16.onnx` is made from
the managed model with these exact versions, so it matches its pinned hash:

```sh
python -m pip install onnx==1.23.2 onnxconverter-common==1.16.0 protobuf==7.36.2 numpy==2.4.6
python -c "import onnx; from onnxconverter_common import float16; onnx.save(float16.convert_float_to_float16(onnx.load('rmbg-2.0.onnx'), keep_io_types=True), 'rmbg-2.0-fp16.onnx')"
```

BRIA's export also accepts other sizes in its declared shape, but its graph
only runs at 1024 by 1024, and fixing that size in the session made DirectML
move most of the work to the CPU (24 seconds per image). The new files are
fixed at 1024 by 1024 and run entirely on the GPU.

## Speed

An RTX 4090 and an i9-13900K, serve mode, 25 exercise pictures of about 1 MP;
old and new builds alternated in one sitting:

| | Before | DirectML | TensorRT |
| --- | --- | --- | --- |
| One cut-out, sent and answered | 325 to 362 ms | 117 to 124 ms | 66 to 73 ms |
| 25 cut-outs at once, until the last | 4.5 to 4.7 s | 1.9 to 2.0 s | 0.96 to 1.0 s |
| In the model, one picture | 160 to 180 ms | 73 to 76 ms | 36 to 37 ms |
| Decoding and scaling, then saving | 70 to 80, 90 to 100 ms | 18 to 19, 25 to 27 ms | 10 to 12, 19 to 23 ms |
| Start to ready | 12.6 to 14.3 s | 4.4 to 7.2 s | 9.1 s (the first start builds the engine) |
| One finished picture (WebP) | | 129 ms | 94 ms |

On the CPU, back to back: 14.2 s an image before, 10.8 s with
`rmbg-2.0-cpu.onnx` (CPU timings vary with the machine's load; the same files
measured 14.0 against 7.5 s when it was idle). On the same machine's Intel UHD
770 through DirectML, `rmbg-2.0-fast-fp16.onnx` takes 10.2 s against 19.3 s for
BRIA's half-precision export.

Per inference on the RTX 4090 (ONNX Runtime 1.24, one 1024 by 1024 mask):

| Model file | DirectML | CUDA | TensorRT |
| --- | --- | --- | --- |
| BRIA's export, half precision | 156 ms | 184 ms | |
| BRIA's export, float32 | 261 ms | 251 ms | |
| fast, half precision with float32 grids | 76 ms | 147 ms | |
| fast, float32 | 141 ms | 169 ms | 31 ms (half-precision engine) |

PyTorch's own half-precision model takes 62 ms.

## Quality

Masks against BRIA's PyTorch model in float32 with the same filters (25
exercise pictures, final alpha at each picture's size):

| | IoU at 0.5 (worst) | Mean alpha difference | Edge band difference (worst) |
| --- | --- | --- | --- |
| Before: BRIA's half-precision export, DirectML | 0.9990 (0.9928) | 0.05/255 | 0.98/255 (4.99) |
| DirectML, `rmbg-2.0-fast-fp16.onnx` | 0.9993 (0.9925) | 0.04/255 | 0.70/255 (5.17) |
| TensorRT, `rmbg-2.0-fast.onnx` | 0.9992 (0.9906) | 0.04/255 | 0.77/255 (6.35) |
| CPU, `rmbg-2.0-cpu.onnx` (3 pictures) | 0.9999 | 0.01/255 | 0.28/255 |

The half-precision differences sit on soft edges such as an exercise mat's
lower layer, the same places PyTorch's own half precision differs. Against
human-labelled masks (DIS-VD, 470 pictures of hard objects), the runtimes and
precisions are indistinguishable: mean absolute error 0.0206 to 0.0207 for
float32, half precision, the fast files and TensorRT alike.

The filters matter more. justrmbg scales the picture to 1024 and the mask back
with Lanczos; BRIA's reference pipeline uses bilinear and bicubic. On DIS-VD,
Lanczos for both made 2% less error (0.0202 against 0.0206; bilinear for the
mask was worst at 0.0211), so justrmbg keeps Lanczos. Against BRIA's pipeline
the two differ by IoU 0.995 on average, most on one picture in 25 (0.92),
where they disagree about how much of a mat's dark underside to keep.

## TensorRT and CUDA

TensorRT and CUDA need a GPU build of ONNX Runtime and NVIDIA's libraries in
one folder, named by `ORT_DYLIB_PATH`:

```sh
python scripts/rmbg/make_nvidia_runtime.py "%LOCALAPPDATA%\JustGains\JustTools\cache\onnxruntime\nvidia-1.24.4-windows-x86_64"
set ORT_DYLIB_PATH=%LOCALAPPDATA%\JustGains\JustTools\cache\onnxruntime\nvidia-1.24.4-windows-x86_64\onnxruntime.dll
justrmbg --serve --provider tensorrt
```

The folder holds ONNX Runtime 1.24 with its CUDA and TensorRT providers, CUDA
12, cuDNN 9 and TensorRT 10 (about 5.5 GB with TensorRT's builders for every
GPU generation). TensorRT builds a half-precision engine for each GPU and model
the first time, which took 12 to 27 minutes on an RTX 4090, and keeps it in
the model cache's `tensorrt` folder; later starts take seconds. Auto uses
TensorRT only once an engine has been built (so an explicit `--provider
tensorrt` run comes first), then CUDA, then DirectML, all within the
`ORT_DYLIB_PATH` runtime. Without `ORT_DYLIB_PATH`, Windows uses the managed
DirectML runtime.

## Small devices

BRIA's model needs more memory than a 4 GB board has: on an Orange Pi 5 Pro
(RK3588S, 4 GB) it runs out of memory before its first image. Three things let
a smaller export of the same weights run there:

- **A model with a fixed input size runs at that size.** justrmbg reads the
  size from the model and uses 1024 by 1024 only when the model leaves it open.
- **Below 8 GB of memory**, CPU inference allocates each tensor as it is
  needed instead of keeping an arena and a precomputed memory plan. That is
  slower, but it decides whether an image finishes.
- **On Linux CPUs that mix fast and slow cores** (an RK3588's Cortex-A76 and
  Cortex-A55 cores), justrmbg keeps itself to the fast cores (those with at
  least three quarters of the top core's capacity) and runs one inference
  thread per fast core. Work split across every core waits on the slow ones,
  and that load restarted the Orange Pi. An affinity set with `taskset` or a
  cgroup CPU set is respected: justrmbg runs one thread per allowed core.

The status lines say when either applies. On the Orange Pi's four Cortex-A76
cores, BRIA's weights exported at 768 by 768 with 8-bit matrix weights take
about 10 seconds an image in serve mode with about 2 GB of memory. On 15
exercise pictures their masks overlap the full model's at 1024 by 1024 by 98.7%
on average. At 512 by 512, the model drops parts of exercise mats.

The RK3588's NPU is no help with this model: its runtime cannot run the
deformable convolutions, and the Swin transformer alone takes 2.7 seconds at
512 by 512.
