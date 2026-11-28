# enhance: upscale a wallpaper

`rehue enhance` makes a wallpaper bigger, for a picture or video that's
smaller than your screen. It writes the result named after the source
in the output directory: a PNG for a picture (`wall.webp` becomes
`wall.png`), the same container for a video (`clip.mp4` stays
`clip.mp4`).

```sh
rehue enhance -i wall.webp -o up --upscale 2          # Lanczos, built in
rehue enhance -i wall.webp -o up --upscale 4 --sr     # AI super-resolution
rehue enhance -i clip.mp4 -o up --upscale 2 --sr      # a video
```

## Lanczos

The default. Lanczos resampling is built in, needs nothing else, and
gives the same bytes every time. `--upscale` takes 2 to 8. It keeps
edges as sharp as resampling can, but it can't add detail the picture
doesn't have: a 4x Lanczos upscale looks soft up close.

## Super-resolution

`--sr` runs [Real-ESRGAN](https://github.com/xinntao/Real-ESRGAN)
instead, which fills in plausible detail, so a small picture holds up
at screen size. It also cleans up JPEG blocks and ringing. It needs
[realesrgan-ncnn-vulkan](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan)
installed: on your `PATH`, named by `$REHUE_SR_TOOL`, or given with
`--sr-tool`. It runs on Vulkan, so any GPU works (Intel, AMD, NVIDIA)
and so does a software driver, without CUDA.

- Every model upscales 4x natively, so `--sr` takes `--upscale` 1 to 4;
  rehue reaches smaller factors with Lanczos after the model, and
  `--upscale 1` cleans a picture up at its own size.
- Results can differ slightly between GPUs and drivers, unlike
  Lanczos.

### Models

Pick one with `--model`:

| model | for | what it does |
| --- | --- | --- |
| `photos` (default) | photographs and paintings | the most detail; adds fine texture where the source has none |
| `anime` | drawn pictures with flat areas and outlines | clean lines and flat fills |
| `soft` | photographs, when `photos` looks invented | adds no texture; smooth, close to the source |
| `general` | anything, with `--denoise` | a light model; `--denoise 0` keeps the most grain, `1` smooths the most (default `0.5`) |

```sh
rehue enhance -i photo.jpg -o up --upscale 2 --sr --model soft
rehue enhance -i photo.jpg -o up --upscale 2 --sr --model general --denoise 0
```

`photos` and `anime` come with realesrgan-ncnn-vulkan. `soft` and
`general` come from a models directory.

### Get the soft and general models

They are converted from the official Real-ESRGAN weights by
[`scripts/sr-models.py`](../scripts/sr-models.py). Get them one of
three ways:

- Download `rehue-sr-models.tar.gz` from the
  [releases](https://github.com/fortesoft-co/rehue/releases) and unpack
  it into `~/.local/share/rehue/sr-models` (or
  `$XDG_DATA_HOME/rehue/sr-models`), where rehue looks by default:

  ```sh
  mkdir -p ~/.local/share/rehue/sr-models
  tar -xzf rehue-sr-models.tar.gz -C ~/.local/share/rehue/sr-models
  ```

- With Nix, build them and point rehue at the result:

  ```sh
  export REHUE_SR_MODELS=$(nix build --no-link --print-out-paths github:fortesoft-co/rehue#sr-models)
  ```

- Convert them yourself from the official weights (needs Python with
  torch and numpy); the script's header lists the files and releases:

  ```sh
  python3 -I scripts/sr-models.py weights ~/.local/share/rehue/sr-models
  ```

rehue looks in `--sr-models`, then `$REHUE_SR_MODELS`, then the default
directory.

## Videos

A video (mp4, webm, mkv or mov) is scaled frame by frame, with every
model and with Lanczos, and keeps its audio. It needs ffmpeg: on your
`PATH`, named by `$REHUE_FFMPEG`, or given with `--ffmpeg`.

```sh
rehue enhance -i clip.mp4 -o up --upscale 2 --sr --model general
```

- The output is encoded like [wal](wal.md)'s (H.264 crf 16, or VP9 for
  WebM); `--ffmpeg-args` replaces the codec settings.
- Super-resolution takes a while: about 0.2 s a frame for a 960x540
  clip at 2x on a fast GPU, so a 30-second loop takes a few minutes.
  A progress line shows the frames left.
- Each frame is upscaled on its own, so the models' invented detail can
  shimmer slightly between frames on fine textures such as rain.
  `--model general` with `--denoise 1` or `--model soft` invent the
  least.

## Enhance first, then grade

Upscale the original, then repaint the upscaled one with [wal](wal.md):

```sh
rehue enhance -i wall.webp -o up --upscale 4 --sr
rehue wal -i up/wall.png -s gruvbox-light -H 1
```

Super-resolution invents detail from the colors it's given, so it
works best on the original. Grading afterwards then repaints the new
detail along with the rest, exactly as it would the original.

## Credits

Real-ESRGAN and its weights are by Xintao Wang et al., BSD-3-Clause.
The `soft` and `general` models are converted from the official
RealESRNet_x4plus, realesr-general-x4v3 and realesr-general-wdn-x4v3
weights; their license ships with them.

## Every flag

<!-- gen:flags:enhance:upscaling -->
| flag | value | what it does |
| --- | --- | --- |
| `--upscale` | `UPSCALE` | Target scale factor vs the source. Lanczos takes 2..=8; SR weights are native 4x, so --sr takes 1..=4 (1 = same-size restoration). Needed here or in the config |
| `--sr` |  | AI super-resolution instead of plain Lanczos. Needs realesrgan-ncnn-vulkan: on $PATH, as $REHUE_SR_TOOL, or via --sr-tool |
| `--model` | `photos \| anime \| soft \| general` | SR weights (with --sr): photos = x4plus, anime = x4plus-anime (both shipped with the tool); soft = RealESRNet x4plus, general = realesr-general-x4v3 (both from the models directory) |
| `--denoise` | `LEVEL` | How much --model general smooths: 0, 0.5 or 1; 0 keeps the most grain, 1 smooths the most (absent: 0.5) |
| `--sr-tool` | `TOOL` | Path to the SR tool (with --sr); else $REHUE_SR_TOOL / $PATH |
| `--sr-models` | `DIR` | Directory with the soft and general models; else $REHUE_SR_MODELS, else $XDG_DATA_HOME/rehue/sr-models |
<!-- /gen:flags:enhance:upscaling -->

<!-- gen:flags:enhance:video -->
| flag | value | what it does |
| --- | --- | --- |
| `--ffmpeg` | `FFMPEG` | Path to ffmpeg, for a video; else $REHUE_FFMPEG / $PATH |
| `--ffmpeg-args` | `ARGS` | Encoding flags for ffmpeg, in place of rehue's codec defaults (H.264 crf 16, or VP9 crf 20 for WebM) |
<!-- /gen:flags:enhance:video -->
