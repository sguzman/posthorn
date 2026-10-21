//! First Wayland layer-shell integration boundary.
//!
//! This is a configure-handshake probe, not a visible notification renderer.
//! It does not acquire the desktop notification D-Bus name and cannot replace
//! Mako yet. The future rendering loop will extend this protocol state.

use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch},
    WEnum,
    Connection, Dispatch, QueueHandle,
};
use std::collections::HashMap;
use std::io::Write;
use nix::poll::{poll, PollFd, PollFlags};
use std::os::fd::AsFd;
use std::time::Duration;
use crate::render::PixelFrame;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupLayer {
    Background, Bottom, Top, Overlay,
}
impl PopupLayer {
    pub fn from_mako(value:&str)->Option<Self>{
        match value {
            "background"=>Some(Self::Background),
            "bottom"=>Some(Self::Bottom),
            "top"=>Some(Self::Top),
            "overlay"=>Some(Self::Overlay),
            _=>None,
        }
    }
    fn protocol(self)->Layer{
        match self {
            Self::Background=>Layer::Background,
            Self::Bottom=>Layer::Bottom,
            Self::Top=>Layer::Top,
            Self::Overlay=>Layer::Overlay,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenCorner {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl ScreenCorner {
    pub fn from_mako(value:&str)->Option<Self>{
        match value {
            "top-left"=>Some(Self::TopLeft),
            "top-center"=>Some(Self::TopCenter),
            "top-right"=>Some(Self::TopRight),
            "center-left"=>Some(Self::CenterLeft),
            "center"=>Some(Self::Center),
            "center-right"=>Some(Self::CenterRight),
            "bottom-left"=>Some(Self::BottomLeft),
            "bottom-center"=>Some(Self::BottomCenter),
            "bottom-right"=>Some(Self::BottomRight),
            _=>None,
        }
    }

    fn anchors(self) -> Anchor {
        match self {
            Self::TopLeft=>Anchor::Top|Anchor::Left,
            Self::TopCenter=>Anchor::Top,
            Self::TopRight=>Anchor::Top|Anchor::Right,
            Self::CenterLeft=>Anchor::Left,
            Self::Center=>Anchor::empty(),
            Self::CenterRight=>Anchor::Right,
            Self::BottomLeft=>Anchor::Bottom|Anchor::Left,
            Self::BottomCenter=>Anchor::Bottom,
            Self::BottomRight=>Anchor::Bottom|Anchor::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupMargins {
    pub top:i32, pub right:i32, pub bottom:i32, pub left:i32,
}
impl PopupMargins {
    pub fn uniform(n:i32)->Self{
        Self{top:n,right:n,bottom:n,left:n}
    }
    /// Mako's one-to-four value CSS-style margin syntax.
    pub fn parse_mako(raw:&str)->Option<Self>{
        let values:Vec<i32>=raw.split(',').map(|v|v.trim().parse()).collect::<Result<_,_>>().ok()?;
        match values.as_slice() {
            [a]=>Some(Self::uniform(*a)),
            [v,h]=>Some(Self{top:*v,right:*h,bottom:*v,left:*h}),
            [t,h,b]=>Some(Self{top:*t,right:*h,bottom:*b,left:*h}),
            [t,r,b,l]=>Some(Self{top:*t,right:*r,bottom:*b,left:*l}),
            _=>None,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerClick {
    pub x: f64,
    pub y: f64,
    /// Linux input-event button codes: left 272, right 273, middle 274.
    pub button: u32,
    pub serial: u32,
}

#[derive(Debug, Default)]
struct ConfigureState {
    size: Option<(u32, u32)>,
    closed: bool,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    touch_contacts: HashMap<i32,(f64,f64)>,
    active_surface: Option<wl_surface::WlSurface>,
    pointer_inside: bool,
    pointer_x: f64,
    pointer_y: f64,
    clicks: Vec<PointerClick>,
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
delegate_noop!(ConfigureState: ignore wl_shm::WlShm);
delegate_noop!(ConfigureState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ConfigureState: ignore wl_buffer::WlBuffer);

impl Dispatch<wl_seat::WlSeat, ()> for ConfigureState {
    fn event(
        state: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event,
        _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            if let WEnum::Value(flags) = capabilities {
                if flags.contains(wl_seat::Capability::Pointer) {
                    if state.pointer.is_none() {
                        state.pointer = Some(seat.get_pointer(qh, ()));
                    }
                } else {
                    if let Some(pointer) = state.pointer.take() { pointer.release(); }
                    state.pointer_inside = false;
                }
                if flags.contains(wl_seat::Capability::Touch) {
                    if state.touch.is_none() {
                        state.touch = Some(seat.get_touch(qh, ()));
                    }
                } else {
                    if let Some(touch)=state.touch.take(){touch.release();}
                    state.touch_contacts.clear();
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for ConfigureState {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } => {
                state.pointer_inside = state.active_surface.as_ref()==Some(&surface);
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Leave { .. } => state.pointer_inside = false,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Button { serial, button, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. }
                if state.pointer_inside =>
            {
                state.clicks.push(PointerClick {
                    x: state.pointer_x, y: state.pointer_y, button, serial,
                });
            }
            _ => {}
        }
    }
}


impl Dispatch<wl_touch::WlTouch, ()> for ConfigureState {
    fn event(
        state:&mut Self,_:&wl_touch::WlTouch,event:wl_touch::Event,
        _:&(),_:&Connection,_:&QueueHandle<Self>,
    ){
        match event {
            wl_touch::Event::Down{surface,id,x,y,..}=>{
                if state.active_surface.as_ref()==Some(&surface){
                    state.touch_contacts.insert(id,(x,y));
                }
            }
            wl_touch::Event::Motion{id,x,y,..}=>{
                if let Some(position)=state.touch_contacts.get_mut(&id){
                    *position=(x,y);
                }
            }
            wl_touch::Event::Up{id,serial,..}=>{
                if let Some((x,y))=state.touch_contacts.remove(&id){
                    state.clicks.push(PointerClick{x,y,button:0,serial});
                }
            }
            wl_touch::Event::Cancel=>state.touch_contacts.clear(),
            _=>{}
        }
    }
}

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
    let anchor=bounds.corner.anchors();
    layer.set_margin(
        if anchor.contains(Anchor::Top) {margin} else {0},
        if anchor.contains(Anchor::Right) {margin} else {0},
        if anchor.contains(Anchor::Bottom) {margin} else {0},
        if anchor.contains(Anchor::Left) {margin} else {0},
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


/// Experimental Wayland wl_shm frame presentation. This displays one already
/// rasterized notification for a bounded duration and then removes it.
/// Not used by the installed desktop notification service, which is still Mako.
///
/// Mako parity still needs multiple surfaces, output selection, scaling,
/// input, redraws and a continuously running Wayland dispatch loop.
pub fn present_frame_once(
    frame: &PixelFrame, corner: ScreenCorner, margin: i32, visible_for: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_frame(frame)?;
    if margin < 0 { return Err("negative margin".into()); }
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<ConfigureState>(&conn)?;
    let qh = queue.handle();
    // damage_buffer is a wl_surface v4 request. Modern Hyprland supports it.
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=6, ())?;
    let shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(&surface, None, Layer::Top, "posthorn-preview".into(), &qh, ());
    layer.set_size(frame.width, frame.height);
    layer.set_anchor(corner.anchors());
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    let anchor=corner.anchors();
    layer.set_margin(
        if anchor.contains(Anchor::Top) {margin} else {0},
        if anchor.contains(Anchor::Right) {margin} else {0},
        if anchor.contains(Anchor::Bottom) {margin} else {0},
        if anchor.contains(Anchor::Left) {margin} else {0},
    );
    surface.commit();
    let mut state = ConfigureState::default();
    while state.size.is_none() && !state.closed {
        queue.blocking_dispatch(&mut state)?;
    }
    if state.closed { return Err("layer surface closed before drawing".into()); }
    let (configured_width, configured_height) = state.size.ok_or("no initial configure")?;
    if (configured_width != 0 && configured_width != frame.width)
        || (configured_height != 0 && configured_height != frame.height) {
        return Err("compositor configured a size different from the supplied frame".into());
    }

    // Use a regular anonymous temporary file as the SHM backing store. It is
    // unlink-on-open, and its pixels are immutable for the lifetime of the
    // compositor's use of this buffer. No unsafe memory mapping is needed.
    let mut file = tempfile::tempfile()?;
    file.write_all(&frame.pixels)?;
    let pool = shm.create_pool(file.as_fd(), frame.pixels.len() as i32, &qh, ());
    let buffer = pool.create_buffer(0, frame.width as i32, frame.height as i32,
        frame.stride, wl_shm::Format::Argb8888, &qh, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, frame.width as i32, frame.height as i32);
    surface.commit();
    conn.flush()?;
    std::thread::sleep(visible_for);
    buffer.destroy();
    pool.destroy();
    layer.destroy();
    surface.destroy();
    conn.flush()?;
    Ok(())
}


/**
 * Reusable layer-shell presenter. Unlike present_frame_once, this keeps a
 * mapped surface alive until its content changes or notifications disappear.
 * It is intentionally confined to the opt-in development renderer.
 */
pub struct LivePresenter {
    conn: Connection,
    queue: wayland_client::EventQueue<ConfigureState>,
    qh: QueueHandle<ConfigureState>,
    compositor: wl_compositor::WlCompositor,
    shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    shm: wl_shm::WlShm,
    state: ConfigureState,
    _seat: Option<wl_seat::WlSeat>,
    active: Option<LiveSurface>,
}

struct LiveSurface {
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    surface: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    // Retain backing storage until no compositor buffer references it.
    _backing: std::fs::File,
}

impl LivePresenter {
    pub fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init::<ConfigureState>(&conn)?;
        let qh = queue.handle();
        let compositor = globals.bind(&qh, 4..=6, ())?;
        let shell = globals.bind(&qh, 1..=4, ())?;
        let shm = globals.bind(&qh, 1..=1, ())?;
        // Headless preview still works without any pointer-capable seat.
        let seat = globals.bind(&qh, 1..=9, ()).ok();
        Ok(Self {
            conn, queue, qh, compositor, shell, shm,
            state: ConfigureState::default(), _seat: seat, active: None,
        })
    }

    pub fn update_frame(
        &mut self, frame: Option<&PixelFrame>, corner: ScreenCorner, margin: i32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.update_frame_with_layer(frame,corner,margin,PopupLayer::Top)
    }

    pub fn update_frame_with_layer(
        &mut self, frame: Option<&PixelFrame>, corner: ScreenCorner, margin: i32,
        selected_layer: PopupLayer,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.update_frame_with_options(frame,corner,PopupMargins::uniform(margin),selected_layer)
    }

    pub fn update_frame_with_options(
        &mut self, frame: Option<&PixelFrame>, corner: ScreenCorner,
        margins: PopupMargins, selected_layer: PopupLayer,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(frame) = frame { validate_frame(frame)?; }
        self.clear();
        let Some(frame) = frame else {
            self.conn.flush()?;
            return Ok(());
        };
        // Preserve registered seat and pointer objects across redraws.
        self.state.size = None;
        self.state.closed = false;
        self.state.pointer_inside = false;
        self.state.touch_contacts.clear();
        self.state.clicks.clear();
        let surface = self.compositor.create_surface(&self.qh, ());
        self.state.active_surface=Some(surface.clone());
        let layer = self.shell.get_layer_surface(
            &surface, None, selected_layer.protocol(), "posthorn-private-preview".into(), &self.qh, (),
        );
        layer.set_anchor(corner.anchors());
        layer.set_size(frame.width, frame.height);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        let anchor=corner.anchors();
        layer.set_margin(
            if anchor.contains(Anchor::Top) {margins.top} else {0},
            if anchor.contains(Anchor::Right) {margins.right} else {0},
            if anchor.contains(Anchor::Bottom) {margins.bottom} else {0},
            if anchor.contains(Anchor::Left) {margins.left} else {0},
        );
        surface.commit();
        while self.state.size.is_none() && !self.state.closed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        if self.state.closed {return Err("layer-shell compositor closed the popup".into());}
        let (cw,ch) = self.state.size.ok_or("no layer-shell configure")?;
        if (cw != 0 && cw != frame.width) || (ch != 0 && ch != frame.height) {
            return Err("compositor supplied an unexpected popup size".into());
        }
        let mut backing = tempfile::tempfile()?;
        backing.write_all(&frame.pixels)?;
        let pool = self.shm.create_pool(
            backing.as_fd(), frame.pixels.len() as i32, &self.qh, (),
        );
        let buffer = pool.create_buffer(
            0, frame.width as i32, frame.height as i32, frame.stride,
            wl_shm::Format::Argb8888, &self.qh, (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, frame.width as i32, frame.height as i32);
        surface.commit();
        self.conn.flush()?;
        self.active = Some(LiveSurface { layer, surface, pool, buffer, _backing: backing });
        Ok(())
    }

    pub fn poll_events(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // dispatch_pending() alone does NOT read compositor socket events.
        // First dispatch existing events, then poll the Wayland fd without
        // blocking the runtime's notification refresh loop.
        self.queue.dispatch_pending(&mut self.state)?;
        self.conn.flush()?;
        if let Some(read) = self.conn.prepare_read() {
            let ready = {
                let mut fd = [PollFd::new(self.conn.as_fd(), PollFlags::POLLIN)];
                poll(&mut fd, 0u8)? > 0
                    && fd[0].revents().is_some_and(|events| events.contains(PollFlags::POLLIN))
            };
            if ready { read.read()?; }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        if self.state.closed {
            self.clear();
            self.state.closed = false;
        }
        Ok(())
    }

    pub fn take_clicks(&mut self) -> Vec<PointerClick> {
        std::mem::take(&mut self.state.clicks)
    }

    pub fn clear(&mut self) {
        self.state.pointer_inside = false;
        self.state.active_surface=None;
        self.state.touch_contacts.clear();
        self.state.clicks.clear();
        if let Some(old) = self.active.take() {
            old.buffer.destroy();
            old.pool.destroy();
            old.layer.destroy();
            old.surface.destroy();
        }
    }
}

impl Drop for LivePresenter {
    fn drop(&mut self) {
        self.clear();
        let _ = self.conn.flush();
    }
}

fn validate_frame(frame: &PixelFrame) -> Result<(), &'static str> {
    if frame.width == 0 || frame.height == 0 { return Err("empty frame"); }
    let expected_stride = i64::from(frame.width) * 4;
    let len = i64::from(frame.stride) * i64::from(frame.height);
    if i64::from(frame.stride) < expected_stride || len <= 0 || len > i64::from(i32::MAX)
        || usize::try_from(len).ok() != Some(frame.pixels.len()) {
        return Err("invalid Wayland pixel buffer size or stride");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_contact_translates_into_the_same_event_envelope() {
        let mut state=ConfigureState::default();
        state.touch_contacts.insert(3,(20.0,15.0));
        let (x,y)=state.touch_contacts.remove(&3).unwrap();
        state.clicks.push(PointerClick{x,y,button:0,serial:11});
        assert_eq!(state.clicks.remove(0).button,0);
        assert!(state.touch_contacts.is_empty());
    }

    #[test]
    fn pointer_click_is_a_explicitly_typed_event() {
        let mut state = ConfigureState::default();
        state.pointer_inside = true;
        state.pointer_x = 32.5;
        state.pointer_y = 40.0;
        state.clicks.push(PointerClick {
            x: state.pointer_x, y: state.pointer_y, button: 272, serial: 5,
        });
        assert_eq!(state.clicks.remove(0).button, 272);
    }

    #[test]
    fn refuses_invalid_shm_lengths_without_connecting_to_wayland() {
        let frame = PixelFrame { width: 4, height: 4, stride: 16, pixels: vec![0; 64] };
        assert!(validate_frame(&frame).is_ok());
        assert!(validate_frame(&PixelFrame { pixels: vec![0; 60], ..frame.clone() }).is_err());
        assert!(validate_frame(&PixelFrame { stride: 4, ..frame }).is_err());
    }

    #[test]
    fn directional_margins_match_mako_one_to_four_value_parsing() {
        assert_eq!(PopupMargins::parse_mako("10"),Some(PopupMargins::uniform(10)));
        assert_eq!(PopupMargins::parse_mako("10,20"),
            Some(PopupMargins{top:10,right:20,bottom:10,left:20}));
        assert_eq!(PopupMargins::parse_mako("10,20,30"),
            Some(PopupMargins{top:10,right:20,bottom:30,left:20}));
        assert_eq!(PopupMargins::parse_mako("1,2,3,4"),
            Some(PopupMargins{top:1,right:2,bottom:3,left:4}));
        assert_eq!(PopupMargins::parse_mako("-2"),
            Some(PopupMargins::uniform(-2)));
        assert!(PopupMargins::parse_mako("1,2,3,4,5").is_none());
    }

    #[test]
    fn all_mako_layer_names_translate_without_ambiguity() {
        assert_eq!(PopupLayer::from_mako("top"),Some(PopupLayer::Top));
        assert_eq!(PopupLayer::from_mako("overlay"),Some(PopupLayer::Overlay));
        assert_eq!(PopupLayer::from_mako("bottom"),Some(PopupLayer::Bottom));
        assert_eq!(PopupLayer::from_mako("background"),Some(PopupLayer::Background));
        assert_eq!(PopupLayer::from_mako("other"),None);
    }

    #[test]
    fn all_nine_mako_anchors_map_to_wayland_layer_shell_flags() {
        for option in ["top-left","top-center","top-right",
            "center-left","center","center-right",
            "bottom-left","bottom-center","bottom-right"]{
            assert!(ScreenCorner::from_mako(option).is_some());
        }
        assert_eq!(ScreenCorner::from_mako("center").unwrap().anchors(),Anchor::empty());
        assert_eq!(ScreenCorner::from_mako("bottom-center").unwrap().anchors(),Anchor::Bottom);
        assert!(ScreenCorner::from_mako("invalid").is_none());
    }

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
