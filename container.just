IMAGE     := "geotrace-dev"
SDK_IMAGE := "geotrace-sdk-dev"

_runner := `if command -v docker >/dev/null 2>&1; then echo docker; elif command -v podman >/dev/null 2>&1; then echo podman; fi`
RUNNER  := _runner

[private]
_require-runner:
    @[ -n "{{ RUNNER }}" ] || { echo "error: neither docker nor podman found; install one to use container recipes" >&2; exit 1; }

# Run a command inside the Rust dev container, or directly if already in one.
[private]
_dev *args:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "${IN_GEOTRACE_CONTAINER:-}" == "1" ]] && { {{ args }}; exit; }
    just _require-runner
    {{ RUNNER }} run --rm \
        --user "$(id -u):$(id -g)" \
        -e IN_GEOTRACE_CONTAINER=1 \
        -e HOME=/tmp \
        -e RUSTUP_HOME=/root/.rustup \
        -e CARGO_HOME=/root/.cargo \
        -v "{{justfile_directory()}}:/workspace" \
        -v "$HOME/.cargo/registry:/root/.cargo/registry" \
        -v "$HOME/.cargo/git:/root/.cargo/git" \
        -v "geotrace-target:/workspace/target" \
        -w /workspace \
        {{ IMAGE }} \
        {{ args }}

# Run a command inside the SDK dev container (adds cmake build volumes), or directly if already in one.
[private]
_sdk *args:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "${IN_GEOTRACE_CONTAINER:-}" == "1" ]] && { {{ args }}; exit; }
    just _require-runner
    {{ RUNNER }} run --rm \
        --user "$(id -u):$(id -g)" \
        -e IN_GEOTRACE_CONTAINER=1 \
        -v "{{justfile_directory()}}:/workspace" \
        -v "$HOME/.cargo/registry:/root/.cargo/registry" \
        -v "$HOME/.cargo/git:/root/.cargo/git" \
        -v "geotrace-target:/workspace/target" \
        -v "geotrace-cmake-c:/workspace/sdk/c/build" \
        -v "geotrace-cmake-cpp:/workspace/sdk/cpp/build" \
        -w /workspace \
        {{ SDK_IMAGE }} \
        {{ args }}

# Open an interactive shell inside the Rust dev container.
[group("shells")]
dev-shell: _require-runner
    {{ RUNNER }} run --rm -it \
        --user "$(id -u):$(id -g)" \
        -e IN_GEOTRACE_CONTAINER=1 \
        -e HOME=/tmp \
        -e RUSTUP_HOME=/root/.rustup \
        -e CARGO_HOME=/root/.cargo \
        -v "{{justfile_directory()}}:/workspace" \
        -v "$HOME/.cargo/registry:/root/.cargo/registry" \
        -v "$HOME/.cargo/git:/root/.cargo/git" \
        -v "geotrace-target:/workspace/target" \
        -w /workspace \
        {{ IMAGE }} \
        bash

# Open an interactive shell inside the SDK dev container.
[group("shells")]
dev-shell-sdk: _require-runner
    {{ RUNNER }} run --rm -it \
        --user "$(id -u):$(id -g)" \
        -e IN_GEOTRACE_CONTAINER=1 \
        -v "{{justfile_directory()}}:/workspace" \
        -v "$HOME/.cargo/registry:/root/.cargo/registry" \
        -v "$HOME/.cargo/git:/root/.cargo/git" \
        -v "geotrace-target:/workspace/target" \
        -v "geotrace-cmake-c:/workspace/sdk/c/build" \
        -v "geotrace-cmake-cpp:/workspace/sdk/cpp/build" \
        -w /workspace \
        {{ SDK_IMAGE }} \
        bash

# Build the Rust dev image (stage 1).
[group("images")]
build-image: _require-runner
    {{ RUNNER }} build --target rust-dev -t {{ IMAGE }} .

# Build the SDK dev image (stage 2 - extends the Rust image).
[group("images")]
build-sdk-image: _require-runner
    {{ RUNNER }} build --target sdk-dev -t {{ SDK_IMAGE }} .

# Build both images (stage 2 implicitly builds stage 1 first).
[group("images")]
build-images: build-sdk-image

[group("utils")]
osv-scanner: _require-runner
    {{ RUNNER }} run --rm \
        -v "{{justfile_directory()}}:/src:ro" \
        ghcr.io/google/osv-scanner:latest \
        scan --lockfile /src/Cargo.lock --config /src/osv-scanner.toml
