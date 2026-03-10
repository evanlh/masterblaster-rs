# Configurable UI: Colors, Keybindings, and Fonts from config.toml

Created: 20260308

## Status

- [ ] Phase 1: Config infrastructure (`AppConfig`, load/parse, Cmd+R reload)
- [ ] Phase 2: Extractable color theme (`ColorTheme` struct, wire to all UI panels)
- [ ] Phase 3: Configurable keybindings (action-name → key-combo mapping)
- [ ] Phase 4: Configurable font (font path + size from config)
- [ ] Phase 5: Theme directory prep (future: `theme = "name"` → `~/.config/masterblaster/themes/name.toml`)

## Problem

All colors, keybindings, and font settings are hardcoded constants scattered across multiple UI files. There's no way to customize the look and feel without recompiling. We want a `~/.config/masterblaster/config.toml` that can be edited and hot-reloaded with Cmd+R.

## Design

### Config file location

```
~/.config/masterblaster/config.toml
```

Use the `dirs` crate (`dirs::config_dir()`) for cross-platform support. Falls back gracefully to built-in defaults if the file doesn't exist.

### config.toml format

```toml
# Future: theme = "cyberpunk"  # loads from themes/cyberpunk.toml

[font]
path = "/Users/me/fonts/JetBrainsMono-Regular.ttf"
size = 14.0

[colors]
background = [0.10, 0.10, 0.10, 1.0]
playing_text = [0.39, 0.78, 0.51, 1.0]
playing_bg = [0.15, 0.30, 0.20, 1.0]
empty_text = [0.24, 0.24, 0.27, 1.0]
data_text = [0.78, 0.78, 0.78, 1.0]
cursor_bg = [0.25, 0.25, 0.50, 0.7]
cursor_edit_bg = [0.50, 0.20, 0.20, 0.7]
cursor_row_bg = [0.18, 0.18, 0.35, 0.40]
cursor_text = [1.0, 1.0, 1.0, 1.0]
selection_bg = [0.20, 0.30, 0.50, 0.35]
muted_text = [0.35, 0.35, 0.35, 1.0]
selected_track_bg = [0.25, 0.25, 0.40, 0.5]
mute_button_active = [0.6, 0.2, 0.2, 1.0]
row_label_beat16 = [0.39, 0.39, 0.59, 1.0]
row_label_beat4 = [0.31, 0.31, 0.39, 1.0]
row_label_default = [0.24, 0.24, 0.27, 1.0]
playing_row_label = [0.55, 0.55, 0.75, 1.0]
graph_master_bg = [0.20, 0.20, 0.27, 1.0]
graph_master_border = [0.47, 0.47, 0.71, 1.0]
graph_channel_bg = [0.16, 0.22, 0.16, 1.0]
graph_channel_border = [0.35, 0.55, 0.35, 1.0]
graph_connection = [0.31, 0.39, 0.31, 1.0]
graph_text = [0.78, 0.78, 0.78, 1.0]
view_toggle_active = [0.30, 0.30, 0.55, 1.0]

[keys]
play_stop = "Space"
play_pattern = "Ctrl+Space"
toggle_edit = "Backtick"
switch_graph = "Cmd+G"
switch_pattern = "Cmd+P"
switch_sequencer = "Cmd+E"
octave_up = "Cmd+Up"
octave_down = "Cmd+Down"
step_up = "Ctrl+Up"
step_down = "Ctrl+Down"
copy = "Cmd+C"
paste = "Cmd+V"
undo = "Cmd+Z"
redo = "Cmd+Shift+Z"
mute_track = "Ctrl+M"
reload_config = "Cmd+R"
```

Missing keys use built-in defaults. Unknown keys are ignored.

---

## Phase 1: Config infrastructure

### New dependencies (Cargo.toml)
```toml
serde = { version = "1", features = ["derive"] }
toml = "0.8"
dirs = "6"
```

### New file: `src/config.rs`

