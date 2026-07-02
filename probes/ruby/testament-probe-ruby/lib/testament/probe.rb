require "coverage"
require "fileutils"
require "json"

module Testament
  module Probe
    DEFAULT_OUTPUT = ".testament/per-test-coverage.json"

    class << self
      def install!(output: ENV.fetch("TESTAMENT_PROBE_OUTPUT", DEFAULT_OUTPUT))
        @output = output
        install_rspec if defined?(RSpec)
        install_minitest if defined?(Minitest::Test)
        at_exit { write! }
      end

      def record(case_id)
        start_coverage
        result = yield
        merge_case(case_id, capture_coverage)
        result
      ensure
        write!
      end

      def write!
        output = @output || DEFAULT_OUTPUT
        FileUtils.mkdir_p(File.dirname(output))
        File.write(output, JSON.pretty_generate("cases" => cases))
      end

      private

      def cases
        @cases ||= {}
      end

      def start_coverage
        return if coverage_running?

        Coverage.start(lines: true)
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
