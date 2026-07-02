# testament

`testament` is a Rust CLI and library workspace for evaluating test quality
guardrails from static test IR and optional dynamic evidence. The first
supported target is Ruby test suites, with RSpec, Minitest, and test-unit
adapters.

## What It Measures

- Adequacy signals such as assertion density, line/branch coverage, mutation
  score, and checked coverage from trace evidence.
- Redundancy signals from static similarity, assertion overlap, per-test
  coverage, and per-test mutation evidence.
- Maintainability signals from test-smell rules.

The design rationale is documented in
[test-quality-guardrail-design.md](test-quality-guardrail-design.md).

## Requirements

- Rust 1.91, managed by [rust-toolchain.toml](rust-toolchain.toml)
- Ruby 3.1 or newer for the Ruby probe syntax/runtime checks

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
ruby -c probes/ruby/testament-probe-ruby/lib/testament/probe.rb
```

Run a fixture report:

```sh
cargo run -p testament-cli -- --config fixtures/evidence/testament.toml report --format json fixtures/ruby/sample_spec.rb
```

## Ruby Probe

The Ruby probe can collect per-test coverage and trace evidence:

```ruby
require "testament/probe"
```

By default it writes `.testament/per-test-coverage.json` and
`.testament/trace.json`. See
[probes/ruby/testament-probe-ruby/README.md](probes/ruby/testament-probe-ruby/README.md)
for probe-specific options.

## License

This project is licensed under the MIT license. See [LICENSE](LICENSE).
