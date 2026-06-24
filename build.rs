// Embed the app icon and version metadata into the Windows executable (winres
// is Windows-only, so this compiles away elsewhere). A failure here would ship
// a Windows release without its icon/metadata, so panic and fail loudly.
#![allow(
    clippy::expect_used,
    reason = "a build script must fail loudly if embedding Windows resources fails"
)]

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/geotrace_icon.ico");
        // winres fills version/company from Cargo metadata. Set the display
        // strings explicitly so they read "GeoTrace" rather than the crate name.
        res.set("ProductName", "GeoTrace");
        res.set(
            "FileDescription",
            "GeoTrace - GNSS navigation data visualizer",
        );
        res.set("OriginalFilename", "geotrace.exe");
        res.compile()
            .expect("failed to embed the Windows app icon and metadata");
    }
}