```rust
struct AppConfig {
    colors: ColorTheme,
    keys: KeyBindings,
    font: FontConfig,
}

struct FontConfig {
    path: Option<String>,
    size: f32,  // default 14.0
}
```

- `AppConfig::load()` — reads `~/.config/masterblaster/config.toml`, falls back to `Default`
- `AppConfig::default()` — returns current hardcoded values
- All fields use `#[serde(default)]` so partial configs work
- Parsing errors logged to status bar, config falls back to defaults

### Reload mechanism

- Add `EditorAction::ReloadConfig` variant
- Map `Cmd+R` in `poll_global_shortcuts`
- Handler in `process_actions`: calls `AppConfig::load()`, replaces `gui.config`
- Font reload requires rebuilding the imgui font atlas (see Phase 4)

### GuiState changes

```rust
pub struct GuiState {
    pub config: AppConfig,  // new field
    // ... existing fields
}
```

### Files to modify
| File | Change |
|------|--------|
| `Cargo.toml` | Add `serde`, `toml`, `dirs` |
| `src/config.rs` | New: `AppConfig`, `ColorTheme`, `KeyBindings`, `FontConfig`, load/parse |
| `src/ui/mod.rs` | Add `config` to `GuiState`, `ReloadConfig` handler |
| `src/ui/input.rs` | Add `ReloadConfig` variant, map `Cmd+R` |

---

## Phase 2: Extractable color theme

### New struct: `ColorTheme` (in `src/config.rs`)

Contains all 24 color values currently scattered across:
- `src/ui/colors.rs` (9 constants)
- `src/ui/sequencer.rs` (5 constants + 2 inline)
- `src/ui/pattern_editor.rs` (4 constants)
- `src/ui/graph.rs` (6 constants)
- `src/ui/transport.rs` (2 inline)
- `src/app.rs` (1 inline: clear_color)

### Migration strategy

1. `ColorTheme` struct with `#[serde(default)]` on every field
2. `Default` impl returns current hardcoded values
3. Replace `use super::colors::*` with `&gui.config.colors` reference
4. Pass `&ColorTheme` (or individual colors) to panel functions that currently use constants
5. Delete `src/ui/colors.rs` — all constants now live in `ColorTheme::default()`

### Panel function signature changes

Functions that draw colored UI currently import from `colors.rs`. They need access to the theme:
- `pattern_editor()` — gets `&ColorTheme` param or reads from `gui.config.colors`
- `sequencer_panel()` — already takes `&mut GuiState`, reads `gui.config.colors`
- `graph_panel()` — already takes `&mut GuiState`
- `transport_panel()` — already takes `&mut GuiState`
- `draw_row_bg()`, `cell_color()`, `row_beat_color()` — take color params instead of using constants

### Files to modify
| File | Change |
|------|--------|
| `src/config.rs` | `ColorTheme` struct with all 24 colors |
| `src/ui/colors.rs` | Delete (constants move to `ColorTheme::default()`) |
| `src/ui/mod.rs` | Remove `mod colors`, functions read from `gui.config.colors` |
| `src/ui/sequencer.rs` | Replace constants with `gui.config.colors.*` |
| `src/ui/pattern_editor.rs` | Replace constants with theme reference |
| `src/ui/graph.rs` | Replace constants with theme reference |
| `src/ui/transport.rs` | Replace inline colors with theme reference |
| `src/app.rs` | Read `config.colors.background` for `clear_color` |

---

## Phase 3: Configurable keybindings

### New struct: `KeyBindings` (in `src/config.rs`)

```rust
struct KeyBindings {
    play_stop: KeyCombo,
    play_pattern: KeyCombo,
    toggle_edit: KeyCombo,
    switch_graph: KeyCombo,
    switch_pattern: KeyCombo,
    switch_sequencer: KeyCombo,
    // ... etc for all ~15 bindable actions
    reload_config: KeyCombo,
}

struct KeyCombo {
    key: imgui::Key,
    cmd: bool,
    ctrl: bool,
    shift: bool,
}
```

