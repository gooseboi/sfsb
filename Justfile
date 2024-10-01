build-arch := if os() == "linux" { "x86_64-unknown-linux-gnu" } else if os() == "windows" { "x86_64-pc-windows-msvc" } else { "unsupported arch" }

build:
	cargo -Z build-std build --target={{build-arch}}

build_release:
	cargo -Z build-std build --target={{build-arch}} --release

run *ARGS: build
	./target/{{build-arch}}/debug/sfsb {{ARGS}}

run_release *ARGS: build_release
	./target/{{build-arch}}/release/sfsb {{ARGS}}

check:
	cargo clippy --all-targets --all-features

test:
	cargo test
