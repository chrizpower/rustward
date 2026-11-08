#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
checker="$root/scripts/check-staged-file-size"
scratch=$(mktemp -d)
trap 'result=$?; if (( result )); then
    evidence="$HOME/artifacts/rustward/file-size-tests/$(basename "$scratch")"
    mkdir -p "$evidence"
    cp "$scratch/result" "$evidence/result" 2>/dev/null || true
    printf "File size test failed. Evidence: %s\n" "$evidence" >&2
fi' EXIT
git init -q "$scratch/repository"
cd "$scratch/repository"
git config core.hooksPath /dev/null
git config user.name 'Synthetic test'
git config user.email 'test@example.invalid'
git config commit.gpgSign false

size_file() {
    head -c "$2" /dev/zero | tr '\0' x > "$1"
}

passes() {
    bash "$checker" > "$scratch/result" 2>&1
}

fails() {
    if passes; then
        printf 'Expected the staged file size check to fail.\n' >&2
        exit 1
    else
        test "$?" -eq 1
    fi
    grep -q 'Split each file listed above' "$scratch/result"
    grep -q 'Stage all split files' "$scratch/result"
}

passes
size_file source.rs 20000
git add source.rs
passes
size_file source.rs 20001
git add source.rs
fails
grep -q '20001 staged bytes' "$scratch/result"

# Unstaged content must not change the result for the staged blob.
size_file source.rs 1
fails
git add source.rs
size_file source.rs 20001
passes

# Attribute changes take effect only when staged.
size_file generated.rs 20001
git add generated.rs
printf 'generated.rs -token-limit\n' > .gitattributes
fails
git add .gitattributes
passes
printf 'generated.rs token-limit\n' > .gitattributes
passes
git add .gitattributes
fails
cp "$root/.gitattributes" .gitattributes
printf 'generated.rs -token-limit\n' >> .gitattributes
size_file Cargo.lock 30000
git add .gitattributes Cargo.lock
passes

# NUL-delimited paths must preserve spaces, newlines, and pathspec characters.
for name in 'two words.rs' $'line\nbreak.rs' '[part].rs' ':part.rs'; do
    size_file "$name" 20001
    git --literal-pathspecs add -- "$name"
    fails
    size_file "$name" 20000
    git --literal-pathspecs add -- "$name"
    passes
done

git commit -qm 'Add synthetic file size fixtures'
git mv source.rs renamed.rs
size_file renamed.rs 20001
git add renamed.rs
fails
size_file renamed.rs 20000
git add renamed.rs
passes
git commit -qm 'Rename synthetic fixture'
git rm -q renamed.rs
passes
git update-index --add --cacheinfo 160000,1111111111111111111111111111111111111111,external
passes

printf 'Staged file size checks passed.\n'
