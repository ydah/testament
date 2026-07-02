require "coverage"
require "fileutils"
require "json"

module Testament
  module Probe
    DEFAULT_OUTPUT = ".testament/per-test-coverage.json"
    DEFAULT_TRACE_OUTPUT = ".testament/trace.json"
    DEFAULT_TRACE_WINDOW = 200
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
        install_rspec if defined?(RSpec)
        install_minitest if defined?(Minitest::Test)
        at_exit { write! }
      end

      def record(case_id)
        start_coverage
        start_trace
        previous_case = @current_case
        previous_recent_lines = @recent_lines
        previous_assertion_depth = @assertion_depth
        @recent_lines = []
        @assertion_depth = 0
        @current_case = case_id
        trace_cases[case_id]
        yield
      ensure
        merge_case(case_id, capture_coverage) if case_id
        @recent_lines = previous_recent_lines || []
        @assertion_depth = previous_assertion_depth || 0
        @current_case = previous_case
        write!
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

      def start_coverage
        return if coverage_running?

        Coverage.start(lines: true)
      end

      def start_trace
        @tracepoint ||= TracePoint.new(:line, :call, :c_call, :return, :c_return) do |event|
          case event.event
          when :line
            record_trace_line(event.path, event.lineno)
          when :call, :c_call
            enter_assertion if assertion_method?(event.method_id)
          when :return, :c_return
            leave_assertion if assertion_method?(event.method_id)
          end
        end
        @tracepoint.enable unless @tracepoint.enabled?
      end

      def coverage_running?
        Coverage.respond_to?(:running?) && Coverage.running?
      end

      def capture_coverage
        Coverage.result(stop: false, clear: true)
      rescue ArgumentError
        Coverage.result
      end

      def merge_case(case_id, coverage)
        normalized = normalize_coverage(coverage)
        return if normalized.empty?

        existing = cases.fetch(case_id, {})
        normalized.each do |path, lines|
          merged = (existing.fetch(path, []) + lines).uniq.sort
          existing[path] = merged
        end
        cases[case_id] = existing
      end

      def record_trace_line(path, line)
        return unless @current_case

        path = normalize_trace_path(path)
        return unless path

        trace = trace_cases[@current_case]
        append_trace_line(trace["executed"], path, line)
        append_trace_line(trace["checked"], path, line) if assertion_active?
        @recent_lines ||= []
        @recent_lines << [path, line]
        @recent_lines.shift while @recent_lines.length > trace_window
      end

      def enter_assertion
        mark_recent_lines_checked
        @assertion_depth = assertion_depth + 1
      end

      def leave_assertion
        @assertion_depth = [assertion_depth - 1, 0].max
      end

      def assertion_active?
        assertion_depth.positive?
      end

      def assertion_depth
        @assertion_depth ||= 0
      end

      def mark_recent_lines_checked
        return unless @current_case

        checked = trace_cases[@current_case]["checked"]
        @recent_lines.each do |path, line|
          append_trace_line(checked, path, line)
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

      def assertion_method?(method_id)
        ASSERTION_METHODS.include?(method_id)
      end

      def trace_window
        [@trace_window || DEFAULT_TRACE_WINDOW, 1].max
      end

      def normalize_coverage(coverage)
        coverage.each_with_object({}) do |(path, value), result|
          lines = line_hits(value)
          covered = lines.each_with_index.filter_map do |hits, index|
            index + 1 if hits && hits.positive?
          end
          result[path] = covered unless covered.empty?
        end
      end

      def line_hits(value)
        if value.is_a?(Hash)
          value.fetch(:lines, value.fetch("lines", []))
        else
          value || []
        end
      end

      def install_rspec
        RSpec.configure do |config|
          config.around(:each) do |example|
            Testament::Probe.record(example.full_description) { example.run }
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
