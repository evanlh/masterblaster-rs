# 025: Command Registry, Control Channel, and MCP Server

Created: 20261013
Updated: 20261013

## Status

- [ ] Phase 1: command registry in `mb-master`
  - [ ] `CommandSpec`, `Controller::execute(id, params)`, `command.list`
  - [ ] Typed params (serde with `deny_unknown_fields`) and a JSON Schema for each command
  - [ ] Commands for the existing `Controller` API and `Edit` variants
  - [ ] Query commands: `song.inspect`, `pattern.inspect`, `machine.inspect`, `graph.inspect`
  - [ ] Move undo/redo from `GuiState` into `Controller`
- [ ] Phase 2: headless scripting from `mb-cli`
  - [ ] `mb-cli serve`: JSON lines on stdio
  - [ ] `mb-cli run script.jsonl`
- [ ] Phase 3: UI commands
  - [ ] Stable string ids for every `EditorAction`, plus a catalog table
  - [ ] `ui.inspect` (a serde snapshot of `GuiState`) and a whitelisted `ui.set`
- [ ] Phase 4: desktop control channel
  - [ ] Loopback TCP transport thread → mpsc → main-thread drain
  - [ ] Waking the event loop through `EventLoopProxy`
  - [ ] `ui.action`, `ui.key`, `ui.screenshot`, `ui.elements`
- [ ] Phase 5: MCP server (`crates/mb-automation`, `mb-cli mcp [--bridge]`)
- [ ] Phase 6: tests (registry, hostile params, a shared `Driver`, MCP tool annotations)

## Problem

Today four separate surfaces change or read state, and none of them can be reached from
outside the process:

| Surface                           | Where                         | Reachable by                                  |
|-----------------------------------|-------------------------------|-----------------------------------------------|
| `EditorAction` (24 variants)      | `src/ui/input.rs`             | imgui key polling, `App::inject_actions`      |
| `Edit` (3 variants)               | `crates/mb-ir/src/edit.rs`    | `Controller::apply_edit`, the undo stack      |
| `Controller` methods              | `crates/mb-master/src/lib.rs` | GUI code, `mb-cli` with hand-parsed arguments |
| `GuiState` (cursor, view, undo)   | `src/ui/mod.rs`               | GUI code only                                 |

The consequences:

- No agent, script or test can drive the app or a headless session through one stable interface.
- The GUI tests need a real window (`gui_tests.rs`, `harness = false`, main thread).
- The parity work in `~/code/artcraft-parity.md` needs scripted renders of a machine with
  parameter changes at given ticks. There's no command surface to send those scripts through.
- Undo lives in `GuiState`, so an edit made outside the GUI couldn't be undone.

## How PhotoCraft and FilmCraft do it

Both apps share one architecture. File references below point into `~/code/filmcraft` unless
marked `photocraft:`.

```
egui UI ──────┐
CLI ──────────┤                          crates/engine (no UI deps)
control TCP ──┼──► Session::execute(id, params: Value) -> Result<Value>
MCP (rmcp) ───┘        │
                       └─ commands::find(id) → CommandSpec { enabled, run }
```

### 1. Every action is a command

`crates/engine/src/commands.rs`:

```rust
type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

pub struct CommandSpec {
    pub id: &'static str,                 // "sequence.addEdit", following the target app's menus
    pub label: &'static str,
    pub menu: &'static [&'static str],    // ["Sequence"]; empty means not in a menu
    pub shortcut: Option<&'static str>,   // "Cmd+K"
    pub params: &'static str,             // a free-form "JSON-ish" description for agents
    pub enabled: Enabled,                 // Err("no clips selected") becomes "disabled: …"
    pub run: Run,
    pub journal: bool,                    // false for read-only queries
}
pub fn command_specs() -> &'static [CommandSpec]  // OnceLock<Vec<_>>, built once
pub fn find(id: &str) -> Option<&'static CommandSpec>
```

- `cmd!` and `query!` macros keep each entry to one line. Queries are commands too
  (`sequence.inspect`, `command.list`), so every transport gets them for free.
- `Session::execute` checks `enabled`, then runs `run` inside `catch_unwind`. A panic becomes
  `"internal error in <id>"` and the session survives.
- Enabled checks consider either the selection or explicit targets. A command acting on the
  selection also accepts `{"clips":[id]}`, so an agent doesn't have to select first.
