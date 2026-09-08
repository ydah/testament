# Releasing testament

The Rust workspace and Ruby probe use the same semantic version. A tag named
`vX.Y.Z` must match both the workspace version in `Cargo.toml` and
`Testament::Probe::VERSION`.

## Prerequisites

- A clean `main` branch with all required checks passing.
- A crates.io account authorized to publish every `testament-*` crate.
- A RubyGems.org account authorized to publish `testament-probe-ruby`.
- The GitHub `release` environment configured with any desired reviewers.

## Preflight

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo audit
scripts/check-release-version
cargo package --workspace --locked --no-verify
uvx zizmor@1.26.1 --persona auditor --format=plain .
(cd probes/ruby/testament-probe-ruby && gem build testament-probe-ruby.gemspec)
```

Inspect the generated packages under `target/package/` before publishing.

## Registry publishing

The first crates.io release must be published in dependency order. Wait until
each crate is visible in the crates.io index before publishing its dependents.
Do not run `cargo publish` without `-p` from the virtual workspace root: Cargo
selects every workspace member and cannot verify unpublished inter-crate
dependencies through its temporary registry. Publish and verify one crate at a
time instead.

```sh
cargo publish -p testament-core
cargo publish -p testament-adapter-api
cargo publish -p testament-evidence
cargo publish -p testament-lang-ruby
cargo publish -p testament-fw-rspec
cargo publish -p testament-fw-minitest
cargo publish -p testament-fw-testunit
cargo publish -p testament-metrics
cargo publish -p testament-report
cargo publish -p testament-cli
```

Build and publish the Ruby probe from its own directory so the gemspec includes
the intended files:

```sh
cd probes/ruby/testament-probe-ruby
gem build testament-probe-ruby.gemspec
gem push testament-probe-ruby-X.Y.Z.gem
```

Publishing registry versions is permanent. Do not continue if any package
contents or version differs from the intended release.

## GitHub release

Create and push the matching signed tag only after registry publishing succeeds:

```sh
git tag -s vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

The release workflow verifies version consistency, rebuilds all packages,
creates Linux, macOS, and Windows CLI archives, generates SHA-256 checksums,
and publishes the artifacts in a GitHub Release.
