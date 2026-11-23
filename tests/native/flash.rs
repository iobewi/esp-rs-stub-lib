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
