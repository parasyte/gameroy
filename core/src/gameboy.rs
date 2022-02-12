use crate::cartridge::Cartridge;
use crate::save_state::{LoadStateError, SaveState};
use crate::sound_controller::SoundController;
use crate::{cpu::Cpu, disassembler::Trace, ppu::Ppu};
use std::cell::RefCell;
use std::io::{Read, Write};

#[derive(Default, PartialEq, Eq)]
pub struct Timer {
    pub div: u16,
    pub tima: u8,
    tma: u8,
    tac: u8,
    last_counter_bit: bool,

    /// The last clock cycle where Timer was updated
    last_clock_count: u64,
}
impl SaveState for Timer {
    fn save_state(&self, data: &mut impl Write) -> Result<(), std::io::Error> {
        self.div.save_state(data)?;
        self.tima.save_state(data)?;
        self.tma.save_state(data)?;
        self.tac.save_state(data)?;
        [&self.last_counter_bit].save_state(data)?;
        self.last_clock_count.save_state(data)?;
        Ok(())
    }

    fn load_state(&mut self, data: &mut impl Read) -> Result<(), LoadStateError> {
        self.div.load_state(data)?;
        self.tima.load_state(data)?;
        self.tma.load_state(data)?;
        self.tac.load_state(data)?;
        [&mut self.last_counter_bit].load_state(data)?;
        self.last_clock_count.load_state(data)?;
        Ok(())
    }
}
// TODO: At some point, I want this timer to be lazy evaluated.
impl Timer {
    /// Advance the timer by one cycle
    /// Return true if there is a interrupt
    pub fn update(&mut self, clock_count: u64) -> bool {
        let mut interrupt = false;
        for _ in self.last_clock_count..clock_count {
            self.div = self.div.wrapping_add(1);

            let f = [9, 3, 5, 7][(self.tac & 0b11) as usize];
            let counter_bit = ((self.div >> f) as u8 & (self.tac >> 2)) & 0b1 != 0;

            // faling edge
            if self.last_counter_bit && !counter_bit {
                let (v, o) = self.tima.overflowing_add(1);
                self.tima = v;
                // TODO: TIMA, on overflow, should keep the value 0 for 4 cycles
                // before the overflow be detected. A write in this interval would cancel it.
                if o {
                    self.tima = self.tma;
                    interrupt = true;
                }
            }

            self.last_counter_bit = counter_bit;
        }
        self.last_clock_count = clock_count;

        interrupt
    }

    fn read_div(&self) -> u8 {
        (self.div >> 8) as u8
    }
    fn write_div(&mut self, _div: u8) {
        self.div = 0;
    }

    fn read_tima(&self) -> u8 {
        self.tima
    }
    fn write_tima(&mut self, tima: u8) {
        self.tima = tima;
    }

    fn read_tma(&self) -> u8 {
        self.tma
    }
    fn write_tma(&mut self, tma: u8) {
        self.tma = tma;
    }

    fn read_tac(&self) -> u8 {
        self.tac
    }
    fn write_tac(&mut self, tac: u8) {
        self.tac = tac;
    }
}

pub struct GameBoy {
    pub trace: RefCell<Trace>,
    pub cpu: Cpu,
    pub cartridge: Cartridge,
    /// C000-DFFF: Work RAM
    pub wram: [u8; 0x2000],
    /// FF80-FFFE: Hight RAM
    pub hram: [u8; 0x7F],
    pub boot_rom: Option<[u8; 0x100]>,
    pub boot_rom_active: bool,
    pub clock_count: u64,
    pub timer: Timer,
    pub sound: RefCell<SoundController>,
    pub ppu: RefCell<Ppu>,
    /// JoyPad state. 0 bit means pressed.
    /// From bit 7 to 0, the order is: Start, Select, B, A, Down, Up, Left, Right
    /// FF00: P1
    pub joypad_io: u8,
    pub joypad: u8,
    /// FF01: SB
    pub serial_data: u8,
    /// FF02: SC
    pub serial_control: u8,
    pub serial_transfer: Box<dyn FnMut(u8) + Send>,
    /// FF0F: IF
    pub interrupt_flag: u8,
    /// FFFF: IE
    pub interrupt_enabled: u8,

    pub v_blank: Option<Box<dyn FnMut(&mut GameBoy) + Send>>,
}