### Parsing

`KeyCombo` deserializes from strings like `"Cmd+Shift+Z"`:
- Split on `+`
- Last token is the key name (mapped to `imgui::Key`)
- Prefix tokens are modifiers: `Cmd`, `Ctrl`, `Shift`
- Custom `Deserialize` impl or `FromStr`

### Mapping: KeyBindings → EditorAction

The `KeyBindings` fields map 1:1 to `EditorAction` variants. `poll_global_shortcuts` becomes a data-driven loop over a binding table:

```rust
fn check_combo(ui: &imgui::Ui, combo: &KeyCombo) -> bool {
    is_pressed(ui, combo.key)
        && ui.io().key_super == combo.cmd
        && ui.io().key_ctrl == combo.ctrl
        && ui.io().key_shift == combo.shift
}

fn poll_global_shortcuts(ui: &imgui::Ui, keys: &KeyBindings, actions: &mut Vec<EditorAction>) {
    let bindings: &[(&KeyCombo, EditorAction)] = &[
        (&keys.play_stop,        EditorAction::TogglePlayStop),
        (&keys.play_pattern,     EditorAction::TogglePlayPatternStop),
        (&keys.toggle_edit,      EditorAction::ToggleEditMode),
        (&keys.switch_graph,     EditorAction::SwitchToGraph),
        (&keys.switch_pattern,   EditorAction::SwitchToPattern),
        (&keys.switch_sequencer, EditorAction::SwitchToSequencer),
        (&keys.octave_up,        EditorAction::AdjustOctave(1)),
        (&keys.octave_down,      EditorAction::AdjustOctave(-1)),
        (&keys.step_up,          EditorAction::AdjustStep(1)),
        (&keys.step_down,        EditorAction::AdjustStep(-1)),
        (&keys.copy,             EditorAction::Copy),
        (&keys.paste,            EditorAction::Paste),
        (&keys.undo,             EditorAction::Undo),
        (&keys.redo,             EditorAction::Redo),
        (&keys.mute_track,       EditorAction::MuteSelectedTrack),
        (&keys.reload_config,    EditorAction::ReloadConfig),
    ];

    for (combo, action) in bindings {
        if check_combo(ui, combo) {
            actions.push(action.clone());
        }
    }
}
```

No separate mapping struct needed — `KeyBindings` field names *are* the action names. The 15 hand-written `if` chains collapse into this single loop.

Navigation keys (arrows, tab, page up/down) and note entry keys are NOT configurable — only the ~15 global shortcuts. However, `poll_navigation` should also be refactored to a table-driven style for consistency:

```rust
fn poll_navigation(ui: &imgui::Ui, shift: bool, actions: &mut Vec<EditorAction>) {
    let cmd = ui.io().key_super;
    let ctrl = ui.io().key_ctrl;
    if cmd || ctrl {
        return;
    }

    if shift {
        let bindings: &[(imgui::Key, EditorAction)] = &[
            (imgui::Key::UpArrow,    EditorAction::SelectMove { drow: -1, dchannel: 0 }),
            (imgui::Key::DownArrow,  EditorAction::SelectMove { drow: 1, dchannel: 0 }),
            (imgui::Key::LeftArrow,  EditorAction::SelectMove { drow: 0, dchannel: -1 }),
            (imgui::Key::RightArrow, EditorAction::SelectMove { drow: 0, dchannel: 1 }),
        ];
        for (key, action) in bindings {
            if is_pressed(ui, *key) {
                actions.push(action.clone());
            }
        }
    } else {
        let bindings: &[(imgui::Key, EditorAction)] = &[
            (imgui::Key::UpArrow,      EditorAction::MoveCursor { drow: -1, dchannel: 0, dcolumn: 0 }),
            (imgui::Key::DownArrow,    EditorAction::MoveCursor { drow: 1, dchannel: 0, dcolumn: 0 }),
            (imgui::Key::LeftArrow,    EditorAction::MoveCursor { drow: 0, dchannel: 0, dcolumn: -1 }),
            (imgui::Key::RightArrow,   EditorAction::MoveCursor { drow: 0, dchannel: 0, dcolumn: 1 }),
            (imgui::Key::Tab,          EditorAction::TabForward),
            (imgui::Key::PageUp,       EditorAction::PageUp),
            (imgui::Key::PageDown,     EditorAction::PageDown),
            (imgui::Key::Enter,        EditorAction::EnterOnCell),
            (imgui::Key::KeypadEnter,  EditorAction::EnterOnCell),
        ];
        for (key, action) in bindings {
            if is_pressed(ui, *key) {
                actions.push(action.clone());
            }
        }
    }

    // Shift+Tab handled separately (shift is already consumed above)
    if shift && is_pressed(ui, imgui::Key::Tab) {
        actions.push(EditorAction::TabBackward);
    }
}
```

