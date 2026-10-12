# JustRMBG

Remove image backgrounds locally with BRIA RMBG-2.0. Every result is a PNG with
a transparent background, and inputs are always kept.

```sh
justrmbg                                # console launcher
justrmbg portrait.jpg                   # portrait-nobg.png beside the input
justrmbg a.jpg b.png --output cutouts   # several images into a folder
justrmbg photos --output cutouts        # every still image in a folder
justrmbg --check                        # test the runtime without the model
justrmbg --serve                        # keep the model loaded for a program
```

`justrmbg --help` lists providers and output rules. The pinned ONNX Runtime and
model download only with `--download`, from the launcher, or after confirmation
in a terminal. BRIA's weights require a separate license for commercial use.

## Resident serve mode

Loading the model takes several seconds. A program that removes many
backgrounds can start one serve process and send it images one at a time:

```sh
justrmbg --serve [--provider EP | --cpu | --gpu] [--model PATH] [--download]
```

`--serve` cannot be combined with image inputs, `--output`, or `--check`.
Startup resolves the runtime and model and loads the engine exactly like a
normal run, with the same download rules. It then runs the model once on a
blank image, because the first inference prepares the GPU and takes a few
seconds. After that, stdout receives one line:

```json
{"ready":true,"provider":"DirectML","load_seconds":9.1}
```

`provider` is the provider in use, and `load_seconds` covers all of startup. If
startup fails, the process exits nonzero with the error on stderr and never
prints the ready line.

Then write one JSON object per line to stdin:

```json
{"id":"42","input":"C:/frames/frame-0.webp","output":"C:/cutouts/frame-0.png"}
```

Each request gets exactly one stdout line, in request order, flushed at once:

```json
{"id":"42","ok":true,"output":"C:\\cutouts\\frame-0.png","seconds":0.42,"stages":{"prepare":0.05,"infer":0.3,"write":0.06}}
{"id":"43","ok":false,"error":"input not found: C:/frames/frame-9.webp"}
```

- The model runs one image at a time. Meanwhile the server reads and decodes
  the next request and saves the previous cutout, so send requests as they
  come instead of waiting for each answer. `seconds` runs from reading a
  request to its answer, including time behind earlier requests; `stages`
  gives the seconds spent decoding (`prepare`), in the model (`infer`), and
  blending and saving (`write`).
- Relative paths resolve against the server's working directory, and a
  successful `output` is absolute. Output is always PNG, compressed for speed:
  files are about a fifth larger than a normal run's, written in a few
  milliseconds instead of a quarter second. An existing output file is
  replaced atomically.
- A request fails, and serving continues, when its input is missing, not a
  file, animated, multi-page, or a GIF; when its output folder does not exist;
  or when its output is a directory, the input itself, or the model.
- A line that is not a JSON object with string `id`, `input`, and `output`
  fields gets `{"id":null,"ok":false,"error":"..."}`, with the `id` echoed when
  it could be read. Blank lines are ignored.
- If Auto's GPU inference fails, the engine switches to CPU for that request
  and every later one, and says so on stderr.
- Stdout carries only protocol lines. Status goes to stderr, almost all of it
  during startup; read or discard stderr so it cannot fill a pipe.
- Closing stdin ends the server with exit code 0.

On an RTX 4090 that is also running a game, ten requests sent at once are all
answered after about 3 seconds and a single request after about 0.4. One
process at a time per GPU is right: DirectML runs one image at a time anyway,
and each process keeps its own copy of the model in GPU memory.

## Half precision on the GPU

GPU runs look for `rmbg-2.0-fp16.onnx` beside the managed model, in the model
cache or the `models` folder next to the executable. When it is there and
matches its pinned SHA-256, the GPU runs it instead of the full model:

| | Full model | Half precision |
| --- | --- | --- |
| One image on DirectML (RTX 4090) | 0.46 to 0.5 s | 0.28 s |
| GPU memory | 12.4 GB | 8.2 GB |
| Masks | | at least 99.95% overlap, alpha 0.02/255 apart on average |

CPU runs, `--model`, and `RMBG_MODEL` use the full or named model, and a
GPU-to-CPU fallback switches to the full model. The status line says
"(half precision)" when it is used.

The file is never downloaded. Make it from the managed model with these exact
versions, so it matches the pinned hash:

```sh
python -m pip install onnx==1.23.2 onnxconverter-common==1.16.0 protobuf==7.36.2 numpy==2.4.6
python -c "import onnx; from onnxconverter_common import float16; onnx.save(float16.convert_float_to_float16(onnx.load('rmbg-2.0.onnx'), keep_io_types=True), 'rmbg-2.0-fp16.onnx')"
```

The model also accepts other sizes in its declared shape, but its graph only
runs at 1024 by 1024, and fixing that size in the session made DirectML move
most of the work to the CPU (24 seconds per image). ONNX Runtime's CUDA
provider was no faster than DirectML here.

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
  and that load restarted the Orange Pi. An affinity set with `taskset` is
  only narrowed.

The status lines say when either applies. On the Orange Pi's four Cortex-A76
cores, BRIA's weights exported at 768 by 768 with 8-bit matrix weights take
about 10 seconds an image in serve mode with about 2 GB of memory. On 15
exercise pictures their masks overlap the full model's at 1024 by 1024 by 98.7%
on average. At 512 by 512, the model drops parts of exercise mats.

The RK3588's NPU is no help with this model: its runtime cannot run the
deformable convolutions, and the Swin transformer alone takes 2.7 seconds at
512 by 512.
