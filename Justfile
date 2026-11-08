check: check-commit
    bash tests/file-size.sh
    cargo test --locked --all-targets
    just coverage

check-commit:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings

coverage:
    cargo llvm-cov --locked --workspace --all-targets --ignore-filename-regex '(^|/)(tests|examples)/' --fail-under-lines 95

check-file-size:
    bash scripts/check-staged-file-size

hooks-install:
    git config --local core.hooksPath .githooks