- The UI's menus, shortcuts and buttons all dispatch by id. PhotoCraft's menu catalog uses the
  same ids, which is how `cargo xtask parity` counts live menu items.
- Pure view state (zoom, panels, tool) is handled by **UI commands** in the UI crate
  (`menus::invoke`). `engine.execute` routes to both kinds.

### 2. The desktop control channel

- **Transport** (`apps/filmcraft/src/control_server.rs`, ~290 lines): a `std::net::TcpListener`
  on `127.0.0.1`, one thread per connection, no tokio. Each request is one JSON line,
  `{id, method, params}`, and each reply is `{id, ok, result | error}`.
- **Hand-off to the UI thread.** The transport sends
  `ControlRequest { method, params, reply: Sender<Value> }` over an mpsc channel and calls
  `ctx.request_repaint()`. It then blocks on `reply.recv_timeout(60 s)`.
- **Dispatch on the UI thread.** `FilmcraftApp::drain_control` runs once per frame and calls
  `control::handle(app, ctx, &req)`, which returns an `Outcome`:

  ```rust
  pub enum Outcome {
      Done(Value),
      Retry(String),        // e.g. the element isn't on screen yet; retried each frame for up to 3 s
      AfterInput,           // reply once the queued synthetic input has been consumed
      Screenshot { path, crop },
  }
  ```
- **Methods** (`crates/ui-egui/src/control.rs`, ~630 lines):

  | Method                                              | Purpose                                                        |
  |-----------------------------------------------------|----------------------------------------------------------------|
  | `engine.execute`, `ui.menu.invoke`                  | Run any command                                                 |
  | `engine.commands`, `ui.menu.list`                   | List commands with their enabled state; list the menu tree      |
  | `ui.inspect`                                        | The serde UI state                                              |
  | `ui.set`                                            | Change whitelisted UI fields, validated before anything changes |
  | `ui.elements {prefix}`                              | Registered widgets: id, label, rect                             |
  | `ui.click`, `ui.drag`, `ui.scroll`, `ui.key`, `ui.type` | Synthetic input                                             |
  | `ui.screenshot {path?, panel?}`                     | PNG of the window or one panel                                  |
  | `ui.playback`, `perf.stats`, `app.quit`             | Playback control, performance counters, quit                    |

- **Synthetic input.** Events are queued on the app and fed in through eframe's
  `raw_input_hook`. Pointer events go **one per frame**, so egui sees a real drag rather than a
  click.
- **The automation registry** (`crates/ui-egui/src/automation.rs`). Every interactive widget calls
  `auto.add(id, rect, label)` each frame, with hierarchical ids such as `timeline.track.V1.lock`
  and `tools.Razor`. Lookups use the *previous* frame's list, because handlers run between frames.
- **Hardening:**
  - A line that isn't a JSON object with a string `method` gets an error and the connection is
    closed, so a browser's cross-origin `fetch` can't smuggle in a command.
  - Lines are limited to 4 MiB and there are at most 16 connections.
  - PhotoCraft adds a 256-bit token. The first frame must be `auth`, and the token file is
    created with mode `0600`. It also adds read and write root directories for file paths, a 30 s
    idle timeout and a 256-step batch limit (`photocraft: crates/automation/src/security.rs`).
- **Platform pitfalls they hit:**
  - An occluded window (or a Wayland window without frame callbacks) stops drawing, so requests
    time out.
  - macOS App Nap throttles a hidden app, so they opt out.
  - Driving the app must never steal keyboard focus from the user.

### 3. Headless use

- `filmcraft-cli run script.jsonl` runs the same commands with no window.
- `photocraft-cli serve` uses the same envelope over stdio (or loopback TCP) on one long-lived
  session. It's the fastest path for scripts, and supports `batch {steps, stopOnError}`.
- **UI tests run headless.** `egui_kittest` hosts the real app. A `Driver` sends
  `ControlRequest`s through the same handler and steps frames until the reply arrives:
  `d.exec(cmd, params)`, `d.ok(method, params)`, `d.click(id)`, `d.frames(n)`.

### 4. MCP (`crates/automation`, its own crate)

- **Dependencies:** `rmcp` (the official SDK; features `server`, `macros`, `transport-io`),
  `tokio`, `schemars`, `base64` and `image`. tokio is confined to this crate.
