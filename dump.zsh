#!/bin/zsh
# dump_rust_only.zsh  –  ONLY .rs files, perfect for AI upload

output="NOMOS_RUST_SOURCES_ONLY.txt"
rm -f "$output"

print "# Nomos Testing Framework — Rust sources only — $(date)\r" > "$output"
echo "Dumping only .rs files (no target, no lockfile, no generated code)...\n"

counter=0

# Find ONLY .rs files, skip everything unwanted
for file in **/*.rs(N); do
    # Skip if in target/ or any generated dir
    [[ "$file" == target/* ]] && continue
    [[ "$file" == */target/* ]] && continue
    [[ "$file" == "$output" ]] && continue

    # Optional: skip files > 300 KB (usually generated)
    size=$(stat -f%z "$file" 2>/dev/null || stat -c%s "$file" 2>/dev/null || echo 0)
    (( size > 300000 )) && continue

    counter=$((counter + 1))
    printf "\rProgress: %4d files → %s" $counter "${file:t}"

    {
        print "\r\n════════════════════════════════════════════════\r"
        print "FILE: $file\r"
        print "────────────────────────────────────────────────\r"
        cat "$file"
        print "\r\n\r\n"
    } >> "$output"
done

printf "\r%s\r" "$(tput el 2>/dev/null || echo)"
echo "\nDONE! Pure Rust sources → $output"
echo "   Files : $counter"
echo "   Size  : $(du -h "$output" | cut -f1)"
ls -lh "$output"
