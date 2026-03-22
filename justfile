# List available recipes
default:
    @just --list

# Type-check without building
check:
    cargo check

# Build debug binary
build:
    cargo build

# Format code
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Run clippy lints
clippy:
    cargo clippy -- -D warnings

# Run tests
test:
    cargo test

# Run full CI suite locally (format check + clippy + coverage)
ci: fmt-check clippy coverage

# Generate LCOV coverage report
coverage: _test-coverage
    grcov . \
        --binary-path ./target/debug/ \
        -s . \
        -t lcov \
        --branch \
        --ignore-not-existing \
        --ignore "/*" \
        --ignore "target/*" \
        --ignore "tests/*" \
        --ignore "build.rs" \
        -o lcov.info
    @echo ""
    @echo "Coverage report written to lcov.info"
    @awk '/^LH:/{h+=substr($0,4)} /^LF:/{t+=substr($0,4)} END{printf "Overall: %d/%d lines (%.1f%%)\n",h,t,(h/t)*100}' lcov.info
    find . -name "*.profraw" -delete

# Generate HTML coverage report
coverage-html: _test-coverage
    grcov . \
        --binary-path ./target/debug/ \
        -s . \
        -t html \
        --branch \
        --ignore-not-existing \
        --ignore "/*" \
        --ignore "target/*" \
        --ignore "tests/*" \
        --ignore "build.rs" \
        -o coverage/
    @echo ""
    @echo "HTML report written to coverage/"
    find . -name "*.profraw" -delete

# Remove coverage artifacts
clean-coverage:
    rm -rf lcov.info coverage/
    find . -name "*.profraw" -delete

_test-coverage:
    find . -name "*.profraw" -delete
    CARGO_INCREMENTAL=0 \
    RUSTFLAGS="-C instrument-coverage" \
    LLVM_PROFILE_FILE="pistachio-%p-%m.profraw" \
    cargo test
