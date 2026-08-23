# 05-02 Spike — what does `read_to_image` cost at a 30 ms cadence, and do the synthetic codec numbers survive contact with Servo?

**Assumptions under test:** `05-RESEARCH.md` **A1** (the phase's highest-risk row) and **A8**.

> **A1** — `OffscreenRenderingContext::read_to_image` is fast enough to run at 30 ms cadence
> alongside the local chrome's own render.
>
> **A8** — the synthetic benchmark frames `05-RESEARCH.md` measured the codec against are
> representative of real Servo output at 1280×800.

`05-CONTEXT.md` § "Required spikes" requires A1 settled **before the frame-pump plan (05-08) is
executed**, exactly as `04-02`'s spike ran in wave 1 and unblocked the authorization-server plans.

**Method:** one throwaway cargo example, `crates/talaria-shell/examples/readback_spike.rs`, compiled
against the workspace's real resolved dependency graph so it links this tree's actual Servo. It
stands up the same arrangement `app.rs` does — a winit window, a `WindowRenderingContext`, a Servo
instance, an egui chrome drawn into the same `glow` context, and **two** webviews on their own
`OffscreenRenderingContext`s: one the local human is looking at (blitted into the chrome every
frame, as `Gui::update` does) and one a remote viewer has leased. It then runs the **pump's shape,
not the screenshot's shape**: the leased tab is `show()`n once for the life of the loop, `paint()` is
called unconditionally on each tick without waiting for `notify_new_frame_ready`
(licensed by `servo-0.4.0/webview.rs:77`), and the readback covers the full surface. The file was
deleted at the end of plan 05-02 Task 2; only these findings survive.

Three shapes, three separate process invocations, 300 ticks each at a 30 ms target — 900 measured
ticks in total.

---

## Verdicts

A1: CONFIRMED

A8: REFUTED — the synthetic frames bracket real Servo output on *time* but not on *bytes*
(3.2× on the photo case), the whole-frame dirty-tile scan costs 2.4× its synthetic figure, and the
premise underneath the headline encode number is wrong for this tree: `encode_screenshot` does
**not** run at `png::Compression::Default`. It already runs at `Compression::Fast` + `FilterType::Sub`,
because that is `png 0.17`'s `Info::default()`. The 21 ms / 175 ms figures describe a configuration
this codebase never executes. D-05-05's *choice* survives and is strengthened; D-05-05's *rationale*
and the "sibling encoder that diverges on three lines" it hands 05-08 do not — see § A8 in full.

---

## Environment, and why there is one number rather than two

| What | Value |
|------|-------|
| Machine | ThinkPad, headless (no display manager, no compositor) |
| GPUs present | Intel Alder Lake-P iGPU (`0000:00:02.0`) and NVIDIA T1200 (`0000:01:00.0`) |
| DRI nodes | `/dev/dri/{card1,card2,renderD128,renderD129}`, all readable by this user via seat ACL |
| X display used | `:95`, `Xvfb :95 -screen 0 1280x800x24` — the harness's own geometry |
| `GL_VENDOR` | `Mesa` |
| `GL_RENDERER` | **`llvmpipe (LLVM 20.1.2, 256 bits)`** |
| `GL_VERSION` | `4.5 (Core Profile) Mesa 25.2.8-0ubuntu0.24.04.2` |
| Build | `cargo build --release --locked --example readback_spike` |

**This machine has no non-llvmpipe path, and that is a finding rather than an omission.** The
evidence, so a later reader does not have to take it on trust:

1. Every run printed `libEGL warning: DRI3 error: Could not get DRI3 device` before reporting
   `llvmpipe`. Xvfb has no DRI3, so Mesa falls back to software rasterisation regardless of the two
   real GPUs sitting on the PCI bus.
2. **Every X display on this machine is an Xvfb.** `:20` and `:21` belong to two Sunshine services,
   `:98` to the self-hosted CI runner. The spike was run on `:21` as a control and reported the same
   `llvmpipe (LLVM 20.1.2, 256 bits)`.
3. Real hardware GL would need either a Wayland compositor (none installed — no `weston`, `cage`,
   `sway`, `labwc` or `kwin_wayland`) or an Xorg holding DRM master. `Xorg` is present but wrapped:
   `/etc/X11/Xwrapper.config` says `allowed_users=console`, and this session is not the console
   session on `seat0`. Taking DRM master would mean starting an X server on the laptop's own tty.

So: **one figure, reported as what it is.** Every number below is software-rendered. Do not present
any of them as evidence about the product's behaviour on the user's real hardware.

**Two consequences that matter more than the missing second column.** Software rendering does not
scale the whole pipeline down uniformly — it moves cost between the two halves, and it moves it in
*opposite* directions:

- **`paint()` is pessimistic under llvmpipe.** Rasterising a scrolling page in software cost 5.1 ms
  mean / 7.8 ms p95 here; a GPU does that work in the compositor.
- **`read_to_image` is very likely optimistic under llvmpipe.** Under software rendering the
  framebuffer already lives in system memory, so `glReadPixels` is close to a `memcpy`. On a real
  GPU the same call is a bus transfer with a pipeline stall, and it can cost *more* than the render
  it follows. The Xvfb readback figure is therefore **not a conservative upper bound** on the
  readback — it is plausibly a lower one.

That asymmetry is exactly why the readback number was worth measuring and why the missing hardware
column becomes a `VERIFICATION.md` manual item rather than a shrug. See § What 05-08 must do.

---

## A1 — the readback at cadence

Three page shapes, 300 ticks each, 30 ms target interval, chrome rendering throughout, leased tab
held `show()`n for the whole loop. All times in **milliseconds**.

### Shape 1 — static text-heavy page

`https://en.wikipedia.org/wiki/Rust_(programming_language)`

| Measurement | n | mean | median | p95 | max | min |
|-------------|--:|-----:|-------:|----:|----:|----:|
| `paint()` | 300 | 0.560 | 0.181 | 0.245 | 111.601 | 0.136 |
| `paint()` after tick 10 | 290 | 0.186 | 0.181 | 0.243 | **0.345** | 0.136 |
| **`read_to_image()`** | 300 | **1.555** | 1.500 | **1.860** | 3.541 | 1.117 |
| `paint()` + `read_to_image()` | 300 | 2.114 | 1.690 | 2.079 | 112.775 | 1.431 |
| chrome frame, pump running | 300 | 2.639 | 2.605 | 3.237 | 4.907 | 1.816 |
| chrome frame, pump idle | 180 | 2.419 | 2.208 | 3.695 | 5.992 | 1.721 |
| dirty-tile scan (off-thread work) | 299 | 0.330 | 0.303 | 0.479 | 0.607 | 0.273 |
| whole tick | 300 | 5.100 | 4.632 | 5.612 | 114.859 | 3.627 |

Dirty tiles per tick: **0 for all 299 comparisons**. 299/299 ticks had nothing to send.
Overruns of the 30 ms interval: **1/300** (tick 0, the 111 ms `paint()` — see below).
`read_to_image` returned the failure case: **0/300**.

### Shape 2 — the same page, driven by a scripted scroll between ticks

`https://en.wikipedia.org/wiki/Rust_(programming_language)`, with a `-40 px` `WheelEvent` delivered
to the leased tab before every tick.

| Measurement | n | mean | median | p95 | max | min |
|-------------|--:|-----:|-------:|----:|----:|----:|
| `paint()` | 300 | 5.386 | 4.943 | 8.002 | 75.832 | 0.795 |
| `paint()` after tick 10 | 290 | 5.103 | 4.919 | **7.811** | 13.240 | 0.941 |
| **`read_to_image()`** | 300 | **1.194** | 1.092 | **1.790** | 3.599 | 0.876 |
| `paint()` + `read_to_image()` | 300 | 6.580 | 6.039 | **9.620** | 77.222 | 1.923 |
| chrome frame, pump running | 300 | 2.434 | 2.276 | 3.438 | 7.449 | 1.854 |
| chrome frame, pump idle | 180 | 2.093 | 2.046 | 2.499 | 3.241 | 1.820 |
| dirty-tile scan (off-thread work) | 299 | 0.109 | 0.087 | 0.300 | 0.572 | 0.026 |
| whole tick | 300 | 9.424 | 8.793 | **13.482** | 80.223 | 5.034 |

Dirty tiles per tick: mean **215.8**, median **231**, p95 **258**, max **260 of 260**.
17/299 ticks were clean; **282/299 changed more than a third of the frame**.
Overruns: **1/300**. `read_to_image` failures: **0/300**.

Note the dirty-tile scan gets *faster* while scrolling (0.109 ms vs 0.330 ms): almost every tile
differs on its first compared row, so the short-circuit fires immediately. The scan's worst case is a
**static** page, where every row of every tile must be compared before the tile can be called clean.

### Shape 3 — image-heavy

`https://upload.wikimedia.org/wikipedia/commons/thumb/f/f0/Ctenophora_edited.jpg/1280px-Ctenophora_edited.jpg`
— a 1280×1156 photographic document, chosen after the obvious candidate
(`Commons:Featured_pictures/Natural_phenomena`) turned out to have all its photographs below the
fold, so its *viewport* was text.

| Measurement | n | mean | median | p95 | max | min |
|-------------|--:|-----:|-------:|----:|----:|----:|
| `paint()` | 300 | 0.380 | 0.195 | 0.257 | 53.238 | 0.140 |
| `paint()` after tick 10 | 290 | 0.199 | 0.195 | 0.255 | 0.434 | 0.140 |
| **`read_to_image()`** | 300 | **1.543** | 1.482 | **1.856** | 3.799 | 1.081 |
| `paint()` + `read_to_image()` | 300 | 1.923 | 1.682 | 2.137 | 54.325 | 1.408 |
| chrome frame, pump running | 300 | 2.557 | 2.528 | 2.934 | 7.814 | 1.853 |
| chrome frame, pump idle | 180 | 2.165 | 2.118 | 2.664 | 3.867 | 1.720 |
| dirty-tile scan (off-thread work) | 299 | 0.333 | 0.300 | 0.476 | 0.658 | 0.278 |
| whole tick | 300 | 4.832 | 4.573 | 5.361 | 56.462 | 3.576 |

Dirty tiles per tick: **0 for all 299**. Overruns: **1/300**. `read_to_image` failures: **0/300**.

### What the distributions say

- **The readback is not the problem.** `read_to_image` cost **1.19–1.56 ms mean and never more than
  1.86 ms at p95** across all three shapes, against a 30 ms budget. Its worst single sample over 900
  ticks was 3.80 ms. It is also remarkably *flat*: the page's content barely moves it, because the
  work is a fixed-size 4 MB `glReadPixels` plus — see `servo-paint-api-0.4.0/rendering_context.rs`,
  `Framebuffer::read_framebuffer_to_image` — a full `clone()` of those 4 MB and a row-by-row vertical
  flip. Roughly 12 MB of memory traffic per call, independent of what the page contains. That
  content-independence is why the figure is stable; it is also the part that will **not** get cheaper
  on a GPU.
- **`paint()` is the variable, and scrolling is what makes it one.** 0.19 ms on a settled page,
  **5.1 ms mean / 7.8 ms p95 while scrolling** — a 27× spread, all of it in the rasteriser. This is
  the half of the tick that a real GPU should improve most.
- **The warm-up tick is real and it is the only overrun.** Every shape overran exactly once, on the
  tick where the lease is first taken: 111 ms (static), 75 ms (scroll), 53 ms (image). Servo does
  deferred work on the first `paint()` after `show()`. **05-08 must expect its first frame after
  attach to be an outlier** and must not treat it as a budget signal for the rate controller.
- **`read_to_image` never failed** — 0 out of 900 sustained ticks across three shapes. The pump does
  not silently drop frames on this path. (Reading the source, the `None` arm is reachable only when
  `RgbaImage::from_raw` rejects a buffer whose length disagrees with the rectangle, so a failure here
  would indicate something structurally wrong rather than transient load.) **T-05-12-B is closed as
  not-observed** under software rendering; it is not closed for hardware, where a lost context or a
  surface resize could plausibly produce one.
- **Holding a second tab `show()`n for the whole loop did not disturb the locally displayed tab.**
  An FNV-1a checksum over the displayed tab's own framebuffer, taken before the lease and again after
  it was released, was **identical in all three runs**. The per-tab-framebuffer invariant
  (`app.rs:2197-2202`) is now proven for the lease shape rather than cited for the screenshot shape.

### T-05-12 — the loop-starvation ratio, which is the point of measuring the chrome beside it

"The frame pump starves the winit loop" is a claim only a ratio can support or refute. Main-thread
cost per 30 ms tick, using post-warm-up means, with the dirty-tile scan excluded because the shipped
design does it on the encoder thread:

| Shape | pump on the loop (`paint`+`read`) | chrome frame | total | pump's share of 30 ms | loop's total share |
|-------|----------------------------------:|-------------:|------:|----------------------:|-------------------:|
| static text | 1.74 ms | 2.64 ms | 4.38 ms | **5.8 %** | 14.6 % |
| scrolling | 6.30 ms | 2.43 ms | 8.73 ms | **21.0 %** | 29.1 % |
| image-heavy | 1.74 ms | 2.56 ms | 4.30 ms | **5.8 %** | 14.3 % |

And the local render's own measured slowdown while a viewer is attached — the thing the user would
actually feel — is small: the chrome frame went from 2.419 → 2.639 ms (static, **+9 %**),
2.093 → 2.434 ms (scrolling, **+16 %**), 2.165 → 2.557 ms (image-heavy, **+18 %**).

**T-05-12 is quantified and does not block.** On a software rasteriser, a viewer attached to a
scrolling page costs the local browser under a fifth of a 30 ms tick and slows the local chrome's own
frame by under a fifth of its own cost. `05-RESEARCH.md` estimated "the difference between the frame
pump costing ~10 % of the loop and ~25 %" if the diff and encode were left on the main thread; the
measured split is 5.8 % / 21 % with them moved off it, which lands inside that estimate.

**One caveat 05-08 must carry:** the scrolling row is the load-bearing one, and its 21 % is *paint*,
not readback. If a real GPU makes `paint()` cheap and `read_to_image` expensive — the asymmetry
described above — the ratio does not simply improve; it redistributes. Re-measure on hardware before
claiming a hardware ratio.

---

## A8 — the codec against real Servo readback

Every encode below ran against buffers this loop actually produced — four sampled frames per shape,
straight out of `read_to_image`, never a synthetic frame. Times are per-encode; bytes are the PNG
payload before any transport framing.

### The finding that changes 05-08: `encode_screenshot` is not what the research thought it was

`05-RESEARCH.md` and `05-CONTEXT.md` D-05-05 both rest on this sentence: *"`encode_screenshot` sets
colour and depth and nothing else, so it runs at `png::Compression::Default` by omission"*, and on
the 20.98 ms / 175.40 ms figures that follow from it.

**The premise is false for `png 0.17.16`, which is what this tree resolves.**
`png::Info::default()` sets `compression: Compression::Fast`, with the comment *"Default to
`deflate::Compression::Fast` and `filter::FilterType::Sub` to maintain backward compatible output"*
(`png-0.17.16/src/common.rs:636-638`), and `Encoder::set_filter`'s own doc says *"The default filter
is `FilterType::Sub`"* (`png-0.17.16/src/encoder.rs:321`). `png::Compression::Default` is a value you
must ask for; it is not what you get by omission.

So `encode_screenshot` has been running at **`Compression::Fast` + `FilterType::Sub`** all along —
which is precisely the configuration D-05-05 chose for the *frame* path.

This is not an inference. The spike encoded the same real frame both ways and the outputs are
**byte-identical**, while an explicitly-set `Compression::Default` on the same buffer is a different
size and roughly 12× slower:

| Frame (static text page) | time | bytes |
|--------------------------|-----:|------:|
| `encode_screenshot`'s exact configuration, as shipped | **1.685 ms** | **465,662** |
| `Compression::Fast` + `FilterType::Sub`, set explicitly | 1.847 ms | **465,662** |
| `Compression::Default` + `FilterType::Sub`, set explicitly | **21.275 ms** | 183,030 |
| `Compression::Best` + `FilterType::Sub` | 77.402 ms | 183,261 |
| `Compression::Fast` + `FilterType::NoFilter` | 4.140 ms | 1,969,933 |

The explicit-`Default` row lands at **21.275 ms**, within 1.4 % of `05-RESEARCH.md`'s 20.98 ms. The
benchmark was measured correctly; it was measuring a configuration this codebase does not use.

**What this does and does not change:**

- **D-05-05's choice stands, and is now cheaper than it looked.** `png` at `Compression::Fast` +
  `FilterType::Sub` over 64×64 tile diffs remains right, and no new codec dependency is needed.
- **The MCP `screenshot` tool is not slow, and never was.** A whole-frame screenshot of a text page
  costs **1.7 ms**, not 21 ms; of a photographic page, **5.5 ms**, not 175 ms. Pitfall 1 in
  `05-RESEARCH.md` ("reusing the screenshot encoder for frames … a real page blows the budget") is
  **not a real hazard in this tree**. Its *conclusion* — a separate encoder for frames — is still
  right, but for a different and smaller reason.
- **The "sibling that diverges on three lines" is a sibling that diverges on one.** Compression and
  filter are already what the frame path wants. The only genuine divergences are that the frame
  encoder emits raw bytes instead of base64, and encodes a tile rectangle instead of the whole
  surface. See § What 05-08 must do.
- **The screenshot path's base64 is now the visible cost, not its compression.** It adds
  0.30 ms / +33 % on a text page and **0.71 ms / +360 KB** on a photographic one (1,079,996 →
  1,439,996 bytes). That is the +33 % `05-RESEARCH.md` already called out, and it is the whole of
  what the frame path saves by going binary.

### Re-measured against the synthetic table

`05-RESEARCH.md`'s two synthetic cases beside the real pages that should bracket them:

| Encoder, 1280×800 whole frame | synthetic *page-like* | **real static text page** | synthetic *photo-like* | **real photographic page** |
|-------------------------------|----------------------:|--------------------------:|-----------------------:|---------------------------:|
| `Fast` + `Sub` — time | 1.80–2.16 ms | **1.52–1.85 ms** | 4.70 ms | **4.83–5.16 ms** |
| `Fast` + `Sub` — bytes | 522 KB | **466 KB** | 3.45 MB | **1.08 MB** |
| `Fast` + `NoFilter` — time | 5.26 ms | **4.14–4.38 ms** | 8.62 ms | **5.64–7.40 ms** |
| `Fast` + `NoFilter` — bytes | 3.01 MB | **1.97 MB** | 4.10 MB | **3.22 MB** |
| explicit `Default` — time | 20.98 ms | **21.21–21.65 ms** | 175.40 ms | **117.7–121.2 ms** |
| explicit `Default` — bytes | 67 KB | **183 KB** | 2.06 MB | **858 KB** |

| Tile-diff operation | synthetic | **real static text** | **real scrolling** | **real photographic** |
|---------------------|----------:|---------------------:|-------------------:|----------------------:|
| Whole-frame dirty-tile scan (260 tiles) | 0.14 ms | **0.330 ms mean / 0.479 p95** | **0.109 ms mean / 0.300 p95** | **0.333 ms mean / 0.476 p95** |
| One 64×64 tile — time | 0.007 ms | **0.007–0.014 ms** | **0.009–0.021 ms** | **0.015–0.036 ms** |
| One 64×64 tile — bytes | 3.9 KB | **413 B** | **1,095–4,175 B** | **1,685 B** |
| Four-tile (128×128) cluster — time | 0.034 ms | **0.024–0.039 ms** | **0.023–0.084 ms** | **0.054–0.076 ms** |
| Four-tile cluster — bytes | 15 KB | **5,054 B** | **4,827–15,456 B** | **7,108 B** |
| Full keyframe — time | 2.164 ms | **1.52–1.85 ms** | **1.32–1.92 ms** | **4.83–5.16 ms** |
| Full keyframe — bytes | 522 KB | **466 KB** | **259–548 KB** | **1.08 MB** |

**Did real pages bracket between the synthetic page-like and photo-like cases, as predicted?**
**On time, yes — almost exactly. On bytes, no.** Every real encode time falls inside or within a few
percent of the synthetic range, so the CPU half of the budget was predicted correctly. The byte half
was not: the synthetic photo-like frame was gradient-plus-noise, which is close to incompressible, so
its 3.45 MB overstates a real photograph by **3.2×** (real: 1.08 MB). In the other direction, the
synthetic page-like frame's explicit-`Default` output (67 KB) *understates* a real text page (183 KB)
by 2.7×, because a real page has far more distinct colour than a hand-drawn one. The dirty-tile scan
is **2.4× slower** than its synthetic figure on a static page — 0.330 ms against 0.14 ms — because a
static page is the scan's worst case: no tile short-circuits, so every row of all 260 tiles is
compared, about 4.3 MB at roughly 10 GB/s.

That is why **A8 is REFUTED**: the frames were representative enough to get the encoder decision
right, and not representative enough to plan wire budgets or a keyframe threshold from. Both of those
now have measured numbers below instead.

### What this means on the wire

Payload at a 30 ms cadence, using the measured `Fast`+`Sub` bytes rather than the synthetic ones:

| Case | measured payload | needs at 30 ms | direct WireGuard (≈350 Mbit/s) | DERP relay (13 Mbit/s) |
|------|-----------------:|---------------:|:-------------------------------|:-----------------------|
| One tile, static page | 413 B | 0.11 Mbit/s | ✅ | ✅ |
| Four-tile cluster, static page | 5.1 KB | 1.3 Mbit/s | ✅ | ✅ |
| Four-tile cluster, scrolling | 15.5 KB | 4.1 Mbit/s | ✅ | ✅ |
| Keyframe, text page | 466 KB | 124 Mbit/s | ✅ | ❌ (≈287 ms per frame) |
| Keyframe, scrolling | 259–548 KB | 69–146 Mbit/s | ✅ | ❌ |
| **Keyframe, photographic page** | **1.08 MB** | **288 Mbit/s** | ⚠️ close to the measured ceiling | ❌ (≈665 ms per frame) |

The photographic keyframe is the one row that moved materially against the plan: at 288 Mbit/s it is
inside the 342–447 Mbit/s a direct path measured on this tailnet, but not comfortably. D-05-06's
degrade-and-report ladder is not a relay-only feature — a full-bleed image page on a direct link can
reach for it too. **Half-res costs 4× fewer bytes and is the rung that answers this**, which is
another reason the ladder's rungs must be reachable from the interactive path and not only from a
relay detection.

**The static cases are the reassuring ones and they are the common ones.** A settled page — text or
photographic — produced **0 dirty tiles on 299 of 299 comparisons in both shapes**. The pump sends
*nothing* on a page nobody is touching, for 0.33 ms of `memcmp` per tick. `05-RESEARCH.md`'s
"static pages then cost one readback and 0.14 ms of `memcmp` per tick and send nothing" is confirmed,
at 0.33 ms rather than 0.14 ms.

### The keyframe threshold, with a measured basis

The scrolling shape is the only one that produces dirty tiles, and it produces almost all of them:
**median 231 of 260 tiles per tick, p95 258, and 282 of 299 ticks over a third of the grid.** Scroll
is a keyframe every tick, exactly as predicted.

Where the crossover actually sits is decided by **per-encode fixed cost, not by bytes**. Bytes are
close to linear in tile count either way — 260 × 1,989 B of individual scroll tiles ≈ 517 KB against
a 548 KB whole-frame keyframe — so the tile path saves nothing on the wire once most of the frame is
dirty, and it pays one envelope header per tile. Time is what separates them: a 64×64 tile encode
costs **~0.02 ms** including its fixed overhead, and a whole keyframe costs **1.5–1.9 ms** on a
page-like frame. They meet at roughly **90 tiles**.

> **Recommended threshold for 05-08: send a keyframe when the dirty-tile count exceeds 90 of 260
> tiles at 1280×800 — about a third of the grid.** Below it, per-tile messages are cheaper in CPU and
> far cheaper on the wire. Above it, the keyframe is no more expensive to produce, no larger to send,
> and one message instead of ninety. Express it as a fraction of the grid, not as the literal 90, so
> a viewport resize does not silently re-tune it.

### The `Send` precondition

`servo::RgbaImage` is `image::RgbaImage` = `ImageBuffer<Rgba<u8>, Vec<u8>>`. The spike asserted this
statically in this tree — `fn assert_send<T: Send>() {}` instantiated at `servo::RgbaImage` — and it
compiled and ran. **The off-thread diff-and-encode design is sound**: the readback buffer can be
moved to an encoder thread, which is where the 0.33 ms scan and the 1.5–5.2 ms encode belong.

---

## What 05-08 must do — the instruction this spike exists to produce

**The Xvfb figure carries the cadence, so the answer is the simple one:**

> **05-08 builds the pump to request a 30 ms cadence unconditionally, on every machine, including
> under the harness's Xvfb.** The harness sustained it with one overrun in 300 ticks in each of three
> shapes, and that one overrun was the first-frame-after-attach outlier rather than a steady-state
> failure. **Do not add an environment check, an Xvfb-specific cadence, or a `TALARIA_*` knob that
> lowers it** — a suite that measures a different cadence from the product measures the wrong thing.

**And the falsifiable conditional, which is still live because the hardware number does not exist:**

> **If any future measurement shows the requested cadence cannot be sustained — under Xvfb or on real
> hardware — 05-08 must not lower the requested cadence. It must let 05-10's degrade ladder lower the
> *delivered* one, and 05-10's e2e latency suite must then assert the *requested* cadence plus the
> ladder's *response* at a rung the measuring machine can actually sustain, with the real-hardware
> figure carried into `VERIFICATION.md` as SC 2's evidence and the two-machine manual check as its
> confirmation.** The product's claim is about a direct WireGuard path on real hardware; software
> rendering is the harness, not the claim.

Four further instructions, each with the measurement behind it:

1. **Expect the first frame after attach to be an outlier and keep it out of the rate controller.**
   Measured at 111.6 ms (static), 75.8 ms (scrolling) and 53.2 ms (photographic) on the tick where
   the lease is taken, against a steady state of 0.19–5.1 ms. Seed the latency EWMA from the second
   frame, or the controller will degrade a healthy link on its first sample.
2. **Do the dirty-tile scan and the PNG encode on the encoder thread, not on the loop.** `Send` is
   confirmed above; on the loop these would add 0.33 ms + 1.5–5.2 ms to a tick that currently spends
   1.74–6.30 ms there.
3. **Write the frame encoder as its own function — but do not describe it as a three-line divergence,
   and do not "fix" `encode_screenshot`.** Its compression and filter already match. Its real
   divergences are raw bytes instead of base64 and a tile rectangle instead of the whole surface.
   D-05-05's two-encoders-for-two-jobs argument still holds; its 21 ms/175 ms justification does not,
   and repeating that number in a code comment would embed a false claim in the tree.
4. **Keyframe above a third of the grid** (≈90 of 260 tiles at 1280×800), per the measured crossover
   above.

## What 05-10 may claim — the ladder's top rung, as a number

**05-10's fastest rung is 30 ms full-res, and nothing faster may be added.**

The measured server-side production floor — the shortest interval at which this machine can actually
produce a frame, main-thread work only, using p95 rather than mean — is:

| Shape | `paint`+`read` p95 | chrome frame p95 | **floor per tick** |
|-------|-------------------:|-----------------:|-------------------:|
| static text | 2.08 ms | 3.24 ms | **≈5.3 ms** |
| photographic | 2.14 ms | 2.93 ms | **≈5.1 ms** |
| **scrolling (worst)** | **9.62 ms** | 3.44 ms | **≈13.1 ms** |

So the 30 ms rung has about **2.3× headroom in the worst measured shape** and ~5.7× in the
interactive ones — on a software rasteriser, with none of that headroom yet spent on the wire. That
is comfortable, and it is also the entire margin: a rung at 15 ms would have essentially none in the
scrolling case, and the scrolling case is the one that produces keyframes. **Keep `05-RESEARCH.md`'s
ladder as written — full-res 30 ms → full-res 60 ms → half-res 60 ms → half-res 120 ms →
passive-only 500 ms — and do not invent a faster top rung on the strength of the readback being
cheap.** The readback is cheap; `paint()` is not, and the wire is not.

## What goes into `VERIFICATION.md` as a manual item

**Measure `read_to_image` at cadence on real GPU hardware.** This repository cannot: every X display
on the build machine is an Xvfb on llvmpipe, and reaching a GPU would need a compositor that is not
installed or an Xorg holding DRM master from the console session. That absence is T-05-12-A's
mitigation working as designed — one honest number with its limitation stated, rather than a
software figure presented as a hardware one — and it is the reason SC 2's evidence must come from the
two-machine manual check rather than from the harness alone.

The specific thing to re-measure, and why it is not a formality: under software rendering the
framebuffer is already in system memory, so the readback is close to a `memcpy` and `paint()` carries
the cost. On a GPU that inverts. The number that could still break this phase's capture model is a
hardware `read_to_image` that is *slower* than the 1.19–1.56 ms measured here, not a `paint()` that is
faster.