impl std::fmt::Debug for GameBoy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: derive Debug for fields when the time arrive.
        f.debug_struct("GameBoy")
            // .field("trace", &self.trace)
            // .field("cpu", &self.cpu)
            // .field("cartridge", &self.cartridge)
            .field("wram", &self.wram)
            .field("hram", &self.hram)
            .field("boot_rom", &self.boot_rom)
            .field("boot_rom_active", &self.boot_rom_active)
            .field("clock_count", &self.clock_count)
            // .field("timer", &self.timer)
            // .field("sound", &self.sound)
            // .field("ppu", &self.ppu)
            .field("joypad", &self.joypad)
            .field("joypad_io", &self.joypad_io)
            // .field("serial_transfer", &self.serial_transfer)
            // .field("v_blank", &self.v_blank)
            .finish()
    }
}

impl Eq for GameBoy {}
impl PartialEq for GameBoy {
    fn eq(&self, other: &Self) -> bool {
        // self.trace == other.trace &&
        self.cpu == other.cpu
            && self.cartridge == other.cartridge
            && self.wram == other.wram
            && self.hram == other.hram
            && self.boot_rom == other.boot_rom
            && self.boot_rom_active == other.boot_rom_active
            && self.clock_count == other.clock_count
            && self.timer == other.timer
            && self.sound == other.sound
            && self.ppu == other.ppu
            && self.joypad == other.joypad
            && self.joypad_io == other.joypad_io
        // && self.serial_transfer == other.serial_transfer
        // && self.v_blank == other.v_blank
    }
}
impl SaveState for GameBoy {
    fn save_state(&self, data: &mut impl Write) -> Result<(), std::io::Error> {
        // self.trace.save_state(output)?;
        self.cpu.save_state(data)?;
        self.cartridge.save_state(data)?;
        self.wram.save_state(data)?;
        self.hram.save_state(data)?;
        // self.boot_rom.save_state(output)?;
        [&self.boot_rom_active].save_state(data)?;
        self.clock_count.save_state(data)?;
        self.timer.save_state(data)?;
        {
            let mut sound = self.sound.borrow_mut();
            sound.update(self.clock_count);
            sound.save_state(data)?;
        }
        self.ppu.save_state(data)?;
        self.joypad.save_state(data)?;
        self.joypad_io.save_state(data)?;
        // self.serial_transfer.save_state(data)?;
        // self.v_blank.save_state(data)
        Ok(())
    }

    fn load_state(&mut self, data: &mut impl Read) -> Result<(), LoadStateError> {
        // self.trace.load_state(output)?;
        self.cpu.load_state(data)?;
        self.cartridge.load_state(data)?;
        self.wram.load_state(data)?;
        self.hram.load_state(data)?;
        // self.boot_rom.load_state(output)?;
        [&mut self.boot_rom_active].load_state(data)?;
        self.clock_count.load_state(data)?;
        self.timer.load_state(data)?;
        {
            let mut sound = self.sound.borrow_mut();
            sound.load_state(data)?;
            if sound.last_clock != self.clock_count {
                return Err(LoadStateError::SoundControllerDesync(
                    sound.last_clock,
                    self.clock_count,
                ));
            }
        }
        self.ppu.load_state(data)?;
        self.joypad.load_state(data)?;
        self.joypad_io.load_state(data)?;
        // self.serial_transfer.load_state(data)?;
        // self.v_blank.load_state(data)
        Ok(())
    }
}
impl GameBoy {
    pub fn new(boot_rom: Option<[u8; 0x100]>, cartridge: Cartridge) -> Self {
        let mut this = Self {
            trace: RefCell::new(Trace::new()),
            cpu: Cpu::default(),
            cartridge,
            wram: [0; 0x2000],
            hram: [0; 0x7F],
            boot_rom,
            boot_rom_active: true,
            clock_count: 0,
            timer: Timer::default(),
            sound: RefCell::new(SoundController::default()),
            ppu: Ppu::default().into(),

            joypad: 0xFF,
            joypad_io: 0xFF,
            serial_transfer: Box::new(|c| {
                eprint!("{}", c as char);
            }),
            v_blank: None,
            serial_data: 0,
            serial_control: 0,
            interrupt_flag: 0,
            interrupt_enabled: 0,
        };

        if this.boot_rom.is_none() {
            this.reset_after_boot();
        }

        this
    }

