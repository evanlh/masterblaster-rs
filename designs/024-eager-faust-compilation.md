# 024: Eager Faust JIT Compilation

Created: 20260403
Updated: 20261014

## Status

- [x] Add machine code disk cache (`cache.rs`)
- [x] Add FFI bindings for `writeCDSPFactoryToMachineFile` / `readCDSPFactoryFromMachineFile`
- [x] Cache pre-built machines on Controller (move to load time)
- [x] Pass cached machines to audio thread / offline render
- [x] Verify no pause on Play

## Problem

Pressing Play causes a multi-second pause before audio begins. The root cause:
`build_machines()` runs Faust LLVM JIT compilation on the audio thread
(`lib.rs:472`), blocking until all machines are compiled. WahPro2 is complex
(17 LFO shapes, 8 filter types, ~4.3KB DSP) and takes ~500-800ms per instance.
The Insomnium BMX has 3 WahPro2 instances, totaling ~1.5-2.4s of blocking.

Call chain:
1. User clicks Play -> `Controller::play()` spawns audio thread
2. Audio thread calls `audio_thread()` (line 462)
3. `build_machines(&song, sample_rate)` blocks on JIT (line 472)
4. Audio stream only starts after all machines are compiled (line 480)

## Solution: Two-Layer Caching

### Layer 1: Disk Cache (Machine Code)

The libfaust C API provides factory serialization to native machine code:

```c
// /usr/local/include/faust/dsp/llvm-dsp-c.h (lines 404-459)
bool writeCDSPFactoryToMachineFile(factory, path, target);
llvm_dsp_factory* readCDSPFactoryFromMachineFile(path, target, error_msg);
```

Machine code loads in <1ms vs ~500-800ms for JIT. The cache uses
platform-native directories via `dirs::cache_dir()`:
- macOS: `~/Library/Caches/masterblaster/faust/`
- Linux: `~/.cache/masterblaster/faust/`
- Windows: `%LOCALAPPDATA%/masterblaster/faust/`

Cache is keyed by MD5 of the DSP source string.

Cache flow per machine:
1. Hash DSP source (MD5)
2. Check `{cache_dir}/masterblaster/faust/{name}-{md5}.machine`
3. **Hit**: `readCDSPFactoryFromMachineFile()` -> create instance -> done (<1ms)
4. **Miss**: `createCDSPFactoryFromString()` -> `writeCDSPFactoryToMachineFile()` -> create instance

Cache invalidation is automatic: if the `.dsp` source changes, the MD5 changes,
and the old cache file is ignored. Include `getCLibFaustVersion()` in the cache
key to handle libfaust upgrades.

### Layer 2: In-Memory Cache (Move to Load Time)

After disk-cached compilation, cache the built machines on the Controller so
`play()` doesn't rebuild every time.

### Files Modified

**`crates/mb-faust/src/ffi.rs`** — Add extern bindings:
- `writeCDSPFactoryToMachineFile(factory, path, target) -> bool`
- `readCDSPFactoryFromMachineFile(path, target, error_msg) -> *mut LlvmDspFactory`
- `getCLibFaustVersion() -> *const c_char`

**`crates/mb-faust/src/cache.rs`** (new) — Disk cache logic:
- `fn cache_dir() -> PathBuf` — `dirs::cache_dir() / "masterblaster" / "faust"`
- `fn cache_path(name: &str, source: &str) -> PathBuf` — `{name}-{md5}.machine`
- `fn try_load(name: &str, source: &str) -> Option<*mut LlvmDspFactory>`
- `fn save(name: &str, source: &str, factory: *mut LlvmDspFactory)`

**`crates/mb-faust/src/compiler.rs`** — Add `compile_cached()`:
- Try disk cache first, fall back to JIT + save
- Reuse existing `compile()` for the JIT path
- Factory from cache still needs `createCDSPInstance()` + param discovery

**`crates/mb-master/src/lib.rs`** — Move compilation to load time:
- Add `machines: Vec<Option<Box<dyn Machine>>>` to `Controller`
- Build in `load_bmx()` / `load_mod()` / `set_song()` / `new_song()`
- `play_song()`: `std::mem::take()` to move to audio thread
- `audio_thread()`: accept pre-built machines, skip `build_machines()`

### New Dependencies

**`crates/mb-faust/Cargo.toml`**:
- `dirs = "6"` — platform-native cache directory (also planned for design 021)
- `md5 = "0.7"` — hash DSP source for cache key

### Edge Cases

- **Graceful fallback**: If cache dir doesn't exist or read fails, just JIT compile
- **Architecture**: Cache is arch-specific (arm64 vs x86_64). The `target` param
  to `readCDSPFactoryFromMachineFile()` handles this; pass empty string for host
- **First run**: All machines JIT compile + save to cache. Subsequent runs instant
- **Take-and-rebuild**: `Box<dyn Machine>` is Send but not Clone. On Stop+Play,
  rebuild from disk cache on main thread (~5ms total vs ~2s JIT)

## Verification

1. `cargo test` / `make test-faust` — all tests pass
2. First run: machines compile + cache files appear in platform cache dir
3. Second run: machines load from cache (<10ms total)
4. Modify a `.dsp` file: cache miss, recompile + save new cache
5. Play button: no multi-second pause

> **Note on item 5:** verified indirectly — compilation now happens in
> `rebuild_machines()` at load time and factories load from disk cache via
> `readCDSPFactoryFromMachineFile()`, so `play_song()` no longer JITs. A direct
> GUI play-latency timing (loading a multi-WahPro2 BMX and measuring
> play-to-audio delay) has not been run and should be confirmed manually.

## Implementation Notes

- **Sample-rate handling.** Machines are built on the Controller at a fixed
  `DEFAULT_SAMPLE_RATE` (44100) because the device rate is not known until the
  audio thread opens the output. `finalize_machines()` re-initializes each
  machine at the actual rate and re-applies graph node params before use, since
  `FaustMachine::init()` → `initCDSPInstance()` resets slider zones to the DSP
  defaults. This keeps rate-dependent coefficients correct on non-44100 devices.
- **Cache key.** The filename hash covers the DSP source *and* the value of
  `getCLibFaustVersion()`, so a libfaust upgrade automatically invalidates the
  cache. A library `compile_cached()` is used only by the registry;
  `compiler::compile()` remains uncached for tests.
- **Offline render** (`render_to_wav` / `render_frames`, `&self`) still calls
  `build_machines()`, which now hits the disk cache; the audio thread uses the
  Controller's in-memory machines via `std::mem::take()`.
