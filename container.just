IMAGE := "geotrace-dev"
SDK_IMAGE := "geotrace-sdk-dev"

[private]
RUNNER := `if command -v podman >/dev/null 2>&1; then printf "podman"; elif command -v docker >/dev/null 2>&1; then printf "docker"; fi`

[private]
_require-runner:
    @[ -n "{{ RUNNER }}" ] || { echo "error: neither docker nor podman found; install one to use container recipes" >&2; exit 1; }

# Podman requires userns=keep-id to map host rootless volumes correctly.
USER_ARGS := if RUNNER == "podman" { "--userns=keep-id" } else { "--user $(id -u):$(id -g)" }

SDK_VOLUMES := "\
    -v geotrace-cmake-c:/workspace/sdk/c/build \
    -v geotrace-cmake-cpp:/workspace/sdk/cpp/build \
"

# Extract common arguments to prevent drift and fix the missing HOME in SDK
COMMON_RUN_ARGS := "--rm --init " + USER_ARGS + " \
    -e IN_GEOTRACE_CONTAINER=1 \
    -e HOME=/tmp \
    -e RUSTUP_HOME=/usr/local/rustup \
    -e CARGO_HOME=/usr/local/cargo \
    -v " + justfile_directory() + ":/workspace \
    -v $HOME/.cargo/registry:/usr/local/cargo/registry \
    -v $HOME/.cargo/git:/usr/local/cargo/git \
    -v geotrace-target:/workspace/target \
    -w /workspace"

# Run a command inside the Rust dev container, or directly if already in one.
[private]
_dev *args:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "${IN_GEOTRACE_CONTAINER:-}" == "1" ]] && { {{ args }}; exit; }
    just _require-runner
    echo "{{ GREEN }}{{ BOLD }}{{ args }}{{ NORMAL }}"
    {{ RUNNER }} run {{ COMMON_RUN_ARGS }} {{ IMAGE }} {{ args }}

# Run a command inside the SDK dev container (adds cmake build volumes), or directly if already in one.
[private]
_sdk *args:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "${IN_GEOTRACE_CONTAINER:-}" == "1" ]] && { {{ args }}; exit; }
    just _require-runner
    echo "{{ GREEN }}{{ BOLD }}{{ args }}{{ NORMAL }}"
    {{ RUNNER }} run {{ COMMON_RUN_ARGS }} {{ SDK_VOLUMES }} {{ SDK_IMAGE }} {{ args }}

# Open an interactive shell inside the Rust dev container.
[group("shells")]
dev-shell: _require-runner
    {{ RUNNER }} run -it {{ COMMON_RUN_ARGS }} {{ IMAGE }} bash

# Open an interactive shell inside the SDK dev container.
[group("shells")]
dev-shell-sdk: _require-runner
    {{ RUNNER }} run -it {{ COMMON_RUN_ARGS }} {{ SDK_VOLUMES }} {{ SDK_IMAGE }} bash

# Build the Rust dev image (stage 1).
[group("images")]
build-image: _require-runner
    {{ RUNNER }} build --target rust-dev -t {{ IMAGE }} .

# Build the SDK dev image (stage 2 - extends the Rust image).
[group("images")]
build-sdk-image: _require-runner
    {{ RUNNER }} build --target sdk-dev -t {{ SDK_IMAGE }} .

# Build both images.
[group("images")]
build-images: build-image build-sdk-image

[group("utils")]
osv-scanner: _require-runner
    {{ RUNNER }} run --rm \
        -v "{{ justfile_directory() }}:/src:ro" \
        ghcr.io/google/osv-scanner:latest \
        scan --lockfile /src/Cargo.lock --config /src/osv-scanner.toml
