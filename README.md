# testament

`testament` is a Rust CLI and library workspace for evaluating test quality
guardrails from static test IR and optional dynamic evidence. The first
supported target is Ruby test suites, with RSpec, Minitest, and test-unit
adapters. Ruby syntax is parsed with Prism through the `ruby-prism` crate.

## What It Measures

- Adequacy signals such as assertion density, line/branch coverage, mutation
  score, checked coverage from trace evidence, and a separately identified
  static checked-coverage approximation.
- Redundancy signals from static similarity, assertion overlap, per-test
  coverage, and per-test mutation evidence.
- Maintainability signals from test-smell rules.

## Requirements

- Rust 1.91, managed by
  [rust-toolchain.toml](https://github.com/ydah/testament/blob/main/rust-toolchain.toml)
- Ruby 3.1 or newer for the Ruby probe syntax/runtime checks

## Installation

Install the CLI from crates.io:

```sh
cargo install testament-cli
```

Install the optional Ruby probe from RubyGems.org:

```sh
gem install testament-probe-ruby
```

To install the current `main` branch directly from the repository:

```sh
cargo install --git https://github.com/ydah/testament testament-cli
```

## Usage

```sh
testament check
testament report --format json spec/user_spec.rb
testament explain adequacy.assertion_density
testament explain --file spec/user_spec.rb
testament check --no-rename-tracking
```

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
[Ruby probe README](https://github.com/ydah/testament/blob/main/probes/ruby/testament-probe-ruby/README.md)
for probe-specific options.

## License

This project is licensed under the MIT license. See
[LICENSE](https://github.com/ydah/testament/blob/main/LICENSE).

Maintainer release instructions are documented in
[RELEASING.md](https://github.com/ydah/testament/blob/main/RELEASING.md).
