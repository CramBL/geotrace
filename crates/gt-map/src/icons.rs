//! Marker icon assets still on the texture path: URI constants, startup
//! registration, and the rotated ghost-chevron drawing.
//! The marker renderers draw from pre-tessellated meshes (see
//! [crate::icon_mesh]); migrating the ghost chevrons retires this module.

// URI constants used by the marker renderer and the startup registration call.
pub(crate) const ICON_URI_LIGHTNING: &str = "bytes://gt-map/icons/lightning.svg";
pub(crate) const ICON_URI_CONNECTION_LOST: &str = "bytes://gt-map/icons/connection_lost.svg";
pub(crate) const ICON_URI_WARNING: &str = "bytes://gt-map/icons/warning.svg";
pub(crate) const ICON_URI_ERROR: &str = "bytes://gt-map/icons/error.svg";
pub(crate) const ICON_URI_LOG_PIN: &str = "bytes://gt-map/icons/log_pin.svg";
pub(crate) const ICON_URI_PIN: &str = "bytes://gt-map/icons/pin.svg";
pub(crate) const ICON_URI_CROSS: &str = "bytes://gt-map/icons/cross.svg";
pub(crate) const ICON_URI_CIRCLE_MARKER: &str = "bytes://gt-map/icons/circle_marker.svg";
pub(crate) const ICON_URI_CHECK: &str = "bytes://gt-map/icons/check.svg";
pub(crate) const ICON_URI_SATELLITE: &str = "bytes://gt-map/icons/satellite.svg";
pub(crate) const ICON_URI_SATELLITE_LOST: &str = "bytes://gt-map/icons/satellite_lost.svg";
pub(crate) const ICON_URI_GEAR: &str = "bytes://gt-map/icons/gear.svg";
pub(crate) const ICON_URI_REFRESH: &str = "bytes://gt-map/icons/refresh.svg";
pub(crate) const ICON_URI_DOWNLOAD: &str = "bytes://gt-map/icons/download.svg";
pub(crate) const ICON_URI_UPLOAD: &str = "bytes://gt-map/icons/upload.svg";
pub(crate) const ICON_URI_WRENCH: &str = "bytes://gt-map/icons/wrench.svg";
pub(crate) const ICON_URI_GHOST_FIX: &str = "bytes://gt-map/icons/ghost_fix.svg";

/// Register the embedded SVG marker icons with the egui context.
///
/// Call this once at startup (before the first frame) from your `App::new`
/// implementation, **after** [`egui_extras::install_image_loaders`] has been
/// called. The icons are compiled into the binary via `include_bytes!` and
/// cached by egui's texture system after their first rasterisation. Subsequent
/// frames pay only a GPU quad draw - no CPU tessellation, no heap allocation.
macro_rules! icon_bytes {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/icons/",
            $name
        ))
        .as_slice()
    };
}

pub fn register_marker_icons(ctx: &egui::Context) {
    ctx.include_bytes(ICON_URI_LIGHTNING, icon_bytes!("lightning.svg"));
    ctx.include_bytes(ICON_URI_CONNECTION_LOST, icon_bytes!("connection_lost.svg"));
    ctx.include_bytes(ICON_URI_WARNING, icon_bytes!("warning.svg"));
    ctx.include_bytes(ICON_URI_ERROR, icon_bytes!("error.svg"));
    ctx.include_bytes(ICON_URI_LOG_PIN, icon_bytes!("log_pin.svg"));
    ctx.include_bytes(ICON_URI_PIN, icon_bytes!("pin.svg"));
    ctx.include_bytes(ICON_URI_CROSS, icon_bytes!("cross.svg"));
    ctx.include_bytes(ICON_URI_CIRCLE_MARKER, icon_bytes!("circle_marker.svg"));
    ctx.include_bytes(ICON_URI_CHECK, icon_bytes!("check.svg"));
    ctx.include_bytes(ICON_URI_SATELLITE, icon_bytes!("satellite.svg"));
    ctx.include_bytes(ICON_URI_SATELLITE_LOST, icon_bytes!("satellite_lost.svg"));
    ctx.include_bytes(ICON_URI_GEAR, icon_bytes!("gear.svg"));
    ctx.include_bytes(ICON_URI_REFRESH, icon_bytes!("refresh.svg"));
    ctx.include_bytes(ICON_URI_DOWNLOAD, icon_bytes!("download.svg"));
    ctx.include_bytes(ICON_URI_UPLOAD, icon_bytes!("upload.svg"));
    ctx.include_bytes(ICON_URI_WRENCH, icon_bytes!("wrench.svg"));
    ctx.include_bytes(ICON_URI_GHOST_FIX, icon_bytes!("ghost_fix.svg"));
}
/// Draw a rotated SVG icon centred on `center`, with the icon's "up" direction aligned to
/// `direction`.
///
/// The icon is rendered as a rotated [`egui::epaint::Mesh`] quad so it can be oriented to any
/// travel direction without re-rasterising the SVG. `size` is the half-extent: the quad spans
/// `2*size × 2*size` pixels. The white SVG stroke is multiplied by `tint` at render time so a
/// single texture serves all colours.
pub(crate) fn draw_rotated_cached_icon(
    ui: &egui::Ui,
    uri: &'static str,
    center: egui::Pos2,
    direction: egui::Vec2,
    size: f32,
    tint: egui::Color32,
) {
    let cache_key = egui::Id::new(("gt_icon_tex", uri));
    let tex_id = if let Some(id) = ui.ctx().data(|d| d.get_temp::<egui::TextureId>(cache_key)) {
        id
    } else if let Ok(egui::load::TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
        uri,
        egui::TextureOptions::LINEAR,
        egui::load::SizeHint::default(),
    ) {
        ui.ctx().data_mut(|d| d.insert_temp(cache_key, texture.id));
        texture.id
    } else {
        return;
    };

    // Rotate the four corners of a [-size, size]² quad so the SVG's "up" direction (0, −1)
    // aligns with `direction`. Rotation matrix R where R*(0,−1) = (dx,dy):
    //   R*[px, py] = (−px·dy − py·dx,  px·dx − py·dy)
    let dx = direction.x;
    let dy = direction.y;
    let corner_offsets: [([f32; 2], egui::Pos2); 4] = [
        ([-size, -size], egui::pos2(0.0, 0.0)), // top-left    → UV (0,0)
        ([size, -size], egui::pos2(1.0, 0.0)),  // top-right   → UV (1,0)
        ([size, size], egui::pos2(1.0, 1.0)),   // bottom-right → UV (1,1)
        ([-size, size], egui::pos2(0.0, 1.0)),  // bottom-left → UV (0,1)
    ];

    let mut mesh = egui::epaint::Mesh::with_texture(tex_id);
    for ([px, py], uv) in corner_offsets {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + egui::vec2(-px * dy - py * dx, px * dx - py * dy),
            uv,
            color: tint,
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    ui.painter().add(egui::Shape::Mesh(mesh.into()));
}

/// Spacing between an icon and the text following it in labels.
pub(crate) const ICON_GAP: &str = "  ";
