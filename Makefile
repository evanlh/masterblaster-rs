.PHONY: ci test test-gui test-faust bench

# Run everything: tests, GUI tests, benchmarks
ci: test test-gui bench

# All workspace tests (unit + integration + snapshots), excluding mb-faust (requires libfaust)
test:
	cargo test --workspace --exclude mb-faust

# Faust JIT tests (requires libfaust installed, must be single-threaded)
test-faust:
	cargo test -p mb-faust -- --test-threads=1

# GUI tests (requires display)
test-gui:
	cargo test --test gui_tests --features test-harness

# Criterion benchmarks (quick mode)
bench:
	cargo bench -p mb-engine --bench engine_bench -- --quick
