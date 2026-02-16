# Rough Notes
Miscellaneous thoughts & TODOs that we need to address or turn into design/ plans.

## TODOs
- [ ] Make all keyboard actions configurable, text version of the imgui keyboard enum and a .json mapping layer. Then input.ts can consume EditorActions directly.
- [ ] Need some way to ensure we don't do any allocations in the hotpath -- think we might have some cloning of frames and the Events must get alloced on the heap. Ideally we pre-alloc everything needed at song play (or earlier!) and recycle from a pool. Make this testable, maybe using alloc_tracker to panic if we hit allocs in the hot loop.
- [ ] Consolidate render_frames vs render_frame-- why do we only use render_frames for offline rendering? Seems like we should render (audio rate / control rate) frames as a block.