This keeps the same hardcoded keys but makes the structure uniform with `poll_global_shortcuts`, so promoting nav keys to configurable later is a minimal change.

### Files to modify
| File | Change |
|------|--------|
| `src/config.rs` | `KeyBindings`, `KeyCombo`, parsing |
| `src/ui/input.rs` | `poll_global_shortcuts` reads from `KeyBindings` |

---

## Phase 4: Configurable font

### FontConfig

```rust
struct FontConfig {
    path: Option<String>,  // None = use default imgui font
    size: f32,             // default 14.0
}
```

### Loading

In `create_imgui_context()` (or a new function called after config load):

```rust
if let Some(ref font_path) = config.font.path {
    let font_data = std::fs::read(font_path)?;
    imgui.fonts().add_font(&[imgui::FontSource::TtfData {
        data: &font_data,
        size_pixels: config.font.size,
        config: Some(imgui::FontConfig { ..Default::default() }),
    }]);
} else {
    imgui.fonts().add_font(&[imgui::FontSource::DefaultFontData { ... }]);
}
```

### Hot reload complexity

Font changes on Cmd+R require:
1. Clear the font atlas
2. Re-add fonts
3. Rebuild the font texture
4. Upload the new texture to GPU via the renderer

This is more invasive than color/key reload. The renderer's `reload_font_texture()` method handles the GPU upload. Sequence:
```rust
imgui.fonts().clear();
// re-add font from new config
imgui.fonts().build_rgba32_texture(); // or let renderer handle it
renderer.reload_font_texture(&mut imgui)?;
```

This needs access to `&mut imgui::Context` and `&mut AutoRenderer` which live in `App`, not `GuiState`. So font reload happens in `App::reload_config()`, not in `process_actions`.

### Files to modify
| File | Change |
|------|--------|
| `src/config.rs` | `FontConfig` struct |
| `src/app.rs` | Font loading from config, `reload_config()` method for font rebuild |

---

## Phase 5: Theme directory prep (future)

Not implemented now, but the config structure supports it:

```toml
theme = "cyberpunk"
```

Would cause `AppConfig::load()` to first load `~/.config/masterblaster/themes/cyberpunk.toml` for colors, then overlay `config.toml` on top. The `ColorTheme` struct is already the right shape for this.

For now, `AppConfig::load()` just reads `config.toml` directly.

---

## Verification

### Unit tests
- `AppConfig::default()` produces valid config
- `ColorTheme::default()` matches current hardcoded values
- `KeyCombo` parsing: `"Cmd+Shift+Z"` → correct fields
- Partial config.toml (missing sections) loads without error, fills defaults
- Invalid config.toml (bad syntax) falls back to defaults with error message

### Manual testing
- Launch with no config file → works identically to current behavior
- Create `~/.config/masterblaster/config.toml` with custom colors → colors change
- Edit a color in the toml → Cmd+R → color updates immediately
- Change font path → Cmd+R → font reloads
- Bad font path → Cmd+R → falls back to default font, shows error in status bar
- Rebind a key in `[keys]` → Cmd+R → new binding works
