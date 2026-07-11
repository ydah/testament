# testament-probe-ruby

Small Ruby-side companion for collecting per-test line coverage and assertion
trace evidence consumed by testament.

## Usage

Add the probe to the test process before tests run:

```ruby
require "testament/probe"
```

By default the probe writes `.testament/per-test-coverage.json` and
`.testament/trace.json`. Set `TESTAMENT_PROBE_OUTPUT` or
`TESTAMENT_TRACE_OUTPUT` to write another path.

The trace output records executed project lines, lines executed while assertion
methods are active, and recent executed lines observed when assertions begin.
`TESTAMENT_PROJECT_ROOT` controls the project-root filter, and
`TESTAMENT_TRACE_WINDOW` controls how many recent lines are attributed to an
assertion.

Line/call tracing adds noticeable overhead to the test run. Set
`TESTAMENT_TRACE=0` to collect per-test line coverage without trace evidence.

The probe installs hooks for RSpec and Minitest when those constants are loaded.
Load the probe after the test framework is required.
