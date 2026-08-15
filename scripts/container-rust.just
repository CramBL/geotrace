[group("container-rust")]
typos: (_dev "typos")

[group("container-rust")]
sort: (_dev "cargo" "sort" "--workspace")

[group("container-rust")]
sort-check: (_dev "cargo sort --workspace --check")

[group("container-rust")]
shear *ARGS: (_dev "cargo shear --check-test-targets --deny-warnings " + ARGS)

# msrv runs natively rather than via `_dev`: cargo-msrv pulls many toolchains,
# kept out of the dev image to keep it slim. Install it natively (see CI).
[group("native-checks")]
msrv:
    cargo msrv verify --manifest-path Cargo.toml

[group("native-checks")]
sdk-msrv:
    cargo msrv verify --manifest-path sdk/rust/geotrace-sdk/Cargo.toml

[group("container-rust")]
python-sdk: (_dev "bash" "scripts/python-sdk.sh")
