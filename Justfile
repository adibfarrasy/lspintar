default:
    @just --list

tp filter="":
    @cargo test --release --features integration-test -p {{filter}} -- --show-output

tt filter="":
    @cargo test --release --features integration-test {{filter}} -- --show-output

b:
    @cargo build --release

