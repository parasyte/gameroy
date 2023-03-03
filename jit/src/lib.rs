use dynasmrt::ExecutableBuffer;
use gameroy::{
    consts::{CB_CLOCK, CLOCK},
    disassembler::{Address, Cursor},
    gameboy::{cpu::CpuState, GameBoy},
    interpreter::Interpreter,
};
use std::collections::BTreeMap;

use self::x64::BlockCompiler;

#[cfg(target_os = "windows")]
mod windows;

mod x64;

pub struct Block {
    _start_address: u16,
    _length: u16,
    max_clock_cycles: u32,
    fn_ptr: unsafe extern "sysv64" fn(&mut GameBoy),
    _compiled_code: ExecutableBuffer,
}

impl Block {
    #[inline(never)]
    fn call(&self, gb: &mut GameBoy) {
        // SAFETY: As long as `Block`s are only generated from BlockCompiler::compile, and
        // Self::_compiled_code is not mutated, self.fn_ptr should be pointing to a valid x64
        // function.
        unsafe { (self.fn_ptr)(gb) }
    }
}

fn trace_a_block(gb: &GameBoy, start_address: u16) -> (u16, u16, u32) {
    let bank = gb.cartridge.curr_bank();

    let cursor = Cursor {
        bank: Some(bank),
        pc: start_address,
        reg_a: Some(gb.cpu.a),
    };

    let mut cursors = vec![cursor];

    let mut max_clock_cycles = 0;
    let mut length = 0;

    let only_one_bank = gb.cartridge.num_banks() == 2;

    while let Some(cursor) = cursors.pop() {
        let (op, len) = cursor.get_op(gb);
        length += len as u16;
        max_clock_cycles += CLOCK[op[0] as usize] as u32;
        if op[0] == 0xcb {
            max_clock_cycles += CB_CLOCK[op[1] as usize] as u32;
        }

        let (step, jump) = gameroy::disassembler::compute_step(len, cursor, &op, only_one_bank);

        if jump.is_some() || [0x76, 0xc0, 0xc8, 0xc9, 0xd0, 0xd8, 0xd9].contains(&op[0]) {
            break;
        }

        let Some(step) = step else { break };
        cursors.push(step);
    }

    // in case any of the instructions branches.
    max_clock_cycles += 12;

    (start_address, length, max_clock_cycles)
}

pub struct JitCompiler {
    blocks: BTreeMap<Address, Block>,
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl JitCompiler {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
        }
    }

    pub fn get_block(&mut self, gb: &GameBoy) -> Option<&Block> {
        let pc = gb.cpu.pc;
        let bank = gb.cartridge.curr_bank();

        let address = Address::from_pc(Some(bank), pc)?;
        Some(
            self.blocks
                .entry(address)
                .or_insert_with(|| BlockCompiler::new(gb).compile_block()),
        )
    }

    pub fn interpret_block(&mut self, gb: &mut GameBoy) {
        let block = self.get_block(gb);
        let next_interrupt = gb.next_interrupt();
        let start_clock = gb.clock_count;
        match block {
            Some(block)
                if gb.cpu.state == CpuState::Running
                    && (gb.clock_count + block.max_clock_cycles as u64 + 4) < next_interrupt =>
            {
                // println!("running {:04x} ({})", block._start_address, gb.clock_count);
                block.call(gb);
                debug_assert!(gb.clock_count - start_clock <= block.max_clock_cycles as u64);
                debug_assert!(gb.clock_count != start_clock);
            }
            _ => {
                // println!("interpr {:04x} ({})", gb.cpu.pc, gb.clock_count);
                let mut inter = Interpreter(gb);
                loop {
                    let have_jump = inter.will_jump_to().is_some();

                    inter.interpret_op();

                    if have_jump || inter.0.cpu.state != CpuState::Running {
                        break;
                    }
                }
            }
        }
    }
}
