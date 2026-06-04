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
