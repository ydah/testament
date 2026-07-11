Gem::Specification.new do |spec|
  spec.name = "testament-probe-ruby"
  spec.version = "0.1.0"
  spec.summary = "Per-test Ruby coverage and trace probe for testament"
  spec.description = "Collects per-test line coverage and assertion trace evidence for the testament test-quality analyzer."
  spec.authors = ["testament contributors"]
  spec.license = "MIT"
  spec.homepage = "https://github.com/ydah/testament"
  spec.metadata = {
    "source_code_uri" => "https://github.com/ydah/testament",
    "homepage_uri" => "https://github.com/ydah/testament"
  }
  spec.files = Dir["lib/**/*.rb"]
  spec.require_paths = ["lib"]
  spec.required_ruby_version = ">= 3.1"
end
