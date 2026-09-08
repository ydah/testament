require "coverage"
require "fileutils"
require "tmpdir"

root = Dir.mktmpdir("testament-probe-")
at_exit { FileUtils.remove_entry(root) if File.exist?(root) }
ENV["TESTAMENT_PROJECT_ROOT"] = root
ENV["TESTAMENT_PROBE_OUTPUT"] = File.join(root, "coverage.json")
ENV["TESTAMENT_TRACE_OUTPUT"] = File.join(root, "trace.json")
Coverage.start(lines: true)
require "testament/probe"

File.write(File.join(root, "a.rb"), "def only_a\n  :a\nend\n")
File.write(File.join(root, "b.rb"), "def only_b\n  :b\nend\n")
load File.join(root, "a.rb")
load File.join(root, "b.rb")

only_a
path = File.join(root, "a.rb")
before = Coverage.peek_result.fetch(path).fetch(:lines)[1]
Testament::Probe.record("inside") { only_b }
after = Coverage.peek_result.fetch(path).fetch(:lines)[1]
raise "probe changed global coverage" unless after == before

a_ready, b_ready, a_called, b_done = 4.times.map { Queue.new }
a = Thread.new do
  Testament::Probe.record("A") do
    a_ready << true
    b_ready.pop
    only_a
    a_called << true
    b_done.pop
  end
end
b = Thread.new do
  a_ready.pop
  Testament::Probe.record("B") do
    b_ready << true
    a_called.pop
    only_b
  end
  b_done << true
end
[a, b].each(&:join)

coverage = Testament::Probe.send(:cases)
raise "outside line entered case" if coverage.fetch("inside").key?("a.rb")
raise "A coverage was contaminated" unless coverage.fetch("A").keys == ["a.rb"]
raise "B coverage was contaminated" unless coverage.fetch("B").keys == ["b.rb"]
