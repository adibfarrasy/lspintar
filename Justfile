default:
    @just --list

# (c)lean (u)pdate (b)uild
cub:
    @cargo clean && cargo update && cargo build
    @just test

test:
    @echo "Running all tests..."
    @cargo test

reset-db filter="./dev.db":
    rm -f {{filter}}
    export DATABASE_URL="sqlite:{{filter}}" && \
    sqlx database create && \
    sqlx migrate run

tp filter="":
    @cargo test -p {{filter}} -- --show-output  

