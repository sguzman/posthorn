//! First Wayland layer-shell integration boundary.
//!
//! This is a configure-handshake probe, not a visible notification renderer.
//! It does not acquire the desktop notification D-Bus name and cannot replace
//! Mako yet. The future rendering loop will extend this protocol state.

use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_compositor, wl_registry, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScreenCorner {
    fn anchors(self) -> Anchor {
        match self {
            Self::TopLeft => Anchor::Top | Anchor::Left,
            Self::TopRight => Anchor::Top | Anchor::Right,
            Self::BottomLeft => Anchor::Bottom | Anchor::Left,
            Self::BottomRight => Anchor::Bottom | Anchor::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupBounds {
    pub width: u32,
    pub height: u32,
    pub corner: ScreenCorner,
    pub margin: i32,
}

impl PopupBounds {
    pub fn new(width: u32, height: u32, corner: ScreenCorner, margin: i32)
        -> Result<Self, &'static str>
    {
        if width == 0 || height == 0 {
            return Err("popup width and height must be positive");
        }
        if margin < 0 { return Err("popup margin cannot be negative"); }
        Ok(Self { width, height, corner, margin })
    }
}

#[derive(Debug, Default)]
struct ConfigureState {
    size: Option<(u32, u32)>,
    closed: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ConfigureState {
    fn event(
        _: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
        _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

delegate_noop!(ConfigureState: ignore wl_compositor::WlCompositor);
delegate_noop!(ConfigureState: ignore wl_surface::WlSurface);
delegate_noop!(ConfigureState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for ConfigureState {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer.ack_configure(serial);
                state.size = Some((width, height));
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

/// Opens a non-rendering layer-shell surface and waits for its first configure.
/// Its only purpose is to verify protocol and anchor handling during future QA.
/// Do not use it from the production notification daemon until the render loop
/// and its frame buffer lifetime are implemented.
pub fn probe_configure(bounds: PopupBounds) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<ConfigureState>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ())?;
    let shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;
    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(&surface, None, Layer::Top, "posthorn".into(), &qh, ());

    layer.set_size(bounds.width, bounds.height);
    layer.set_anchor(bounds.corner.anchors());
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);

    let margin = bounds.margin;
    layer.set_margin(
        if matches!(bounds.corner, ScreenCorner::TopLeft | ScreenCorner::TopRight) {margin} else {0},
        if matches!(bounds.corner, ScreenCorner::TopRight | ScreenCorner::BottomRight) {margin} else {0},
        if matches!(bounds.corner, ScreenCorner::BottomLeft | ScreenCorner::BottomRight) {margin} else {0},
        if matches!(bounds.corner, ScreenCorner::TopLeft | ScreenCorner::BottomLeft) {margin} else {0},
    );
    // Layer-shell requires an initial commit without a buffer before configure.
    surface.commit();

    let mut state = ConfigureState::default();
    while state.size.is_none() && !state.closed {
        queue.blocking_dispatch(&mut state)?;
    }
    if state.closed { return Err("layer surface closed before configuration".into()); }
    let configured = state.size.ok_or("layer surface received no configure")?;
    Ok((
        if configured.0 == 0 { bounds.width } else { configured.0 },
        if configured.1 == 0 { bounds.height } else { configured.1 },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_anchors_preserve_mako_default_top_right() {
        assert!(ScreenCorner::TopRight.anchors().contains(Anchor::Top));
        assert!(ScreenCorner::TopRight.anchors().contains(Anchor::Right));
        assert!(!ScreenCorner::TopRight.anchors().contains(Anchor::Bottom));
    }

    #[test]
    fn size_validation_prevents_zero_width_layer_surfaces() {
        assert!(PopupBounds::new(440, 180, ScreenCorner::TopRight, 10).is_ok());
        assert!(PopupBounds::new(0, 180, ScreenCorner::TopRight, 10).is_err());
        assert!(PopupBounds::new(440, 0, ScreenCorner::TopRight, 10).is_err());
        assert!(PopupBounds::new(440, 180, ScreenCorner::TopRight, -5).is_err());
    }
}
