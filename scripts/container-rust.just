[group("container-rust")]
typos: (_dev "typos")

[group("container-rust")]
sort: (_dev "cargo" "sort" "--workspace")

[group("container-rust")]
sort-check: (_dev "cargo sort --workspace --check")

[group("container-rust")]
shear *ARGS: (_dev "cargo shear --check-test-targets --deny-warnings " + ARGS)

# pup/msrv run natively rather than via `_dev`: cargo-pup needs a pinned nightly
# with rustc-dev and cargo-msrv pulls many toolchains, both kept out of the dev
# image to keep it slim. Install them natively (see justfile `setup-pup`, CI).
[group("native-checks")]
pup:
    cargo test -p gt-arch

[group("native-checks")]
msrv:
    cargo msrv verify --manifest-path Cargo.toml

[group("native-checks")]
sdk-msrv:
    cargo msrv verify --manifest-path sdk/rust/geotrace-sdk/Cargo.toml

[group("container-rust")]
python-sdk: (_dev "bash" "scripts/python-sdk.sh")
