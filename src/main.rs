use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{mpsc::sync_channel, Arc},
    thread,
};

use parking_lot::Mutex;

use gameroy::{
    cartridge::Cartridge,
    gameboy::{self, GameBoy},
    parser::Vbm,
};

mod disassembler_viewer;
mod emulator;
mod event_table;
mod fold_view;
mod layout;
mod split_view;
mod style;
mod ui;

pub use emulator::{Emulator, EmulatorEvent};

#[macro_use]
extern crate crui;

const SCREEN_WIDTH: usize = 160;
const SCREEN_HEIGHT: usize = 144;

fn main() {
    env_logger::init();

    let mut diss = false;
    let mut debug = false;
    let mut rom_path = "roms/test.gb".to_string();
    let mut movie = None;

    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-d" | "--disassembly" => diss = true,
            "-b" | "--debug" => debug = true,
            "-m" | "--movie" => {
                let path = args.next().expect("expected path to the movie");
                let mut file = std::fs::File::open(path).unwrap();
                let vbm = gameroy::parser::vbm(&mut file).unwrap();
                movie = Some(vbm);
            }
            _ if arg.starts_with("-") => {
                eprintln!("unknown argument {}", arg);
                return;
            }
            _ => {
                rom_path = arg;
            }
        }
    }
    let rom_path = PathBuf::from(rom_path);

    let rom = std::fs::read(&rom_path).unwrap();

    let mut boot_rom_file = File::open("bootrom/dmg_boot.bin").unwrap();
    let mut boot_rom = [0; 0x100];
    boot_rom_file.read(&mut boot_rom).unwrap();

    let mut cartridge = Cartridge::new(rom).unwrap();
    let mut save_path = rom_path.clone();
    if save_path.set_extension("sav") {
        println!("loading save at {}", save_path.display());
        let saved_ram = std::fs::read(&save_path);
        match saved_ram {
            Ok(save) => *cartridge.ram_mut() = save,
            Err(err) => {
                println!("load save failed: {}", err);
            }
        }
    }
    let mut game_boy = gameboy::GameBoy::new(boot_rom, cartridge);

    {
        let mut trace = game_boy.trace.borrow_mut();

        trace.trace_starting_at(&game_boy, 0, 0x100, Some("entry point".into()));
        trace.trace_starting_at(&game_boy, 0, 0x40, Some("RST_0x40".into()));
        trace.trace_starting_at(&game_boy, 0, 0x48, Some("RST_0x48".into()));
        trace.trace_starting_at(&game_boy, 0, 0x50, Some("RST_0x50".into()));
        trace.trace_starting_at(&game_boy, 0, 0x58, Some("RST_0x58".into()));
        trace.trace_starting_at(&game_boy, 0, 0x60, Some("RST_0x60".into()));

        if diss {
            game_boy.boot_rom_active = false;

            let mut string = String::new();
            trace.fmt(&game_boy, &mut string).unwrap();
            println!("{}", string);

            return;
        }
    }

    create_window(game_boy, movie, rom_path, save_path, debug);
}

use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    window::WindowBuilder,
};

use self::emulator::Breakpoints;

struct AppState {
    /// The current state of the joypad. It is a bitmask, where 0 means pressed, and 1 released.
    pub joypad: u8,
    /// If the emulation is in debug mode.
    pub debug: bool,
}
impl AppState {
    fn new(debug: bool) -> Self {
        Self {
            debug,
            joypad: 0xFF,
        }
    }
}