- **Backends:**

  ```rust
  pub enum Backend {
      Headless(Arc<Mutex<Session>>),  // in-process; commands run on spawn_blocking
      Bridge(Arc<BridgeClient>),      // forwards to `app --control <port>` over the JSON-lines channel
  }
  ```
- **Tools:**
  - A generic core shared across the craft apps: `command_list`, `command_run`,
    `command_batch`, `doc_inspect`, `render_preview`.
  - Domain inspects (`sequence_inspect`, …).
  - `ui_*` tools that work only in bridge mode. In headless mode they return an error explaining
    how to start the bridge.
  - Images come back as MCP image content (base64 PNG).
- **Conventions** (`docs/agents.md`):
  - Every tool has a title and the four hints (read-only, destructive, idempotent, open-world),
    and a test checks them.
  - Unknown arguments are rejected with `-32602`, naming the accepted ones.
  - A failed command returns an `isError` result. A line that isn't JSON gets `-32700` and the
    server keeps serving. A panic becomes `internal error: …`.
  - Resources: `app://document` and `app://commands`.
- **Bridge client** (`bridge.rs`): accepts loopback addresses only and keeps one connection,
  reconnecting on failure.
  - Only *read* tools retry after a transport failure. An edit is never resent; the reply says it
    "may have completed".
  - The client timeout (55 s) is below the server's (60 s), so the caller sees the bridge's
    error rather than a bare timeout.
- **Long jobs:**
  - `wait: false` returns `{job}` immediately, and the caller polls `jobs.list`.
  - With `wait: true`, the server sends MCP progress notifications, and a cancel request stops
    the job.
- **Registration:** `.mcp.json` in the repo registers both a headless server and a bridge
  server.

### 5. What not to copy

- **A free-form `params` string.** FilmCraft reads named targets back out of it by substring
  (`spec.params.contains("\"clips\":[id]")`). Typed parameter structs with a generated schema
  avoid this, and give MCP clients real schemas.
- **A copy of `Driver` in every UI test file** (around 40 files in FilmCraft). Put one in
  `tests/common/`.
- **`ui.set` as a single unbounded schema.** It grew to dozens of nested fields. Make UI state
  changes small UI commands instead.

## Design for masterblaster

### Mapping

| Today                                       | Becomes                                                                                                                                       |
|---------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------|
| `Controller::load_mod/load_bmx/new_song`    | `file.open {path}`, `file.new {channels}`                                                                                                     |
| `Controller::play/stop/play_pattern`        | `transport.play`, `transport.stop`, `transport.playPattern {track, clip}`                                                                      |
| `Controller::render_to_wav/render_pattern_to_wav` | `render.wav {path, seconds, sampleRate?, track?, clip?}` → `{path, frames, peak, rms}`                                                  |
| `Edit::SetCell`                             | `pattern.setCell {track, clip, row, column, cell}`                                                                                            |
| `Edit::SetNodeBypass`, `toggle_track_mute`  | `machine.bypass {node, bypassed}`, `track.mute {track}`                                                                                       |
| `Edit::SetSeqEntry`, `set/remove_seq_entry` | `sequence.setEntry {track, beat, clip?}`                                                                                                      |
| `Machine::set_param` (no `Edit` yet)        | `machine.setParam {node, param, value}`: needs a new `Edit::SetNodeParam` (listed as not done in 006)                                         |
| (none)                                      | `song.inspect`, `pattern.inspect {track, clip, rows?}`, `machine.inspect {node}`, `graph.inspect`, `transport.position`, `command.list`        |
| `UndoStack` in `GuiState`                   | `edit.undo`, `edit.redo` on the `Controller`                                                                                                  |
| `EditorAction::*`                           | UI commands: `editor.moveCursor {drow, dchannel, dcolumn}`, `editor.enterNote {note}`, `view.pattern`, `editor.copy`, …                       |

The command ids are the masterblaster vocabulary. Where Buzz has a named action, use its name
in `label`, so that the UI parity catalog (artcraft-parity.md §4.6) can match labels the way
FilmCraft's menu diff does.

### Phase 1: the registry in `mb-master`

`mb-master` is already the std-only headless API, so the registry goes there. `mb-ir` and
`mb-engine` stay `no_std` and don't change. New dependencies, in `mb-master` only:
`serde`, `serde_json` and `schemars`.

