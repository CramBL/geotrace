# portfile.cmake - overlay port for the GeoTrace C SDK.
#
# This is a development/CI overlay port that builds from the local repo
# checkout.  It is NOT intended for submission to the vcpkg registry.
#
# Usage (from the repo root, with VCPKG_ROOT set):
#   vcpkg install geotrace-c \
#     --overlay-ports=sdk/integration-tests/vcpkg-port \
#     --triplet x64-linux

set(VCPKG_BUILD_TYPE release)

# SOURCE_PATH points at the repo root (three levels up from this file).
set(SOURCE_PATH "${CMAKE_CURRENT_LIST_DIR}/../../..")

# Build the Rust library that underpins the C ABI.
vcpkg_execute_build_process(
    COMMAND cargo build -p geotrace-c --release
    WORKING_DIRECTORY "${SOURCE_PATH}"
    LOGNAME "cargo-build-geotrace-c"
)

# Configure and install the C SDK using its own CMakeLists.txt.
vcpkg_cmake_configure(
    SOURCE_PATH "${SOURCE_PATH}/sdk/c"
    OPTIONS
        -DGEOTRACE_C_LIB_DIR=${SOURCE_PATH}/target/release
)

vcpkg_cmake_install()

# The config files are already in lib/cmake/GeoTraceC/ as installed by our
# CMakeLists.  vcpkg's toolchain appends the installed prefix to
# CMAKE_PREFIX_PATH, so find_package(GeoTraceC) resolves without a
# `vcpkg_cmake_config_fixup` step.  Skipping it avoids breaking the relocatable
# paths baked in by `configure_package_config_file`.

file(REMOVE_RECURSE "${CURRENT_PACKAGES_DIR}/debug/include")

vcpkg_install_copyright(FILE_LIST "${SOURCE_PATH}/sdk/rust/geotrace-sdk/LICENSE")
