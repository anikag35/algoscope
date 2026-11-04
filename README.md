# algoscope

An interactive terminal visualizer for classic algorithms, with step-through
controls and live complexity annotations. Scrub forwards and backwards through
a sort one comparison at a time, and watch the work actually performed next to
the textbook bound for the same `n`.

```
                                                                 ▄▄█│  ALGORITHM
                                                              ▃▇▇███│  Bubble sort
                                                          ▂▂▆▆██████│
                                                       ▁▁▆██████████│  COMPLEXITY
                                                     ▅▅█████████████│  best         O(n)
                                                  ▄█████████████████│  average      O(n²)
                                              ▃▃████████████████████│  worst        O(n²)
                                         ▂▂  ▇██████████████████████│  space        O(1)
                                 ▂    ▆▆ ██  ███████████████████████│  stable       yes
                            ▁▅▅  █    ██ ██  ███████████████████████│
                   ▄▄       ███  █    ██ ██  ███████████████████████│  WORK DONE
              ▄▄ ████       ███  █    ██ ██  ███████████████████████│  comparisons  429
            ▇▇██ ████       ███  █▃▃  ██ ██  ███████████████████████│  writes       518
           ▆████ ████▂▂     ███  ███  ██ ██  ███████████████████████│  step         700 / 1,165
         ▁▁█████ ██████     ███  ███  ██▆██  ███████████████████████│
      ▅  ███████ ██████     ███  ███  █████  ███████████████████████│  BOUNDS (n = 40)
    ▄▄█  ███████ ██████     ████████  █████  ███████████████████████│  n log n ≈    213
  █████  ███████ ██████     ████████  █████▃▃███████████████████████│  n(n-1)/2 =   780
  █████  ███████▂██████ ▇▇  ████████  ██████████████████████████████│
  █████  ██████████████▂██  ████████▆▆██████████████████████████████│
▅▅█████  █████████████████▁▁████████████████████████████████████████│
███████▄▄███████████████████████████████████████████████████████████│
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━────────────────────
Bubble sort  ·  ▶ playing  ·  40 items  ·  speed 3/7  ·  seed 11  ·  ? keys
```

## Install

From source, which needs a Rust toolchain (1.85 or newer):

```bash
git clone https://github.com/anikag35/algoscope && cd algoscope && cargo run --release
```

Or install the binary onto your `PATH`:

```bash
cargo install --path .
```

## Keys

| Key | |
| --- | --- |
| `space` | play / pause |
| `←` `→` or `h` `l` | one step back / forward |
| `pgup` `pgdn` | scrub by a tenth of the run |
| `home` `end` or `g` `G` | jump to the start / end |
| `↑` `↓` or `k` `j` | faster / slower |
| `tab` or `1`–`3` | change algorithm, same array |
| `r` | reshuffle |
| `+` `-` | grow / shrink the array |
| `?` | all keys |
| `q` | quit |

Switching algorithm deliberately keeps the same array, so the comparison counts
of two algorithms are directly comparable rather than being two unrelated runs.

```
algoscope --algorithm insertion --size 120 --seed 42
algoscope --ascii          # ASCII glyphs, no colour
```

## How it works

The interesting problem here is not the sorting, it is the animation state.

The obvious design is to instrument each algorithm so it pauses inline — pass a
tracer, call `tracer.compare(i, j)`, block until the next frame. It reads
naturally and it is a dead end: the *algorithm* owns control flow, so pausing
means parking a thread, and stepping **backwards** is impossible.

So algorithms here never animate themselves. Each one runs to completion
immediately and records every comparison and mutation into an event log:

```rust
pub enum Step {
    Compare   { i: usize, j: usize },
    Swap      { i: usize, j: usize },
    Write     { idx: usize, old: u32, new: u32 },
    MarkSorted{ idx: usize },
}
```

Rendering is then a pure function of `(trace, cursor)`. Stepping forward,
stepping back, scrubbing to an arbitrary position and reverse playback all
collapse into cursor arithmetic over an immutable `Vec<Step>`.

The invariant that buys all of it: **every step carries enough information to
undo itself.** `Swap` is self-inverse for free. `Write` is where it gets
interesting — insertion sort shifts values rather than exchanging them, so
without recording `old` there is no way back. That constraint is baked into the
type, and `tests/properties.rs` holds it honest: for any input, applying every
step forward and then every step backward must return bit-for-bit to the
starting state.

`Playback` owns the trace, the frame and the cursor together, so the rendered
frame cannot drift out of sync with the cursor, and seeks walk from wherever
the cursor already is rather than replaying from the start.

The `algo` module has no dependency on the terminal or on `ratatui`, so the
model is reusable as a library for any other front end.

## Portability

It should look right on a modern terminal and stay readable over SSH on
something ancient, so nothing is assumed. Colour depth and UTF-8 support are
detected once at startup, [`NO_COLOR`](https://no-color.org) is honoured, and
every glyph and colour decision routes through one `Theme`:

- Eighth-block bars degrade to an ASCII ramp, box-drawing borders to `+-|`.
- In monochrome the bar state comes through as reverse and dim video.
- The sidebar is dropped on a narrow terminal and the scrubber on a short one, in that order, because the bars are the point.
- Colour avoids the red/green pairing that red-green colourblind viewers cannot
  separate: settled bars are green, active bars are amber. The magnitude ramp is
  a single blue-to-cyan hue, so it reads by lightness rather than by hue alone.

The array is never resized to fit the terminal, so a resize mid-run cannot
disturb the sort.

## Status

Sorting is done. Next is a grid mode covering BFS, DFS, Dijkstra and A\* — on a
weighted grid rather than an arbitrary node-link graph, which gets the same
algorithmic ground with no layout code and looks considerably better in a
terminal.

## Dependencies

Three: `ratatui`, `crossterm`, `rand`. Argument parsing is 60 lines by hand.

## License

MIT or Apache-2.0, at your option.
