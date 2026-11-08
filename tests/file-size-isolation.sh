#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r name; do
    unset "$name"
done < <(git rev-parse --local-env-vars)

root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d)
trap 'result=$?; if (( result )); then
    evidence="$HOME/artifacts/rustward/file-size-tests/$(basename "$scratch")"
    mkdir -p "$evidence"
    cp -R "$scratch/." "$evidence/"
    printf "Git isolation test failed. Evidence: %s\n" "$evidence" >&2
else
    rm -rf "$scratch"
fi' EXIT

git init -q "$scratch/parent"
cd "$scratch/parent"
git config core.hooksPath /dev/null
git config user.name 'Parent test'
git config user.email 'parent@example.invalid'
git config commit.gpgSign false
printf 'Retain this staged content.\n' > retained
git add retained
git commit -qm 'Add parent fixture'
printf 'Retain this staged change.\n' >> retained
git add retained
cp .git/config "$scratch/config"
cp .git/index "$scratch/index"
head=$(git rev-parse HEAD)
git count-objects -v > "$scratch/objects"

check_parent() {
    cmp .git/config "$scratch/config"
    cmp .git/index "$scratch/index"
    test "$(git rev-parse HEAD)" = "$head"
    git count-objects -v > "$scratch/objects-after"
    cmp "$scratch/objects" "$scratch/objects-after"
}

GIT_DIR="$PWD/.git" bash "$root/tests/file-size.sh"
check_parent

GIT_DIR="$PWD/.git" \
GIT_COMMON_DIR="$PWD/.git" \
GIT_WORK_TREE="$PWD" \
GIT_INDEX_FILE="$PWD/.git/index" \
GIT_OBJECT_DIRECTORY="$PWD/.git/objects" \
GIT_CONFIG_COUNT=1 \
GIT_CONFIG_KEY_0=core.bare \
GIT_CONFIG_VALUE_0=true \
    bash "$root/tests/file-size.sh"
check_parent

printf 'Git repository isolation checks passed.\n'