Commands are grouped into modules by the domain they act on, one small function per command:

```
crates/mb-master/src/commands/
  mod.rs          // CommandSpec, registry(), find(), execute(), the command! macro
  error.rs        // CommandError { Unknown, Disabled{id, why}, BadParams{id, msg}, Failed, Internal }
  enabled.rs      // has_song, can_undo, … (each fn(&Controller) -> Result<(), String>)
  file.rs         // file.open, file.new
  transport.rs    // transport.play, transport.stop, transport.playPattern, transport.position
  render.rs       // render.wav
  pattern.rs      // pattern.setCell, pattern.inspect
  sequence.rs     // sequence.setEntry, sequence.inspect
  machine.rs      // machine.setParam, machine.bypass, machine.inspect
  edit.rs         // edit.undo, edit.redo
  inspect.rs      // song.inspect, graph.inspect, command.list
```

Parameters are typed, so we never parse strings by hand:

```rust
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: Kind,                              // Edit | Query | Transport
    pub shortcut: Option<&'static str>,
    pub schema: fn() -> schemars::Schema,        // JSON Schema of the params struct
    pub enabled: fn(&Controller) -> Result<(), String>,
    pub run: fn(&mut Controller, Value) -> Result<Value, CommandError>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetParam { node: u16, param: u16, value: i32 }

fn set_param(c: &mut Controller, p: SetParam) -> Result<Value, CommandError> {
    let edit = c.machine_param_edit(p.node, p.param, p.value)?;   // validates against ParamInfo
    Ok(json!({ "undo": c.apply_undoable(edit) }))
}

command!("machine.setParam", "Set Machine Parameter", Edit, SetParam, has_song, set_param)
```

The `command!` macro generates a `run` that deserializes into the struct (any error becomes
`BadParams`), calls the handler and serializes the result. Each handler is a small function
taking a typed struct, so it can be tested without JSON.

`Controller::execute(id, params)` looks up the command, checks `enabled` and runs it inside
`catch_unwind`. Edits made during playback still go through the existing SPSC ring to the audio
thread (`apply_edit` already routes between the playing and stopped cases). **The audio thread
never runs command code.**

**Undo moves into `Controller`.** `set_seq_entry` and `remove_seq_entry` already return
`(Edit, Edit)` pairs. Generalize that: every edit command records `(redo, undo)` on a
`Controller`-owned stack, and `GuiState::undo_stack` is removed. The UI, scripts and agents then
share one history.

### Phase 2: headless scripting (`mb-cli serve`, `mb-cli run`)

- `mb-cli serve` reads JSON lines on stdin and writes replies on stdout, all on one long-lived
  `Controller`. Its methods are `engine.execute`, `engine.commands`, `batch {steps, stopOnError}`
  and `methods`.
- `mb-cli run script.jsonl` runs the same script non-interactively and exits with 1 if any step
  fails.
- Keep the existing `mb-cli file.mod [--wav] [--pattern]` form working. Internally it becomes
  `file.open` followed by `render.wav`.
- Neither form needs tokio or a window. **This is the main entry point for the parity oracle
  scripts** (artcraft-parity.md §4.2): the same parameter-change script file runs through
  masterblaster here and through the Buzz oracle host.

```sh
printf '%s\n' \
  '{"id":1,"method":"engine.execute","params":{"command":"file.open","params":{"path":"tests/fixtures/bmx/tribal-60.bmx"}}}' \
  '{"id":2,"method":"engine.execute","params":{"command":"machine.setParam","params":{"node":3,"param":0,"value":200}}}' \
  '{"id":3,"method":"engine.execute","params":{"command":"render.wav","params":{"path":"out.wav","seconds":10}}}' \
  | cargo cli serve
```

### Phase 3: UI commands

- Give every `EditorAction` a stable id in a single table, `src/ui/commands.rs`, with these
  columns: id, label, default key, `EditorAction` constructor and parameter type. This one table
  then serves as:
  - the key-binding source for design 021 (configurable keybindings);
  - the UI parity catalog from artcraft-parity.md §4.6;
  - the dispatch table for `ui.action {id, params}`.
- An exhaustive-match test makes sure every `EditorAction` variant has an entry.
- `process_actions(gui, &[EditorAction])` is already a function over `GuiState` with no imgui
  dependency. Most UI behaviour can therefore be tested **without any window**: build a
  `GuiState`, dispatch actions by id, then assert on `ui.inspect` output.
