# Build libgeotrace_c (release) inside the SDK container.
# Must run before lint-c, test-c, lint-cpp, and test-cpp, which all require the
# library to be present in the target volume.
[group("container-sdk")]
build-c: (_sdk "cargo build -p geotrace-c --release")

# Regenerate C++ and Python recognized-unit catalogs from the Rust catalog.
[group("utils")]
generate-unit-bindings:
    cargo run -p geotrace-sdk-units --example generate_bindings

# Verify committed recognized-unit bindings match the Rust catalog.
[group("ci")]
check-unit-bindings:
    cargo run -p geotrace-sdk-units --example generate_bindings -- --check

# Build and run the C SDK unit/integration tests.
[group("container-sdk")]
test-c: (_sdk "bash" "scripts/test-c.sh")

# C SDK tests with AddressSanitizer + UndefinedBehaviorSanitizer.
[group("container-sdk")]
test-c-asan: (_sdk "bash" "scripts/test-c-asan.sh")

# C SDK tests with ThreadSanitizer (mutually exclusive with ASAN).
[group("container-sdk")]
test-c-tsan: (_sdk "bash" "scripts/test-c-tsan.sh")

# Build and run the C++ SDK unit/integration tests.
[group("container-sdk")]
test-cpp: (_sdk "bash" "scripts/test-cpp.sh")

# C++ SDK tests with AddressSanitizer + UndefinedBehaviorSanitizer.
[group("container-sdk")]
test-cpp-asan: (_sdk "bash" "scripts/test-cpp-asan.sh")

# Build the C++ SDK examples.
[group("container-sdk")]
examples-cpp: (_sdk "bash" "scripts/examples-cpp.sh")

# Install the C and C++ SDKs into /tmp/gtd-sdk and verify find_package works.
[group("container-sdk")]
test-install: (_sdk "bash" "scripts/test-install.sh")

# Install via vcpkg overlay port and verify a consumer project builds.
[group("container-sdk")]
test-install-vcpkg: (_sdk "bash" "scripts/test-install-vcpkg.sh")

# clang-format check + clang-tidy for the C SDK.
[group("container-sdk")]
lint-c: (_sdk "bash" "scripts/lint-c.sh")

# clang-format check + clang-tidy for the C++ SDK.
[group("container-sdk")]
lint-cpp: (_sdk "bash" "scripts/lint-cpp.sh")

# Apply clang-format in-place to C SDK sources.
[group("container-sdk")]
fmt-c: (_sdk "bash" "scripts/fmt-c.sh")

# Apply clang-format in-place to C++ SDK sources.
[group("container-sdk")]
fmt-cpp: (_sdk "bash" "scripts/fmt-cpp.sh")

# Regenerate all four gold files, compare them, then delete the generated three.
[group("container-sdk")]
test-gold-all: test-gold-rs test-gold-c test-gold-cpp test-gold-py test-gold-compare
    # A failed comparison stops before this, leaving the files to inspect.
    rm -f tests/fixtures/gold_dataset/gold_c.gtd \
        tests/fixtures/gold_dataset/gold_cpp.gtd \
        tests/fixtures/gold_dataset/gold_py.gtd

[group("container-sdk")]
test-gold-rs: (_sdk "cargo run --example gold_dataset -p geotrace-sdk")

[group("container-sdk")]
test-gold-c: (_sdk "bash" "scripts/gold-c.sh")

[group("container-sdk")]
test-gold-cpp: (_sdk "bash" "scripts/gold-cpp.sh")

[group("container-sdk")]
test-gold-py: (_sdk "bash" "scripts/gold-py.sh")

# Assert the four freshly generated gold files decode to the same NavFile.
[group("container-sdk")]
test-gold-compare: (_sdk "cargo test -p geotrace-sdk --test gold_conformance")
