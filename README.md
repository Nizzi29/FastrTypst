# FasterTypst

A monkeytype-style trainer for typing math formulas in [Typst](https://typst.app),
focused on electrical engineering notation (German/DIN conventions).

FasterTypst is an unofficial community project and is not affiliated with Typst GmbH.
It embeds the Typst compiler as a library to render and check formulas.

Requires Rust 1.95 or newer ([rustup](https://rustup.rs)):

```sh
cargo run --release
```

## How it works

- **Translate mode** shows a rendered formula; type the Typst source for it.
- **Copy mode** shows the Typst source; type it as fast as possible.
- Press **Space** to show the next formula. The clock runs from then until you solve
  it, so time spent away between formulas is not counted. After 30 s without any
  input (keyboard or mouse) the formula stays on screen, but its time is no longer
  counted: you can still solve it or reveal the solution, and wrong answers still
  count, but none of its time reaches the speed statistics.
- Press **Enter** to submit. Answers are checked by typesetting both formulas with the
  embedded Typst compiler and comparing the result, so `a/b` and `frac(a, b)` are
  both correct.
- **Tab** reveals the solution, **Esc** skips.
- **Strict DIN** requires upright `dif`, `upright(j)` and `upright(e)`. When it is
  off, `d x`, `j` and `e` are accepted too.

Formulas are generated procedurally (circuits & phasors, signals & systems,
calculus & linear algebra, electromagnetics). The difficulty level adapts to your
speed and errors. Constructs you are slow at (fractions, `underline`, `integral.cont`,
…) come up more often; see the statistics window.

## Progress and syncing between devices

Each device appends its attempts to its own `attempts-<device>.jsonl` in the data
folder (default `~/.local/share/fastertypst/`). Every log in that folder is merged on
startup and before each formula, and the difficulty level is replayed from the merged
history. Settings such as mode and DIN strictness stay per device, in
`~/.config/fastertypst/config.json`.

To practice on several machines, open **Statistics → Sync between devices** on each
one and point it at the same folder inside a folder synced by Nextcloud, Syncthing,
Dropbox or similar, e.g. `~/Nextcloud/FasterTypst`. This device's log moves there. Since no file is ever
written by two devices, syncing never produces conflicts, even if a device practices
offline. A pre-sync `profile.json` is migrated automatically and kept as
`profile.json.migrated`.

Build the app on each machine instead of syncing the binary:

```sh
cargo install --locked --path .   # installs ~/.cargo/bin/fastertypst
```

If the project folder itself is inside a synced folder, keep cargo's multi-gigabyte
build output out of it with a local `.cargo/config.toml` (ignored by git):

```toml
[build]
target-dir = "/some/path/outside/the/synced/folder"
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Binaries built from this project embed fonts shipped with Typst (Libertinus Serif,
New Computer Modern, DejaVu Sans Mono), which come with their own licenses; see
[typst-assets](https://github.com/typst/typst-assets).