- `ui.inspect` returns `GuiStateView`, a serde struct built from `GuiState`: center view,
  cursor, selection, edit mode, octave and step, selected track and sequence index, status line.
  The caches stay out of it.
- `ui.set` covers only `{centerView, selectedTrack, cursor}`, validated before anything changes.
  Anything else should be an action.

### Phase 4: the desktop control channel

`src/control/` in the GUI crate. It uses only std, mirroring FilmCraft's transport:

| Piece        | Design                                                                                                                                                                       |
|--------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Start        | `masterblaster --control <port>` (or `MB_CONTROL_PORT`). Off by default. Binds `127.0.0.1` only.                                                                             |
| Transport    | `control/server.rs`: a thread per connection, bounded lines (1 MiB), at most 8 connections, rejecting any line that isn't a request (error, then close), 60 s reply timeout. |
| Wake-up      | Use `EventLoop::<UserEvent>::with_user_event()`. The transport calls `proxy.send_event(UserEvent::Control)`, and `user_event` sets `needs_redraw`. Today's 250 ms idle poll would otherwise add up to 250 ms of latency per request. |
| Dispatch     | `control/handle.rs`: `fn handle(app: &mut App, req: &ControlRequest) -> Outcome`. It's drained once per frame at the top of `App::render_frame`, before `build_ui`. `Outcome` has the same variants as FilmCraft's. |
| Engine calls | `engine.execute` → `gui.controller.execute(..)`, followed by `gui.invalidate_caches()`.                                                                                     |
| UI calls     | `ui.action`, `ui.inspect`, `ui.set`, `ui.key {key}`, `ui.screenshot {path?}`, `ui.elements {prefix?}`, `app.quit`.                                                          |
| `ui.key`     | Pushed into imgui io (`io.add_key_event`) before `new_frame`. One key event per frame, the same pacing as FilmCraft's pointer events.                                         |
| Screenshot   | `Outcome::Screenshot`. After the next `swap_buffers`, call the existing `App::capture_pixels` and write the PNG. Move `write_png` out from behind the `test-harness` feature. |
| Elements     | `gui.auto: ElementRegistry`. Panels call `auto.add(id, ui.item_rect_min(), ui.item_rect_max(), label)` after each widget, with ids such as `pattern.cell.<row>.<ch>.<col>`, `seq.cell.<track>.<row>`, `graph.node.<id>` and `transport.play`. Low priority, because `ui.action` covers most needs without pixel clicks. |

Prefer **action-level control** (`ui.action`) to pixel clicks. imgui has no equivalent of eframe's
`raw_input_hook`, and pixel-level synthetic mouse input is where most of FilmCraft's control code
went.

### Phase 5: MCP (`crates/mb-automation`)

- **Location:** a new crate, `crates/mb-automation`, holding `rmcp`, `tokio`, `schemars` and
  `base64`. It's exposed as `mb-cli mcp` (headless, with optional `--song <path>`) and
  `mb-cli mcp --bridge 127.0.0.1:<port>`. tokio never enters `mb-master` or the GUI.
- **Backends:**

  ```rust
  enum Backend { Headless(Arc<Mutex<Controller>>), Bridge(Arc<BridgeClient>) }
  ```

**Tools** (snake_case, all annotated):

