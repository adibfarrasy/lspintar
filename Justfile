default:
    @just --list

note:
    @echo "Compiling notes..."
    @echo "# TODO" > NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "TODO" src | sed 's/:[[:space:]]*\/\/[[:space:]]*TODO:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# FIXME" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "FIXME" src | sed 's/:[[:space:]]*\/\/[[:space:]]*FIXME:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# HACK" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "HACK" src | sed 's/:[[:space:]]*\/\/[[:space:]]*HACK:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# WARN" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "WARN" src | sed 's/:[[:space:]]*\/\/[[:space:]]*WARN:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# NOTE" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "NOTE" src | sed 's/:[[:space:]]*\/\/[[:space:]]*NOTE:[[:space:]]*/: /' >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi

cub:
    @cargo clean && cargo update && cargo build

b:
    @echo "RUSTFLAGS=-A warnings cargo build"
    @RUSTFLAGS="-A warnings" cargo build
    @just note
