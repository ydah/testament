# testament-probe-ruby

Small Ruby-side companion for collecting per-test line coverage in the
`per-test-json` format consumed by testament.

## Usage

Add the probe to the test process before tests run:

```ruby
require "testament/probe"
```

By default the probe writes `.testament/per-test-coverage.json`. Set
`TESTAMENT_PROBE_OUTPUT` to write another path.

The probe installs hooks for RSpec and Minitest when those constants are loaded.
Load the probe after the test framework is required.
