// Run through tools/test_program.sh. Only the NOR is simulated. tinfl is the
// reference C implementation linked into this host executable.
#[path = "../../targets/esp32p4/src/program.rs"]
mod program;

mod flash {
    use std::cell::RefCell;
    pub const SECTOR: u32 = 4096;
    pub struct Nor(pub RefCell<Vec<u8>>);
    impl Nor {
        pub fn bounds(&self, addr: u32, len: u32) -> Result<(), u8> {
            if u64::from(addr) + u64::from(len) > self.0.borrow().len() as u64 {
                Err(0xc0)
            } else {
                Ok(())
            }
        }
        pub fn erase(&self, addr: u32, len: u32) -> Result<(), u8> {
            assert_eq!((addr | len) % SECTOR, 0);
            self.bounds(addr, len)?;
            self.0.borrow_mut()[addr as usize..(addr + len) as usize].fill(0xff);
            Ok(())
        }
        pub fn program(&self, addr: u32, data: &[u8]) -> Result<(), u8> {
            self.bounds(addr, data.len() as u32)?;
            let mut memory = self.0.borrow_mut();
            for (dst, &src) in memory[addr as usize..].iter_mut().zip(data) {
                if *dst & src != src {
                    return Err(0xc4);
                }
                *dst = src;
            }
            Ok(())
        }
    }
}

fn words(data: &[u32]) -> Vec<u8> {
    data.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn main() {
    use program::Download;
    use stub_core::protocol::data_checksum;
    let args: Vec<_> = std::env::args().collect();
    let plain = std::fs::read(&args[1]).unwrap();
    let compressed = std::fs::read(&args[2]).unwrap();
    for (kind, input) in [(2, plain.as_slice()), (0x10, compressed.as_slice())] {
        let nor = flash::Nor(vec![0x55; 131072].into());
        let block = 257;
        let begin = words(&[
            plain.len() as u32,
            input.len().div_ceil(block) as u32,
            block as u32,
            4096,
        ]);
        let mut state = Download::begin(kind, &begin, &nor).unwrap();
        assert!(state.finish(kind + 2, &words(&[1])).is_err());
        for (seq, data) in input.chunks(block).enumerate() {
            let mut packet = words(&[data.len() as u32, seq as u32, 0, 0]);
            packet.extend_from_slice(data);
            let sum = u32::from(data_checksum(data));
            assert_eq!(state.validate_data(kind + 1, &packet, sum ^ 1), Err(0xc1));
            state.data(kind + 1, &packet, sum, &nor).unwrap();
            assert!(state.validate_data(kind + 1, &packet, sum).is_err());
        }
        assert_eq!(state.finish(kind + 2, &words(&[1])).unwrap(), None);
        let bytes = nor.0.borrow();
        assert_eq!(&bytes[4096..4096 + plain.len()], plain);
        assert!(bytes[..4096].iter().all(|&b| b == 0x55));
        let after = 4096 + plain.len().div_ceil(4096) * 4096;
        assert!(bytes[after..].iter().all(|&b| b == 0x55));
    }
}