fn create_window(
    mut gb: GameBoy,
    movie: Option<Vbm>,
    rom_path: PathBuf,
    save_path: PathBuf,
    debug: bool,
) {
    // create winit's window and event_loop
    let event_loop = EventLoop::with_user_event();
    let wb = WindowBuilder::new().with_inner_size(PhysicalSize::new(600, 400));

    let (mut ui, window) = ui::Ui::new(wb, &event_loop);

    let ppu_screen: Arc<Mutex<Vec<u8>>> =
        Arc::new(Mutex::new(vec![0; SCREEN_WIDTH * SCREEN_HEIGHT]));
    let ppu_screen_clone = ppu_screen.clone();
    let proxy = event_loop.create_proxy();
    gb.v_blank = Some(Box::new(move |gb| {
        {
            let img_data = &mut ppu_screen_clone.lock();
            img_data.copy_from_slice(&gb.ppu.screen);
        }
        let _ = proxy.send_event(UserEvent::FrameUpdated);
    }));

    let inter = Arc::new(Mutex::new(gb));
    let proxy = event_loop.create_proxy();

    let (emu_channel, recv) = sync_channel(3);
    if debug {
        proxy.send_event(UserEvent::Debug(debug)).unwrap();
    }
    emu_channel.send(EmulatorEvent::RunFrame).unwrap();

    let breakpoints = Arc::new(Mutex::new(Breakpoints::default()));

    ui.set::<Arc<Mutex<GameBoy>>>(inter.clone());
    ui.set::<Arc<Mutex<Breakpoints>>>(breakpoints.clone());
    ui.set(emu_channel.clone());
    ui.set::<EventLoopProxy<UserEvent>>(proxy.clone());
    ui.set(AppState::new(debug));

    let mut emu_thread = Some(
        thread::Builder::new()
            .name("emulator".to_string())
            .spawn(move || {
                Emulator::run(inter, breakpoints, recv, proxy, movie, rom_path, save_path)
            })
            .unwrap(),
    );

    // winit event loop
    event_loop.run(move |event, _, control| {
        match event {
            Event::NewEvents(_) => {
                ui.new_events(control, &window);
            }
            Event::LoopDestroyed => {
                emu_channel.send(EmulatorEvent::Kill).unwrap();
                emu_thread.take().unwrap().join().unwrap();
            }
            Event::WindowEvent {
                event, window_id, ..
            } => {
                ui.window_event(&event, &window);
                match event {
                    WindowEvent::CloseRequested => {
                        *control = ControlFlow::Exit;
                    }
                    WindowEvent::Resized(size) => {
                        ui.resize(size, window_id);
                    }
                    _ => {}
                }
            }
            Event::UserEvent(event) => {
                use UserEvent::*;
                match event {
                    FrameUpdated => {
                        let screen: &[u8] = &{
                            let lock = ppu_screen.lock();
                            lock.clone()
                        };
                        let mut img_data = vec![255; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
                        for y in 0..SCREEN_HEIGHT {
                            for x in 0..SCREEN_WIDTH {
                                let i = (x + y * SCREEN_WIDTH) as usize * 4;
                                let c = screen[i / 4];
                                const COLOR: [[u8; 3]; 4] =
                                    [[255, 255, 255], [170, 170, 170], [85, 85, 85], [0, 0, 0]];
                                img_data[i..i + 3].copy_from_slice(&COLOR[c as usize]);
                            }
                        }
                        ui.frame_update(&img_data);
                        ui.notify(event_table::FrameUpdated);
                        window.request_redraw();
                    }
                    EmulatorStarted => {
                        ui.force_render = true;
                        window.request_redraw();
                    }
                    EmulatorPaused => {
                        ui.notify(event_table::EmulatorUpdated);
                        ui.force_render = false;
                    }
                    BreakpointsUpdated => ui.notify(event_table::BreakpointsUpdated),

                    Debug(value) => {
                        ui.get::<AppState>().debug = value;
                        ui.notify(event_table::Debug(value));
                        emu_channel.send(EmulatorEvent::Debug(value)).unwrap();
                    }
                }
            }
            Event::MainEventsCleared => {}
            Event::RedrawRequested(window_id) => {
                // render the gui
                ui.render(window_id);

                if ui.is_animating {
                    *control = ControlFlow::Poll;
                }

                let joypad = ui.get::<AppState>().joypad;
                emu_channel.send(EmulatorEvent::SetJoypad(joypad)).unwrap();
                emu_channel.send(EmulatorEvent::RunFrame).unwrap();
            }
            _ => {}
        }
    });
}

#[derive(Debug)]
pub enum UserEvent {
    FrameUpdated,
    EmulatorPaused,
    EmulatorStarted,
    BreakpointsUpdated,
    Debug(bool),
}
