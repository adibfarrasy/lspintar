default:
    @just --list

reset-db filter="./dev.db":
    rm -f {{filter}}
    export DATABASE_URL="sqlite:{{filter}}" && \
    sqlx database create && \
    sqlx migrate run

tp filter="":
    @cargo test --release -p {{filter}} -- --show-output  

tt filter="":
    @cargo test --release {{filter}} -- --show-output --test-threads=1

