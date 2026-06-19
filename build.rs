// Embed the app icon and version metadata into the Windows executable so
// Explorer, the taskbar, and the installer's Start Menu/Desktop shortcuts show
// it. winres is a Windows-only build dependency, so this whole block compiles
// away on other platforms.
//
// A failure here means a Windows release would ship without its icon/metadata,
// so panic and fail the build loudly rather than hiding it.
#![allow(
    clippy::expect_used,
    reason = "a build script must fail loudly if embedding Windows resources fails"
)]

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/geotrace_icon.ico");
        // winres fills version/company from Cargo metadata; set the display
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
