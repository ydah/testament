require "fileutils"
require "json"
require_relative "probe/version"

module Testament
  module Probe
    DEFAULT_OUTPUT = ".testament/per-test-coverage.json"
    DEFAULT_TRACE_OUTPUT = ".testament/trace.json"
    DEFAULT_TRACE_WINDOW = 200
    THREAD_STATE_KEY = :testament_probe_recording
    ASSERTION_METHODS = %i[
      expect should should_not to not_to to_not
      assert assert_equal assert_same assert_nil assert_not_nil assert_empty
      assert_includes assert_match assert_operator assert_predicate
      assert_instance_of assert_kind_of assert_raises assert_throws
      assert_output assert_silent
      refute refute_equal refute_same refute_nil refute_empty
      refute_includes refute_match refute_operator refute_predicate
      refute_instance_of refute_kind_of
      must_be must_equal must_include must_match must_raise must_be_nil
      wont_be wont_equal wont_include wont_match wont_be_nil
    ].freeze

    class << self
      def install!(
        output: ENV.fetch("TESTAMENT_PROBE_OUTPUT", DEFAULT_OUTPUT),
        trace_output: ENV.fetch("TESTAMENT_TRACE_OUTPUT", DEFAULT_TRACE_OUTPUT)
      )
        @output = output
        @trace_output = trace_output
        @root = File.expand_path(ENV.fetch("TESTAMENT_PROJECT_ROOT", Dir.pwd))
        @probe_file = File.expand_path(__FILE__)
        @trace_window = ENV.fetch("TESTAMENT_TRACE_WINDOW", DEFAULT_TRACE_WINDOW).to_i
        @trace_enabled = ENV.fetch("TESTAMENT_TRACE", "1") != "0"
        install_rspec if defined?(RSpec)
        install_minitest if defined?(Minitest::Test)
        at_exit { write! }
      end

      def record(case_id)
        start_trace
        previous = Thread.current[THREAD_STATE_KEY]
        recording = {
          case_id: case_id,
          coverage: {},
          executed: {},
          checked: {},
          recent_lines: [],
          assertion_depth: 0
        }
        Thread.current[THREAD_STATE_KEY] = recording
        yield
      ensure
        merge_recording(recording) if case_id && recording
        Thread.current[THREAD_STATE_KEY] = previous
      end

      def write!
        output = @output || DEFAULT_OUTPUT
        FileUtils.mkdir_p(File.dirname(output))
        File.write(output, JSON.pretty_generate("cases" => cases))

        trace_output = @trace_output || DEFAULT_TRACE_OUTPUT
        FileUtils.mkdir_p(File.dirname(trace_output))
        File.write(trace_output, JSON.pretty_generate("cases" => trace_cases))
      end

      private

      def cases
        @cases ||= {}
      end

      def trace_cases
        @trace_cases ||= Hash.new do |cases, case_id|
          cases[case_id] = { "executed" => {}, "checked" => {} }
        end
      end

      def start_trace
        events = [:line]
        events.concat([:call, :c_call, :return, :c_return]) if @trace_enabled
        @tracepoint ||= TracePoint.new(*events) do |event|
          case event.event
          when :line
            record_trace_line(event.path, event.lineno)
          when :call, :c_call
            enter_assertion if assertion_method?(event.method_id, event.defined_class)
          when :return, :c_return
            leave_assertion if assertion_method?(event.method_id, event.defined_class)
          end
        end
        @tracepoint.enable unless @tracepoint.enabled?
      end

      def merge_recording(recording)
        mutex.synchronize do
          merge_lines(cases, recording[:case_id], recording[:coverage])
          if @trace_enabled
            trace = trace_cases[recording[:case_id]]
            merge_files(trace["executed"], recording[:executed])
            merge_files(trace["checked"], recording[:checked])
          end
        end
      end

      def merge_lines(collection, case_id, files)
        merge_files(collection[case_id] ||= {}, files)
      end

      def merge_files(existing, additions)
        additions.each do |path, lines|
          existing[path] = (existing.fetch(path, []) + lines).uniq.sort
        end
      end

      def mutex
        @mutex ||= Mutex.new
      end

      def record_trace_line(path, line)
        recording = Thread.current[THREAD_STATE_KEY]
        return unless recording

        path = normalize_trace_path(path)
        return unless path

        append_trace_line(recording[:coverage], path, line)
        return unless @trace_enabled

        append_trace_line(recording[:executed], path, line)
        append_trace_line(recording[:checked], path, line) if assertion_active?
        recording[:recent_lines] << [path, line]
        recording[:recent_lines].shift while recording[:recent_lines].length > trace_window
      end

      def enter_assertion
        mark_recent_lines_checked
        recording = Thread.current[THREAD_STATE_KEY]
        recording[:assertion_depth] = assertion_depth + 1 if recording
      end

      def leave_assertion
        recording = Thread.current[THREAD_STATE_KEY]
        recording[:assertion_depth] = [assertion_depth - 1, 0].max if recording
      end

      def assertion_active?
        assertion_depth.positive?
      end

      def assertion_depth
        Thread.current[THREAD_STATE_KEY]&.fetch(:assertion_depth, 0) || 0
      end

      def mark_recent_lines_checked
        recording = Thread.current[THREAD_STATE_KEY]
        return unless recording

        recording[:recent_lines].each do |path, line|
          append_trace_line(recording[:checked], path, line)
        end
      end

      def append_trace_line(files, path, line)
        lines = files.fetch(path, [])
        lines << line
        files[path] = lines.uniq.sort
      end

      def normalize_trace_path(path)
        return if path.nil? || path.empty?
        return if path.start_with?("<")

        expanded = File.expand_path(path)
        root = @root || Dir.pwd
        return if expanded == @probe_file
        return unless expanded == root || expanded.start_with?("#{root}/")

        expanded.delete_prefix("#{root}/")
      end

      def assertion_method?(method_id, defined_class)
        return false unless ASSERTION_METHODS.include?(method_id)

        owner = defined_class.to_s
        owner.include?("RSpec") || owner.include?("Minitest") || owner.include?("Test::Unit")
      end

      def trace_window
        [@trace_window || DEFAULT_TRACE_WINDOW, 1].max
      end

      def install_rspec
        RSpec.configure do |config|
          config.around(:each) do |example|
            path = example.metadata[:file_path].to_s.sub(%r{\A\./}, "")
            Testament::Probe.record("#{path}::#{example.full_description}") { example.run }
          end
        end
      end

      def install_minitest
        return if Minitest::Test < MinitestIntegration

        Minitest::Test.prepend(MinitestIntegration)
      end
    end

    module MinitestIntegration
      def run
        Testament::Probe.record("#{self.class}##{name}") { super }
      end
    end
  end
end

Testament::Probe.install!
