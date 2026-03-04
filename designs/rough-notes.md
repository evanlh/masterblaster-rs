# Rough Notes

Miscellaneous thoughts & TODOs that we need to address or turn into design/ plans.

## TODOs
- [ ] refactor: Consolidate render_frames vs render_frame-- why do we only use render_frames for offline rendering? Seems like we should render (audio rate / control rate) frames as a block.
- [x] concern: Fixed point vs Floating point -- we chose fixed point at the start of this and I'm thinking it may have been a bad idea after [reading up](https://www.dspguide.com/ch28/4.htm) more on this. There are embedded DSPs with floating point support available, they're just more expensive, but the embedded version of this is still very far away. Faust outputs in floating point, and it seems like we're doing a lot of conversion between these two even now pre-Faust integration. There are noise ratio issues with fixed point we just don't have to think about with floating point, and we don't want to get trapped supporting only low bitrates-- that was fine in the tracker era but not now.
- [x] concern: Need some way to ensure we don't do any allocations in the hotpath -- think we might have some cloning of frames and the Events must get alloced on the heap. Ideally we pre-alloc everything needed at song play (or earlier!) and recycle from a pool. Make this testable, maybe using alloc_tracker to panic if we hit allocs in the hot loop.
  - Partly finished on this one, tho we're still allocing when sending command.
- [ ] bug: Looks like we lost the song analysis in the CLI in e01239604ab91f0828054e5cdcb13e743b0e58ef
- [ ] concern: Bug found in 851ecaaf88c54a644d4d075886f0fd86eada7a99 basically shows we're looping infinitely with PositionJump backwards. Budget is based on # of tracks, but that doesn't make much sense-- should probably establish a max number of backwards jumps to loop for, at least when recording WAV files. Looping infinitely might be desirable for live performances & certainly pattern jamming.
- [ ] feat: Make all keyboard actions configurable, text version of the imgui keyboard enum and a .json mapping layer. Then input.ts can consume EditorActions directly from this mapping table.
- [ ] feat: Similarly, make colors and fonts configurable! Overall GUI needs a makeover, a nice 80s monospace font and a Mad Max color scheme.
- [ ] feat: Add information on the cell contents of the Effects columns to the Modeline, so you can easily change the Effect # & value and see a description of their effect
- [ ] feat: Add popup help. This should show keyboard shortcuts and an effect command cheatsheet.
- [ ] feat: Keybindings for pattern channel nav-- instead of Tab, Alt+left/right = Jump to the same field in the neighboring channel, Alt+up/down = jump rows-per-beat steps up down in current track.
- [ ] feat: Keybindings for nav between pannels-- pause this on deciding whether Clips and Samples columns are how we want to do this, but basically we want a keybinding to navigate between panels (Tab probably) so you can navigate all of the viewports via the keyboard. Also will need arrow/selection of Clips & Samples.
- [ ] feat: We can't play one-off samples to preview what they sound like. This should work in either Edit or non-Edit mode, keyboard should shift the pitch. The way we do this for Patterns is building a whole Song & sending it over because we clone the Song for the audio thread. Might need to investigate more efficient ways of doing that, as there is also some noticeable latency when playing patterns.
- [ ] bug: Can't scroll past 1F in pattern views with 256 rows in Endorphin Rush - Skooled.
- [ ] bug: Can't navigate in Pattern view, probably broke during sequencer additions. Add GUI test. (Actually, maybe specific to Endorphin Rush->Drums->P4 somehow)


## Design Philosophy
- Keyboard-driven, keycombos available for every interaction, nothing that *requires* a mouse.
- Hackable, in progressive stages. Beginner: Learn hex, make instrument graphs, customize themes. Intermediate/advanced: Make Faust patches, script the UI.
- We will not perfectly reproduce the exact sounds of the original MOD/BMX files-- try to Sound Good anyway.
