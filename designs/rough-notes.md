# Rough Notes

Miscellaneous thoughts & TODOs that we need to address or turn into design/ plans.

## TODOs
- [ ] feat: Make all keyboard actions configurable, text version of the imgui keyboard enum and a .json mapping layer. Then input.ts can consume EditorActions directly from this mapping table.
- [ ] refactor: Consolidate render_frames vs render_frame-- why do we only use render_frames for offline rendering? Seems like we should render (audio rate / control rate) frames as a block.
- [x] concern: Fixed point vs Floating point -- we chose fixed point at the start of this and I'm thinking it may have been a bad idea after [reading up](https://www.dspguide.com/ch28/4.htm) more on this. There are embedded DSPs with floating point support available, they're just more expensive, but the embedded version of this is still very far away. Faust outputs in floating point, and it seems like we're doing a lot of conversion between these two even now pre-Faust integration. There are noise ratio issues with fixed point we just don't have to think about with floating point, and we don't want to get trapped supporting only low bitrates-- that was fine in the tracker era but not now.
- [x] concern: Need some way to ensure we don't do any allocations in the hotpath -- think we might have some cloning of frames and the Events must get alloced on the heap. Ideally we pre-alloc everything needed at song play (or earlier!) and recycle from a pool. Make this testable, maybe using alloc_tracker to panic if we hit allocs in the hot loop.
  - Partly finished on this one, tho we're still allocing when sending command.
- [ ] bug: Looks like we lost the song analysis in the CLI in e01239604ab91f0828054e5cdcb13e743b0e58ef
- [ ] concern: Bug found in 851ecaaf88c54a644d4d075886f0fd86eada7a99 basically shows we're looping infinitely with PositionJump backwards. Budget is based on # of tracks, but that doesn't make much sense-- should probably establish a max number of backwards jumps to loop for, at least when recording WAV files. Looping infinitely might be desirable for live performances & certainly pattern jamming.