    /// Reset the gameboy to its stating state.
    pub fn reset(&mut self) {
        if self.boot_rom.is_none() {
            self.reset_after_boot();
            return;
        }
        // TODO: Maybe I should reset the cartridge
        self.cpu = Cpu::default();
        self.wram = [0; 0x2000];
        self.hram = [0; 0x7F];
        self.boot_rom_active = true;
        self.clock_count = 0;
        self.timer = Timer::default();
        self.sound = RefCell::new(SoundController::default());
        self.ppu = Ppu::default().into();
        self.joypad = 0xFF;
        self.joypad_io = 0x00;
    }

    /// Reset the gameboy to its state after disabling the boot.
    pub fn reset_after_boot(&mut self) {
        self.cpu = Cpu {
            a: 0x01,
            f: crate::cpu::Flags(0xb0),
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,
            sp: 0xfffe,
            pc: 0x0100,
            ime: crate::cpu::ImeState::Disabled,
            state: crate::cpu::CpuState::Running,
        };

        self.wram = [0; 0x2000];
        self.hram = [0; 0x7F];
        self.hram[0x7a..=0x7c].copy_from_slice(&[0x39, 0x01, 0x2e]);

        self.boot_rom_active = false;
        self.clock_count = 23_384_580;
        self.timer = Timer {
            div: 0xd2,
            tima: 0x00,
            tma: 0x00,
            tac: 0x00,
            last_counter_bit: false,
            last_clock_count: self.clock_count,
        };
        self.sound
            .borrow_mut()
            .load_state(&mut &include_bytes!("../after_boot/sound.sav")[..])
            .unwrap();
        self.ppu
            .load_state(&mut &include_bytes!("../after_boot/ppu.sav")[..])
            .unwrap();
        self.joypad = 0xFF;
        self.joypad_io = 0x00;
    }

    pub fn read(&self, mut address: u16) -> u8 {
        if self.boot_rom_active {
            if address < 0x100 {
                let boot_rom = self
                    .boot_rom
                    .expect("the boot rom is only actived when there is one");
                return boot_rom[address as usize];
            }
        }
        if (0xE000..=0xFDFF).contains(&address) {
            address -= 0x2000;
        }
        match address {
            // Cartridge ROM
            0x0000..=0x7FFF => self.cartridge.read(address),
            // Video RAM
            0x8000..=0x9FFF => self.ppu.borrow().vram[address as usize - 0x8000],
            // Cartridge RAM
            0xA000..=0xBFFF => self.cartridge.read(address),
            // Work RAM
            0xC000..=0xDFFF => self.wram[address as usize - 0xC000],
            // ECHO RAM
            0xE000..=0xFDFF => unreachable!(),
            // Sprite Attribute table
            0xFE00..=0xFE9F => self.ppu.borrow().oam[address as usize - 0xFE00],
            // Not Usable
            0xFEA0..=0xFEFF => 0xff,
            // I/O registers
            0xFF00..=0xFF7F => self.read_io(address as u8),
            // Hight RAM
            0xFF80..=0xFFFE => self.hram[address as usize - 0xFF80],
            // IE Register
            0xFFFF => self.read_io(address as u8),
        }
    }

    pub fn write(&mut self, mut address: u16, value: u8) {
        if (0xE000..=0xFDFF).contains(&address) {
            address -= 0x2000;
        }

        match address {
            // Cartridge ROM
            0x0000..=0x7FFF => self.cartridge.write(address, value),
            // Video RAM
            0x8000..=0x9FFF => self.ppu.borrow_mut().vram[address as usize - 0x8000] = value,
            // Cartridge RAM
            0xA000..=0xBFFF => self.cartridge.write(address, value),
            // Work RAM
            0xC000..=0xDFFF => self.wram[address as usize - 0xC000] = value,
            // ECHO RAM
            0xE000..=0xFDFF => unreachable!(),
            // Sprite Attribute table
            0xFE00..=0xFE9F => self.ppu.borrow_mut().oam[address as usize - 0xFE00] = value,
            // Not Usable
            0xFEA0..=0xFEFF => {}
            // I/O registers
            0xFF00..=0xFF7F => self.write_io(address as u8, value),
            // Hight RAM
            0xFF80..=0xFFFE => self.hram[address as usize - 0xFF80] = value,
            // IE Register
            0xFFFF => self.write_io(address as u8, value),
        }
    }

