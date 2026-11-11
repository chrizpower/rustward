check: check-commit
    just check-acceptance-fixtures
    bash tests/file-size.sh
    bash tests/file-size-isolation.sh
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

# These commands use synthetic data on an isolated Docker host or prepared VM.
acceptance-container:
    python3 -I -B acceptance/container.py

acceptance-ssh:
    python3 -I -B acceptance/container.py --remote

acceptance-host root tools user="rustward":
    python3 -I -B -c 'import sys; sys.path.insert(0, "acceptance"); from inside import main; sys.exit(main())' --root {{quote(root)}} --tools {{quote(tools)}} --user {{quote(user)}} --source-revision "$(git rev-parse HEAD)"

check-acceptance-fixtures:
    python3 -B -m unittest discover -s acceptance/tests -p 'test_*.py'
