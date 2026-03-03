default:
    @just --list

tp filter="":
    @cargo test --release --features integration-test -p {{filter}} -- --show-output --test-threads=1

tt filter="":
    @cargo test --release --features integration-test {{filter}} -- --show-output --test-threads=1

b:
    @cargo build --release