    /// Advante the clock by 'count' cycles
    pub fn tick(&mut self, count: u8) {
        self.clock_count += count as u64;
        if self.timer.update(self.clock_count) {
            self.interrupt_flag |= 1 << 2;
        }
        let (v_blank_interrupt, stat_interrupt) = Ppu::update(self);
        if v_blank_interrupt {
            if let Some(mut v_blank) = self.v_blank.take() {
                v_blank(self);
                self.v_blank = Some(v_blank);
            }
            self.interrupt_flag |= 1 << 0;
        }
        if stat_interrupt {
            self.interrupt_flag |= 1 << 1;
        }
    }

    pub fn read16(&self, address: u16) -> u16 {
        u16::from_le_bytes([self.read(address), self.read(address.wrapping_add(1))])
    }

    pub fn write16(&mut self, address: u16, value: u16) {
        let [a, b] = value.to_le_bytes();
        self.write(address, a);
        self.write(address.wrapping_add(1), b);
    }

    fn write_io(&mut self, address: u8, value: u8) {
        match address {
            0x00 => self.joypad_io = 0b1100_1111 | (value & 0x30), // JOYPAD
            0x01 => self.serial_data = value,
            0x02 => {
                self.serial_control = value;
                if value == 0x81 {
                    (self.serial_transfer)(self.serial_data);
                }
            }
            0x03 => {}
            0x04 => self.timer.write_div(value),
            0x05 => self.timer.write_tima(value),
            0x06 => self.timer.write_tma(value),
            0x07 => self.timer.write_tac(value),
            0x08..=0x0e => {}
            0x0f => self.interrupt_flag = value,
            0x10..=0x14 | 0x16..=0x1e | 0x20..=0x26 | 0x30..=0x3f => {
                self.sound
                    .borrow_mut()
                    .write(self.clock_count, address, value)
            }
            0x15 => {}
            0x1f => {}
            0x27..=0x2f => {}
            0x40..=0x45 => Ppu::write(self, address, value),
            0x46 => {
                // DMA Transfer
                // TODO: this is not the proper behavior, of course
                let start = (value as u16) << 8;
                for (i, j) in (0xFE00..=0xFE9F).zip(start..start + 0x9F) {
                    let value = self.read(j);
                    self.write(i, value);
                }
            }
            0x47..=0x4b => Ppu::write(self, address, value),
            0x4c..=0x4f => {}
            0x50 => {
                if value & 0b1 != 0 {
                    self.boot_rom_active = false;
                    self.cpu.pc = 0x100;
                }
            }
            0x51..=0x7f => {}
            0x80..=0xfe => self.hram[address as usize - 0x80] = value,
            0xff => self.interrupt_enabled = value,
        }
    }

    fn read_io(&self, address: u8) -> u8 {
        match address {
            0x00 => {
                // JOYPAD
                let v = self.joypad_io;
                let mut r = (v & 0x30) | 0b1100_0000;
                if v & 0x10 != 0 {
                    r |= (self.joypad & 0xF0) >> 4;
                }
                if v & 0x20 != 0 {
                    r |= self.joypad & 0x0F;
                }
                r
            }
            0x01 => self.serial_data,
            0x02 => self.serial_control,
            0x03 => 0xff,
            0x04 => self.timer.read_div(),
            0x05 => self.timer.read_tima(),
            0x06 => self.timer.read_tma(),
            0x07 => self.timer.read_tac(),
            0x08..=0x0e => 0xff,
            0x0f => self.interrupt_flag,
            0x10..=0x14 | 0x16..=0x1e | 0x20..=0x26 | 0x30..=0x3f => {
                self.sound.borrow_mut().read(self.clock_count, address)
            }
            0x15 => 0xff,
            0x1f => 0xff,
            0x27..=0x2f => 0xff,
            0x40..=0x45 => Ppu::read(self, address),
            0x46 => 0xff,
            0x47..=0x4b => Ppu::read(self, address),
            0x4c => 0xff,
            0x4d => 0xff,
            0x4e..=0x4f => 0xff,
            0x50 => 0xff,
            0x51..=0x7F => 0xff,
            0x80..=0xfe => {
                // high RAM, IF flag and IE flag
                self.hram[address as usize - 0x80]
            }
            0xff => self.interrupt_enabled,
        }
    }
}
