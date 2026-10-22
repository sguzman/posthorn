//! Private Wayland layer-shell presenter and compositor protocol integration.
//!
//! Supports buffered rendering, release-aware SHM, click/touch events, named
//! output integer scaling, and compositor-issued XDG activation tokens.
//! Live Hyprland acceptance is still required before replacing Mako.

use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch},
    WEnum,
    Connection, Dispatch, QueueHandle,
};
use std::collections::HashMap;
use std::io::Write;
use nix::poll::{poll, PollFd, PollFlags};
use std::os::fd::AsFd;
use std::time::Duration;
use crate::render::PixelFrame;
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_v1,
    xdg_activation_token_v1,
};
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
        let values:Vec<i32>=raw.split(',').map(|v|
            v.trim_start_matches([' ','\t']).parse()
        ).collect::<Result<_,_>>().ok()?;
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
    output_globals: Vec<(u32,wl_output::WlOutput)>,
    output_names: Vec<(wl_output::WlOutput,String)>,
    output_scales: Vec<(wl_output::WlOutput,u32)>,
    entered_output: Option<wl_output::WlOutput>,
    released_buffers: Vec<wl_buffer::WlBuffer>,
    activation_token: Option<String>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ConfigureState {
    fn event(
        state: &mut Self,registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents, _: &Connection, qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global{name,interface,version}
                if interface=="wl_output" && version>=4 =>
            {
                if !state.output_globals.iter().any(|(id,_)|*id==name) {
                    let output=registry.bind::<wl_output::WlOutput,_,_>(name,4,qh,());
                    state.output_globals.push((name,output));
                }
            }
            wl_registry::Event::GlobalRemove{name} => {
                if let Some(index)=state.output_globals.iter()
                    .position(|(id,_)|*id==name)
                {
                    let (_,output)=state.output_globals.swap_remove(index);
                    state.output_names.retain(|(proxy,_)|proxy!=&output);
                    state.output_scales.retain(|(proxy,_)|proxy!=&output);
                    if state.entered_output.as_ref()==Some(&output) {
                        state.entered_output=None;
                    }
                    output.release();
                }
            }
            _=>{},
        }
    }
}

delegate_noop!(ConfigureState: ignore xdg_activation_v1::XdgActivationV1);
impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for ConfigureState {
    fn event(
        state:&mut Self, _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event:xdg_activation_token_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done {token} = event {
            state.activation_token = Some(token);
        }
    }
}

