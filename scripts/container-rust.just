[group("container-rust")]
typos: (_dev "typos")

[group("container-rust")]
sort: (_dev "cargo" "sort" "--workspace")

[group("container-rust")]
sort-check: (_dev "cargo" "sort" "--workspace" "--check")

[group("container-rust")]
shear *ARGS: (_dev "cargo shear --check-test-targets --deny-warnings " + ARGS)

[group("container-rust")]
pup: (_dev "bash" "scripts/pup.sh")

[group("container-rust")]
msrv: (_dev "cargo" "msrv" "verify" "--manifest-path" "Cargo.toml")

[group("container-rust")]
sdk-msrv: (_dev "cargo" "msrv" "verify" "--manifest-path" "sdk/rust/geotrace-sdk/Cargo.toml")

[group("container-rust")]
python-sdk: (_dev "bash" "scripts/python-sdk.sh")
