//! Isolated headless compositor input fixture. Never connects to an
//! existing desktop intentionally: the caller must set its private
//! WAYLAND_DISPLAY and XDG_RUNTIME_DIR before invoking this example.

use std::error::Error;
use std::time::Duration;
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init,GlobalListContents},
    protocol::{wl_registry,wl_seat,wl_pointer},
    Connection,Dispatch,QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1,zwlr_virtual_pointer_v1,
};

#[derive(Default)]
struct FixtureState;

impl Dispatch<wl_registry::WlRegistry,GlobalListContents> for FixtureState {
    fn event(
        _: &mut Self, _: &wl_registry::WlRegistry,
        _: wl_registry::Event, _: &GlobalListContents,
        _: &Connection, _: &QueueHandle<Self>,
    ) {}
}
delegate_noop!(FixtureState: ignore wl_seat::WlSeat);
delegate_noop!(FixtureState: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(FixtureState: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

fn main()->Result<(),Box<dyn Error>> {
    // Use the same logical coordinate space as the configured headless output.
    let mut args=std::env::args().skip(1);
    let x:u32=args.next().ok_or("missing X")?.parse()?;
    let y:u32=args.next().ok_or("missing Y")?.parse()?;
    let width:u32=args.next().ok_or("missing output width")?.parse()?;
    let height:u32=args.next().ok_or("missing output height")?.parse()?;
    if args.next().is_some() || width==0 || height==0 || x>=width || y>=height {
        return Err("expected: X Y OUTPUT_WIDTH OUTPUT_HEIGHT, coordinates within output".into());
    }

    let connection=Connection::connect_to_env()?;
    let (globals,mut queue)=registry_queue_init::<FixtureState>(&connection)?;
    let qh=queue.handle();
    let manager:zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1=
        globals.bind(&qh,1..=2,())?;
    let seat:wl_seat::WlSeat=globals.bind(&qh,1..=9,())?;
    let pointer=manager.create_virtual_pointer(Some(&seat),&qh,());
    connection.flush()?;
    // Give Sway time to advertise wl_seat.pointer capability to Posthorn.
    queue.roundtrip(&mut FixtureState)?;
    std::thread::sleep(Duration::from_millis(350));

    pointer.motion_absolute(1,x,y,width,height);
    pointer.frame();
    connection.flush()?;
    std::thread::sleep(Duration::from_millis(200));
    pointer.button(2,273,wl_pointer::ButtonState::Pressed);
    pointer.frame();
    connection.flush()?;
    std::thread::sleep(Duration::from_millis(100));
    pointer.button(3,273,wl_pointer::ButtonState::Released);
    pointer.frame();
    connection.flush()?;
    std::thread::sleep(Duration::from_millis(100));
    pointer.destroy();
    manager.destroy();
    connection.flush()?;
    Ok(())
}