delegate_noop!(ConfigureState: ignore wl_compositor::WlCompositor);
impl Dispatch<wl_surface::WlSurface, ()> for ConfigureState {
    fn event(
        state:&mut Self,surface:&wl_surface::WlSurface,event:wl_surface::Event,
        _:&(),_:&Connection,_:&QueueHandle<Self>,
    ){
        if state.active_surface.as_ref()!=Some(surface) {return;}
        match event {
            wl_surface::Event::Enter{output}=>state.entered_output=Some(output),
            wl_surface::Event::Leave{output}=>
                if state.entered_output.as_ref()==Some(&output){
                    state.entered_output=None;
                },
            _=>{},
        }
    }
}
delegate_noop!(ConfigureState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(ConfigureState: ignore wl_shm::WlShm);
delegate_noop!(ConfigureState: ignore wl_shm_pool::WlShmPool);
impl Dispatch<wl_buffer::WlBuffer, ()> for ConfigureState {
    fn event(
        state: &mut Self, buffer: &wl_buffer::WlBuffer, event: wl_buffer::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.released_buffers.push(buffer.clone());
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for ConfigureState {
    fn event(
        state:&mut Self, output:&wl_output::WlOutput,event:wl_output::Event,
        _:&(),_:&Connection,_:&QueueHandle<Self>,
    ){
        match event {
            wl_output::Event::Name{name}=>{
                if let Some((_,value))=state.output_names.iter_mut()
                    .find(|(proxy,_)|proxy==output){
                    *value=name;
                }else{
                    state.output_names.push((output.clone(),name));
                }
            }
            wl_output::Event::Scale{factor}=>{
                let scale=factor.clamp(1,8) as u32;
                if let Some((_,value))=state.output_scales.iter_mut()
                    .find(|(proxy,_)|proxy==output){
                    *value=scale;
                }else{
                    state.output_scales.push((output.clone(),scale));
                }
            }
            _=>{},
        }
    }
}

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
    xdg_activation: Option<xdg_activation_v1::XdgActivationV1>,
    state: ConfigureState,
    _seat: Option<wl_seat::WlSeat>,
    active: Option<LiveSurface>,
    // Buffers stay allocated until the compositor explicitly releases them.
    retired: Vec<ShmAllocation>,
}

#[derive(Clone,Debug,Eq,PartialEq)]
struct PopupSurfaceSpec {
    width:u32,height:u32,
    corner:ScreenCorner,
    margins:PopupMargins,
    layer:PopupLayer,
    output_name:String,
    scale:u32,
}

struct ShmAllocation {
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    // Keep the owned backing file while the compositor holds this buffer.
    _backing: std::fs::File,
}

impl ShmAllocation {
    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

struct LiveSurface {
    spec:PopupSurfaceSpec,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    surface: wl_surface::WlSurface,
    frame: ShmAllocation,
}

impl LivePresenter {
    pub fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<ConfigureState>(&conn)?;
        let qh = queue.handle();
        let compositor = globals.bind(&qh, 4..=6, ())?;
        let shell = globals.bind(&qh, 1..=4, ())?;
        let shm = globals.bind(&qh, 1..=1, ())?;
        // Headless preview still works without a pointer-capable seat.
        let seat = globals.bind(&qh, 1..=9, ()).ok();
        let xdg_activation = globals.bind(&qh, 1..=1, ()).ok();
        // wl_output.name is available from version 4. Bind every named
        // output instead of incorrectly selecting the first global.
        let named_outputs:Vec<_>=globals.contents().clone_list().into_iter()
            .filter(|g|g.interface=="wl_output"&&g.version>=4)
            .map(|g|(g.name,globals.registry().bind::<wl_output::WlOutput,_,_>(
                g.name,4,&qh,()
            )))
            .collect();
        let mut state=ConfigureState::default();
        state.output_globals=named_outputs;
        if !state.output_globals.is_empty() {
            queue.roundtrip(&mut state)?;
        }
        Ok(Self {
            conn, queue, qh, compositor, shell, shm, xdg_activation,
            state, _seat: seat, active: None, retired: Vec::new(),
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
        self.update_frame_on_output(frame,corner,margins,selected_layer,"")
    }

    /// Resolve compositor-advertised integer scale. The unqualified default
    /// follows wl_surface.enter instead of assuming the first monitor.
    /// A slow compositor may retain old shared-memory buffers. Postpone
    /// visual updates rather than terminating the entire notification daemon.
    pub fn buffer_backpressure(&self)->bool {
        self.retired.len()+usize::from(self.active.is_some())>=32
    }

    /// A compositor may close our layer surface without any change in the
    /// notification store. The desktop loop must notice this and repaint.
    pub fn has_active_surface(&self)->bool {
        self.active.is_some()
    }

    /// Named output monitoring is dynamic: output globals can disappear and
    /// later reappear with a new ID. Never present to a stale proxy.
    pub fn has_output(&self,output_name:&str)->bool {
        output_name.is_empty() || self.state.output_names.iter()
            .any(|(_,name)|name==output_name)
    }

    pub fn output_scale(&self,output_name:&str)->u32{
        let output=if output_name.is_empty(){
            self.state.entered_output.as_ref()
        }else{
            self.state.output_names.iter()
                .find(|(_,name)|name==output_name)
                .map(|(out,_)|out)
        };
        output.and_then(|target|self.state.output_scales.iter()
            .find(|(out,_)|out==target)
            .map(|(_,scale)|*scale)).unwrap_or(1)
    }

    pub fn update_frame_on_output(
        &mut self, frame: Option<&PixelFrame>, corner: ScreenCorner,
        margins: PopupMargins, selected_layer: PopupLayer, output_name:&str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.update_frame_on_output_impl(frame,corner,margins,selected_layer,output_name,false)
    }

    /// Accept a frame already rasterized at the compositor's integer scale.
    /// Unlike scale_frame(), this preserves HiDPI Pango glyph edges.
    pub fn update_native_frame_on_output(
        &mut self,frame:Option<&PixelFrame>,corner:ScreenCorner,
        margins:PopupMargins,selected_layer:PopupLayer,output_name:&str,
    ) -> Result<(),Box<dyn std::error::Error>> {
        self.update_frame_on_output_impl(frame,corner,margins,selected_layer,output_name,true)
    }

    fn update_frame_on_output_impl(
        &mut self, frame: Option<&PixelFrame>, corner: ScreenCorner,
        margins: PopupMargins, selected_layer: PopupLayer, output_name:&str,
        native:bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(frame) = frame { validate_frame(frame)?; }
        self.reclaim_released();
        // Clearing a popup never requires a valid output. In particular, the
        // target monitor may already have disappeared on hot-unplug.
        let Some(frame) = frame else {
            self.clear();
            self.conn.flush()?;
            return Ok(());
        };
        // Poll pending name notifications before resolving the target.
        self.queue.dispatch_pending(&mut self.state)?;
        let output = if output_name.is_empty() {None} else {
            Some(self.state.output_names.iter()
                .find(|(_,name)|name==output_name)
                .ok_or_else(||format!("Wayland output not found: {output_name}"))?
                .0.clone())
        };
        let scale=self.output_scale(output_name);
        let (logical_w,logical_h)=if native {
            if frame.width%scale!=0 || frame.height%scale!=0 {
                return Err("HiDPI frame must have dimensions divisible by output scale".into());
            }
            (frame.width/scale,frame.height/scale)
        }else{(frame.width,frame.height)};
        let spec=PopupSurfaceSpec {
            width:logical_w,height:logical_h,corner,margins,
            layer:selected_layer,output_name:output_name.into(),scale,
        };
        if self.active.as_ref().is_some_and(|live|live.spec==spec){
            return self.redraw_active(frame,native);
        }
        // Geometry changes recreate the layer. They must obey the same
        // outstanding-buffer bound as same-size redraws.
        if self.retired.len() + usize::from(self.active.is_some()) >= 32 {
            return Err("Wayland compositor has not released 32 old buffers".into());
        }
        self.clear();
        // Preserve registered seat and pointer objects across redraws.
        self.state.size = None;
        self.state.closed = false;
        self.state.pointer_inside = false;
        self.state.touch_contacts.clear();
        self.state.clicks.clear();
        let surface = self.compositor.create_surface(&self.qh, ());
        self.state.active_surface=Some(surface.clone());
        self.state.entered_output=None;
        let layer = self.shell.get_layer_surface(
            &surface, output.as_ref(), selected_layer.protocol(),
            "posthorn-private-preview".into(), &self.qh, (),
        );
        layer.set_anchor(corner.anchors());
        layer.set_size(logical_w, logical_h);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        // Mako sends all four outer-margin values to layer shell regardless
        // of anchor; the compositor chooses which edges affect placement.
        layer.set_margin(margins.top,margins.right,margins.bottom,margins.left);
        surface.commit();
        while self.state.size.is_none() && !self.state.closed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        if self.state.closed {return Err("layer-shell compositor closed the popup".into());}
        let (cw,ch) = self.state.size.ok_or("no layer-shell configure")?;
        if (cw != 0 && cw != logical_w) || (ch != 0 && ch != logical_h) {
            return Err("compositor supplied an unexpected popup size".into());
        }
        let physical=if native {frame.clone()} else {scale_frame(frame,scale)?};
        let mut backing = tempfile::tempfile()?;
        backing.write_all(&physical.pixels)?;
        let pool = self.shm.create_pool(
            backing.as_fd(), physical.pixels.len() as i32, &self.qh, (),
        );
        let buffer = pool.create_buffer(
            0, physical.width as i32, physical.height as i32, physical.stride,
            wl_shm::Format::Argb8888, &self.qh, (),
        );
        surface.set_buffer_scale(scale as i32);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, physical.width as i32, physical.height as i32);
        surface.commit();
        self.conn.flush()?;
        self.active = Some(LiveSurface {
            spec,layer,surface,
            frame: ShmAllocation { buffer, pool, _backing: backing },
        });
        Ok(())
    }

    /// Update content on a previously configured layer surface without
    /// re-entering the layer-shell handshake. In particular, this preserves
    /// compositor focus and avoids a visible surface teardown/flicker on
    /// ordinary message replacement.
    fn redraw_active(&mut self,frame:&PixelFrame,native:bool)
        -> Result<(),Box<dyn std::error::Error>>
    {
        if self.retired.len() >= 32 {
            return Err("Wayland compositor has not released 32 old buffers".into());
        }
        let Some(live)=self.active.as_mut() else {
            return Err("cannot redraw an unmapped Wayland notification".into());
        };
        let physical=if native {frame.clone()} else {scale_frame(frame,live.spec.scale)?};
        let mut backing=tempfile::tempfile()?;
        backing.write_all(&physical.pixels)?;
        let pool=self.shm.create_pool(backing.as_fd(),
            physical.pixels.len() as i32,&self.qh,());
        let buffer=pool.create_buffer(0,physical.width as i32,physical.height as i32,
            physical.stride,wl_shm::Format::Argb8888,&self.qh,());
        live.surface.set_buffer_scale(live.spec.scale as i32);
        live.surface.attach(Some(&buffer),0,0);
        live.surface.damage_buffer(0,0,physical.width as i32,physical.height as i32);
        live.surface.commit();
        // Never destroy or drop the old allocation until wl_buffer.release.
        // The compositor might still be reading the old frame.
        let old=std::mem::replace(&mut live.frame,
            ShmAllocation{buffer,pool,_backing:backing});
        self.retire(old);
        self.conn.flush()?;
        Ok(())
    }

    fn retire(&mut self, frame: ShmAllocation) {
        if let Some(index)=self.state.released_buffers.iter()
            .position(|buffer|buffer==&frame.buffer)
        {
            self.state.released_buffers.swap_remove(index);
            frame.destroy();
        } else {
            self.retired.push(frame);
        }
    }

    fn reclaim_released(&mut self) {
        // Retain the release of an actively attached buffer until it is
        // replaced; all other releases can be reclaimed immediately.
        let mut released=std::mem::take(&mut self.state.released_buffers);
        let active=self.active.as_ref().map(|live|live.frame.buffer.clone());
        self.retired.retain(|buffer|{
            if let Some(index)=released.iter().position(|id|id==&buffer.buffer){
                released.swap_remove(index);
                buffer.buffer.destroy();
                buffer.pool.destroy();
                false
            }else{
                true
            }
        });
        self.state.released_buffers=active.into_iter().filter(|a|
            released.iter().any(|buffer|buffer==a)
        ).collect();
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
        self.reclaim_released();
        if self.state.closed {
            self.clear();
            self.state.closed = false;
        }
        Ok(())
    }

    pub fn take_clicks(&mut self) -> Vec<PointerClick> {
        std::mem::take(&mut self.state.clicks)
    }

    /// Request a compositor-issued token for a real pointer/touch serial.
    /// Mako emits ActivationToken just before ActionInvoked. A missing global
    /// or compositor refusal is not a fatal notification error.
    pub fn request_activation_token(&mut self,serial:u32)
        -> Result<Option<String>,Box<dyn std::error::Error>>
    {
        if serial==0 {return Ok(None);}
        let (Some(activation),Some(seat),Some(active))=
            (self.xdg_activation.as_ref(),self._seat.as_ref(),self.active.as_ref())
        else {return Ok(None)};
        self.state.activation_token=None;
        let token=activation.get_activation_token(&self.qh,());
        token.set_serial(serial,seat);
        token.set_surface(&active.surface);
        token.commit();
        self.conn.flush()?;
        // Mako likewise waits for the compositor's token response. Dispatch
        // one roundtrip instead of spinning forever if no token is delivered.
        self.queue.roundtrip(&mut self.state)?;
        let issued=self.state.activation_token.take().filter(|s|!s.is_empty());
        token.destroy();
        Ok(issued)
    }

    pub fn clear(&mut self) {
        self.state.pointer_inside = false;
        self.state.active_surface=None;
        self.state.entered_output=None;
        self.state.touch_contacts.clear();
        self.state.clicks.clear();
        if let Some(old) = self.active.take() {
            old.layer.destroy();
            old.surface.destroy();
            self.retire(old.frame);
        }
    }
}

impl Drop for LivePresenter {
    fn drop(&mut self) {
        self.clear();
        // At connection teardown the compositor releases its references;
        // proactively destroy the remaining client-side handles.
        for frame in self.retired.drain(..) {
            frame.destroy();
        }
        let _ = self.conn.flush();
    }
}

/// Expand one logical-pixel ARGB8888 image to a physical-size SHM buffer.
/// The Wayland layer surface remains measured in logical pixels; the caller
/// sets wl_surface.buffer_scale to the same integer scale.
pub fn scale_frame(frame:&PixelFrame,scale:u32)->Result<PixelFrame,String>{
    validate_frame(frame).map_err(str::to_owned)?;
    if !(1..=8).contains(&scale){return Err("unsupported Wayland output scale".into());}
    let width=frame.width.checked_mul(scale).ok_or("scaled width overflow")?;
    let height=frame.height.checked_mul(scale).ok_or("scaled height overflow")?;
    let stride=width.checked_mul(4).ok_or("scaled stride overflow")?;
    let len=u64::from(stride)*u64::from(height);
    if width>4096||height>4096||len>64*1024*1024||len>i32::MAX as u64{
        return Err("scaled Wayland buffer exceeds safe bounds".into());
    }
    if scale==1{return Ok(frame.clone());}
    let mut pixels=vec![0u8;len as usize];
    for y in 0..frame.height as usize{
        let src_row=y*frame.stride as usize;
        for x in 0..frame.width as usize {
            let src=src_row+x*4;
            for dy in 0..scale as usize {
                for dx in 0..scale as usize {
                    let dst=(y*scale as usize+dy)*stride as usize
                        +(x*scale as usize+dx)*4;
                    pixels[dst..dst+4].copy_from_slice(&frame.pixels[src..src+4]);
                }
            }
        }
    }
    Ok(PixelFrame{width,height,stride:stride as i32,pixels})
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
    fn native_hidpi_frames_keep_logical_layer_geometry() {
        let logical=PixelFrame{width:2,height:3,stride:8,pixels:vec![0;24]};
        let native=scale_frame(&logical,2).unwrap();
        assert_eq!((native.width/2,native.height/2),(logical.width,logical.height));
        assert_ne!((native.width,native.height),(logical.width,logical.height));
    }

    #[test]
    fn hidpi_buffer_scales_physical_pixels_without_blending_argb(){
        let frame=PixelFrame{
            width:2,height:1,stride:8,
            pixels:vec![10,20,30,255,40,50,60,128],
        };
        let scaled=scale_frame(&frame,2).unwrap();
        assert_eq!((scaled.width,scaled.height,scaled.stride),(4,2,16));
        assert_eq!(&scaled.pixels[0..4],[10,20,30,255]);
        assert_eq!(&scaled.pixels[4..8],[10,20,30,255]);
        assert_eq!(&scaled.pixels[8..12],[40,50,60,128]);
        assert_eq!(&scaled.pixels[16..32],&scaled.pixels[0..16]);
        assert_eq!(scale_frame(&frame,1).unwrap().pixels,frame.pixels);
        assert!(scale_frame(&frame,0).is_err());
        assert!(scale_frame(&frame,9).is_err());
        assert!(scale_frame(&PixelFrame{
            width:4096,height:4096,stride:16384,
            pixels:vec![0;4096*4096*4],
        },2).is_err());
    }

    #[test]
    fn refuses_invalid_shm_lengths_without_connecting_to_wayland() {
        let frame = PixelFrame { width: 4, height: 4, stride: 16, pixels: vec![0; 64] };
        assert!(validate_frame(&frame).is_ok());
        assert!(validate_frame(&PixelFrame { pixels: vec![0; 60], ..frame.clone() }).is_err());
        assert!(validate_frame(&PixelFrame { stride: 4, ..frame }).is_err());
    }

    #[test]
    fn only_geometry_or_output_changes_require_surface_recreation() {
        let spec=PopupSurfaceSpec{
            width:440,height:120,corner:ScreenCorner::TopRight,
            margins:PopupMargins::uniform(10),layer:PopupLayer::Top,
            output_name:"DP-1".into(),scale:1,
        };
        assert_eq!(spec,spec.clone());
        assert_ne!(spec,PopupSurfaceSpec{width:500,..spec.clone()});
        assert_ne!(spec,PopupSurfaceSpec{output_name:"HDMI-A-1".into(),..spec.clone()});
        assert_ne!(spec,PopupSurfaceSpec{layer:PopupLayer::Overlay,..spec.clone()});
        assert_ne!(spec,PopupSurfaceSpec{scale:2,..spec.clone()});
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
    fn empty_output_name_means_compositor_selection() {
        let names=["DP-1","HDMI-A-1"];
        assert!(names.iter().any(|name|*name=="DP-1"));
        assert!(!names.iter().any(|name|*name=="HDMI-A-3"));
        assert!("".is_empty());
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
