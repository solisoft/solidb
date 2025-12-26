require_relative 'lib/solidb/version'

Gem::Specification.new do |spec|
  spec.name          = "solidb"
  spec.version       = SoliDB::VERSION
  spec.authors       = ["SoliDB Team"]
  spec.email         = ["team@solisoft.net"]

  spec.summary       = "Ruby client for SoliDB"
  spec.description   = "A native Ruby client for SoliDB using the binary MessagePack protocol."
  spec.homepage      = "https://solidb.solisoft.net"
  spec.license       = "MIT"
  spec.required_ruby_version = Gem::Requirement.new(">= 2.5.0")

  spec.metadata["homepage_uri"] = spec.homepage
  spec.metadata["source_code_uri"] = "https://github.com/solisoft/solidb"

  # Specify which files should be added to the gem when it is released.
  spec.files = Dir.chdir(File.expand_path(__dir__)) do
    Dir["{lib}/**/*", "README.md", "LICENSE.txt"]
  end
  spec.bindir        = "exe"
  spec.executables   = spec.files.grep(%r{^exe/}) { |f| File.basename(f) }
  spec.require_paths = ["lib"]

  spec.add_dependency "msgpack", "~> 1.7"
  spec.add_development_dependency "rspec", "~> 3.12"
  spec.add_development_dependency "rake", "~> 13.0"
end
