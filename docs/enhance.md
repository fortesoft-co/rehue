# enhance: upscale a wallpaper

`rehue enhance` makes a wallpaper bigger, for a picture that's smaller
than your screen. It writes a PNG named after the source
(`wall.webp` becomes `wall.png`) in the output directory.

```sh
rehue enhance -i wall.webp -o up --upscale 2          # Lanczos, built in
rehue enhance -i wall.webp -o up --upscale 4 --sr     # AI super-resolution
```

## Lanczos

The default. Lanczos resampling is built in, needs nothing else, and
gives the same bytes every time. `--upscale` takes 2 to 8. It keeps
edges as sharp as resampling can, but it can't add detail the picture
doesn't have: a 4x Lanczos upscale looks soft up close.

## Super-resolution

`--sr` runs Real-ESRGAN instead, which fills in plausible detail, so a
small picture holds up at screen size. It needs
[realesrgan-ncnn-vulkan](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan)
installed: on your `PATH`, named by `$REHUE_SR_TOOL`, or given with
`--sr-tool`. It runs on Vulkan, so any GPU works (Intel, AMD, NVIDIA)
and so does a software driver, without CUDA.

- `--model photos` (default) for photographs, `--model anime` for
  drawn and painted pictures with flat areas.
- The model upscales 4x natively, so `--sr` takes `--upscale` 1 to 4;
  rehue reaches smaller factors with Lanczos after the model, and
  `--upscale 1` cleans a picture up at its own size.
- Results can differ slightly between GPUs and drivers, unlike
  Lanczos.

## Enhance first, then grade

Upscale the original picture, then repaint the upscaled one with
[wal](wal.md):

```sh
rehue enhance -i wall.webp -o up --upscale 4 --sr
rehue wal -i up/wall.png -s gruvbox-light -H 1
```

Super-resolution invents detail from the colors it's given, so it
works best on the original. Grading afterwards then repaints the new
detail along with the rest, exactly as it would the original.

## Every flag

<!-- gen:flags:enhance:upscaling -->
| flag | value | what it does |
| --- | --- | --- |
| `--upscale` | `UPSCALE` | Target scale factor vs the source. Lanczos takes 2..=8; SR weights are native 4x, so --sr takes 1..=4 (1 = same-size restoration) |
| `--sr` |  | AI super-resolution instead of plain Lanczos. Needs realesrgan-ncnn-vulkan: on $PATH, as $REHUE_SR_TOOL, or via --sr-tool |
| `--model` | `photos \| anime` | SR weights (with --sr): photos = general x4plus, anime = x4plus-anime |
| `--sr-tool` | `TOOL` | Path to the SR tool (with --sr); else $REHUE_SR_TOOL / $PATH |
<!-- /gen:flags:enhance:upscaling -->
