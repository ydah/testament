require_relative "lib/testament/probe/version"

Gem::Specification.new do |spec|
  spec.name = "testament-probe-ruby"
  spec.version = Testament::Probe::VERSION
  spec.summary = "Per-test Ruby coverage and trace probe for testament"
  spec.description = "Collects per-test line coverage and assertion trace evidence for the testament test-quality analyzer."
  spec.authors = ["Yudai Takada"]
  spec.license = "MIT"
  spec.homepage = "https://github.com/ydah/testament"
  spec.metadata = {
    "source_code_uri" => "https://github.com/ydah/testament",
    "homepage_uri" => "https://github.com/ydah/testament",
    "bug_tracker_uri" => "https://github.com/ydah/testament/issues",
    "allowed_push_host" => "https://rubygems.org",
    "rubygems_mfa_required" => "true"
  }
  spec.files = Dir["lib/**/*.rb"] + ["LICENSE.txt", "README.md"]
  spec.extra_rdoc_files = ["LICENSE.txt", "README.md"]
  spec.require_paths = ["lib"]
  spec.required_ruby_version = ">= 3.1"
end
