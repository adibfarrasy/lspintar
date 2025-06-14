default:
    @just --list

note:
    @echo "Compiling notes..."
    @echo "# TODO" > NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "TODO" src >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# FIXME" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "FIXME" src >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# HACK" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "HACK" src >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# WARN" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "WARN" src >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
    @echo "" >> NOTES.md
    @echo "# NOTE" >> NOTES.md
    @if ! grep -rn -H --exclude-dir=target --exclude-dir=.git "NOTE" src >> NOTES.md 2>/dev/null; then echo "-" >> NOTES.md; fi