| Tool                                                                    | Mode   | Read-only | Maps to                                                                         |
|-------------------------------------------------------------------------|--------|-----------|---------------------------------------------------------------------------------|
| `command_list {filter?, enabled_only?}`                                 | both   | yes       | `command.list` (includes each command's JSON Schema)                            |
| `command_run {id, params}`                                              | both   | no        | `execute`                                                                       |
| `command_batch {steps, stop_on_error?}`                                 | both   | no        | runs the steps in a loop                                                        |
| `song_inspect`                                                          | both   | yes       | `song.inspect`: tracks, clips, sequence, tempo                                  |
| `pattern_inspect {track, clip, rows?}`                                  | both   | yes       | `pattern.inspect`: cells formatted as the UI shows them (`cell_format.rs`)      |
| `machine_inspect {node}`                                                | both   | yes       | `machine.inspect`: `MachineInfo`, each param's `ParamInfo`, its current value and display text |
| `graph_inspect`                                                         | both   | yes       | `graph.inspect`: nodes, connections, gains, bypass                              |
| `render_audio {seconds, track?, clip?, path?}`                          | both   | yes       | `render.wav` plus an **analysis summary** (see below)                           |
| `ui_inspect`, `ui_action`, `ui_key`, `ui_screenshot`, `ui_elements`, `ui_control` | bridge | varies | the control-channel methods                                          |

Resources: `masterblaster://song` (the same content as `song_inspect`) and
`masterblaster://commands`.

**Audio-specific: agents can't listen.** `render_audio` writes the WAV and returns numbers an
agent can reason about:

- peak and RMS per channel, and loudness in LUFS;
- the first and last non-silent sample;
- a coarse ⅓-octave band spectrum;
- clipping, NaN and denormal counts;
- for a single-track render, per-track levels.

Optionally it also returns a waveform or spectrogram PNG as image content. Use the same
analysis functions that the parity comparison tiers use (artcraft-parity.md §4.5), so agents and
oracle tests measure audio the same way.

Copy FilmCraft's conventions as they are:

- annotations on every tool (enforced by a test);
- `-32602` for unknown arguments;
- `isError` results for failed commands;
- `-32700` for a line that isn't JSON, while the server keeps serving;
- `catch_unwind` around tool calls;
- a bridge client that accepts loopback only, retries reads but never edits, and times out (55 s)
  before the server (60 s);
- a `.mcp.json` registering `masterblaster-headless` and `masterblaster` (bridge).

### Phase 6: tests

| Test                                                    | Where                                    | Asserts                                                                                                                                    |
|---------------------------------------------------------|------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------|
| `registry_is_consistent`                                | `mb-master/src/commands/tests.rs`        | Unique ids; every schema serializes; every `Edit` variant reachable through some command.                                                  |
| `no_command_panics_on_hostile_params`                   | `mb-master/tests/command_hunt.rs`        | Every command × {`{}`, `null`, out-of-range ints, wrong types, huge numbers, unknown fields} × {no song, MOD, BMX} returns `Err` and never panics (PhotoCraft's `panic_hunt`). |
| `undo_round_trips_every_edit_command`                   | `mb-master/tests/commands.rs`            | Execute, undo, and the song equals its earlier state; redo, and it equals the edited state.                                                |
| `serve_script_renders_same_as_api`                      | `tests/serve.rs`                         | A `serve` script's WAV is bit-identical to calling `Controller` directly.                                                                  |
| `every_editor_action_has_an_id`                         | `src/ui/commands.rs`                     | Exhaustive match over `EditorAction`.                                                                                                      |
| Headless UI behaviour                                   | `tests/ui_actions.rs`                    | `ui.action` sequences on a `GuiState`, checked through `ui.inspect`. Replaces most of `gui_tests.rs`.                                      |
| Control envelope                                        | `src/control/tests.rs`                   | A line that isn't a request closes the connection, oversize lines are rejected, ids round-trip, errors come back as `ok: false`.           |
| `every_tool_is_annotated`                               | `mb-automation/tests/tools.rs`           | Title and the four hints on every tool.                                                                                                    |
| MCP end to end                                          | `mb-automation/tests/stdio.rs`           | Spawn `mb-cli mcp`, then initialize → `tools/list` → `command_run file.open` → `render_audio`.                                             |

Put **one** shared `Driver` in `tests/common/mod.rs`, with `exec`, `ok`, `action` and `inspect`,
used by every control and UI test file.

## Changes by file

| File                                                    | Change                                                                               |
|---------------------------------------------------------|--------------------------------------------------------------------------------------|
| `crates/mb-master/Cargo.toml`                           | Add `serde`, `serde_json`, `schemars`                                                |
| `crates/mb-master/src/commands/*` (NEW)                 | Registry, commands, errors, enabled checks                                           |
| `crates/mb-master/src/lib.rs`                           | `execute()`, undo stack, `machine_param_edit()`, inspect helpers                     |
| `crates/mb-ir/src/edit.rs`                              | `Edit::SetNodeParam { node, param, value }` (from design 006)                        |
| `crates/mb-engine/src/…`                                | Apply `SetNodeParam` at the tick boundary                                            |
| `crates/mb-automation/*` (NEW)                          | MCP server and bridge client                                                         |
| `src/bin/cli.rs`                                        | Subcommands: `serve`, `run`, `mcp`; keep the legacy form                             |
| `src/ui/commands.rs` (NEW)                              | UI command table (id ↔ `EditorAction` ↔ key)                                         |
| `src/ui/mod.rs`                                         | Remove `undo_stack`; add `GuiStateView`, `auto` registry                             |
| `src/ui/undo.rs`                                        | Deleted (moves into `Controller`)                                                    |
| `src/control/{server,handle,elements}.rs` (NEW)         | Control channel                                                                      |
| `src/main.rs`, `src/app.rs`                             | `--control`, `EventLoopProxy` wake-up, per-frame drain, screenshot outcome           |
| `.mcp.json` (NEW)                                       | Register the headless and bridge servers                                             |
| `docs/control-protocol.md` (NEW)                        | Method and tool reference                                                            |

## Edge cases

- **Commands during playback.** Edits go through the SPSC ring as they do today. Queries read
  the `Controller`'s copy of the song. `render.wav` while playing renders offline from that copy
  and doesn't touch the audio thread.
- **Faust JIT compile time** (design 024). `file.open` on a BMX can take seconds. Run it as a
  job: `wait: false` returns `{job}`, and the caller polls `jobs.list`. Alternatively, rely on
  024's cache and keep it synchronous under the 60 s limit.
- **Raw parameter values.** BMX stores parameters raw (byte or word, with a "no value"
  sentinel). `machine.setParam` takes the raw value and validates it against `ParamInfo`
  (`min..=max`, or `no_value`). `machine.inspect` reports both the raw value and its display text.
- **Hostile numbers.** Indices from params (track, clip, row, node, beat) are checked with
  `get()`. Out-of-range values return `BadParams`, never an index panic.
- **Window hidden or minimized.** A frame that's never drawn makes `ui.*` requests time out.
  Reply after 10 s with an explicit error instead of the generic 60 s timeout.
  `engine.execute` doesn't need a frame, so the server could answer it directly from the
  transport thread (behind a `Mutex<Controller>`)… but that splits ownership of `Controller`.
  Keep everything on the main thread and accept the latency.
- **File paths.** `file.open` and `render.wav` can read and write any path the user can access.
  That's acceptable for a local, opt-in, loopback-only channel. See the open questions.

## Verification

- `make ci` passes with the new tests, and `cargo test -p mb-master` needs no GUI.
- `printf … | cargo cli serve` renders a BMX file with a parameter change, and the WAV differs
  from the unmodified render only after the change tick.
- `masterblaster --control 9876` plus the Python one-liner client from FilmCraft's `agents.md`
  can `ui.action editor.moveCursor` and `ui.screenshot`, and the PNG shows the cursor moved.
- In Claude Code with `.mcp.json`, `command_list` → `command_run file.open` → `render_audio`
  works headless, and `ui_screenshot` works through the bridge.

## Relationship to other designs

- **006 (edit command system):** commands wrap `Edit`. This design finishes 006's
  `SetNodeParam` and moves undo into `Controller`.
- **021 (configurable UI):** the UI command table becomes the source for key bindings.
- **024 (eager Faust compilation):** this decides whether `file.open` needs the job model.
- **`~/code/artcraft-parity.md`:** `mb-cli serve` scripts are the masterblaster half of the oracle
  harness. `machine.inspect` is the counterpart of the oracle's `GetInfo`/`DescribeValue` dump. The
  UI command table is the UI parity catalog.

## Open questions

1. **Command id vocabulary.** Should ids follow Buzz's own names for actions (from its menus
   and help) so labels can be diffed for parity? Or should they be masterblaster's own names, with
   Buzz names only in `label`?
2. **Undo ownership.** Moving undo from `GuiState` into `Controller` changes behaviour: cursor
   position is no longer restored by undo unless it's recorded with the edit. Is that acceptable,
   or should the UI keep its own layer of cursor history?
3. **Authentication.** Is loopback-only with opt-in `--control` enough, or should we add
   PhotoCraft's token and read/write roots from the start?
4. **The analysis fields of `render_audio`.** Which metrics matter for your workflow: LUFS,
   bands, onset times? They should match whatever the parity tiers settle on.
5. **Dependencies.** Is `schemars` in `mb-master` acceptable? Is tokio acceptable in a separate
   `mb-automation` crate that only the CLI links?
