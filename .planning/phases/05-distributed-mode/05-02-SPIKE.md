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
