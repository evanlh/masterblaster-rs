.PHONY: ci test test-gui bench

# Run everything: tests, GUI tests, benchmarks
ci: test test-gui bench

# All workspace tests (unit + integration + snapshots)
test:
	cargo test --workspace

# GUI tests (requires display)
test-gui:
	cargo test --test gui_tests --features test-harness

# Criterion benchmarks (quick mode)
bench:
	cargo bench -p mb-engine --bench engine_bench -- --quick
