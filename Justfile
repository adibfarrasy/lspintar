default:
    @just --list

tp filter="":
    @cargo test --release -p {{filter}} -- --show-output  

tt filter="":
    @cargo test --release {{filter}} -- --show-output --test-threads=1

b:
    @cargo build --release

