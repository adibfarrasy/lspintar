default:
    @just --list

note:
    @echo "Compiling notes..."
    @echo "# TODO" > NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "// TODO" src | sed 's/:[[:space:]]*\/\/[[:space:]]*TODO:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# FIXME" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "// FIXME" src | sed 's/:[[:space:]]*\/\/[[:space:]]*FIXME:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# HACK" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "// HACK" src | sed 's/:[[:space:]]*\/\/[[:space:]]*HACK:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# WARN" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "// WARN" src | sed 's/:[[:space:]]*\/\/[[:space:]]*WARN:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# NOTE" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "// NOTE" src | sed 's/:[[:space:]]*\/\/[[:space:]]*NOTE:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi

cub:
    @cargo clean && cargo update && cargo build

# Fast build with essential tests only
b:
    @echo "cargo build"
    @cargo build
    @just test
    @just note

# Full build with all tests (slower)
b-full:
    @echo "RUSTFLAGS=-A warnings cargo build"
    @RUSTFLAGS="-A warnings" cargo build
    @just test
    @just note

# Run all tests
test:
    @echo "Running all tests..."
    @cargo test

# Quick test run (essential tests only) - much faster
test-quick:
    @echo "Running quick essential tests..."
    @cargo test --quiet -- --test-threads=1 test_dependency_cache_creation
    @cargo test --quiet -- --test-threads=1 test_symbol_type_is_declaration
    @cargo test --quiet -- --test-threads=1 test_language_detection
