build:
	cargo -Z build-std build --target=x86_64-unknown-linux-gnu

build_release:
	cargo -Z build-std build --target=x86_64-unknown-linux-gnu --release

check:
	cargo clippy --all-targets --all-features
