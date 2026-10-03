# Convenience targets. Requires the Dioxus CLI (`dx`, 0.7.x) and the
# wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`).

APP := crates/app

.PHONY: web desktop serve-web run-desktop test lint renders spectrograms bench clean

## Serve the web build on http://127.0.0.1:8080
serve-web:
	cd $(APP) && dx serve --web --release

## Run the native Linux app (PulseAudio / PipeWire-pulse)
run-desktop:
	cd $(APP) && dx serve --desktop --release

## Release bundles: target/dx/kickwerk/release/{web/public,linux/app}
web:
	cd $(APP) && dx build --web --release

desktop:
	cd $(APP) && dx build --desktop --release

test:
	cargo test --release -p kickwerk-dsp -p kickwerk

lint:
	cargo fmt --all --check
	cargo clippy --all-targets -p kickwerk-dsp -p kickwerk-worklet -- -D warnings
	cargo clippy --all-targets -p kickwerk --features desktop -- -D warnings
	cargo clippy -p kickwerk --features web --target wasm32-unknown-unknown -- -D warnings

## Render every factory patch to ./renders/*.wav
renders:
	cargo run --release -p kickwerk-dsp --example render -- renders

## Waveform + spectrogram PNG of every factory patch in ./renders
spectrograms:
	cargo run --release -p kickwerk-dsp --example spectrogram -- renders

bench:
	cargo run --release -p kickwerk-dsp --example bench

clean:
	cargo clean
